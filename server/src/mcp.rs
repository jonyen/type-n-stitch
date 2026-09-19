//! The MCP endpoint: an agent joins a project as a visible peer.
//!
//! `/mcp` is rmcp's streamable-HTTP transport nested into the same router as
//! the browser API. A tower layer in front of it turns an
//! `Authorization: Bearer` token into the person who minted it plus the bot
//! user that acts on their behalf.
//!
//! What the agent is doing — the project it opened and the hub subscription
//! that makes it show up in everyone's peer list — does not live in the
//! [`McpSession`] rmcp hands the request, because on the current protocol
//! there is no session to live in: the 2026-07-28 lifecycle has none, so
//! rmcp builds a fresh handler for every single request. It lives in
//! [`Agents`] on the shared state instead, keyed by the bot behind the
//! token, and is retired when it has gone quiet for [`IDLE_TIMEOUT`].
//!
//! Every `#[tool]` here is a thin wrapper: it pulls the identity out of the
//! HTTP request parts rmcp forwards, calls the matching `tool_*` function —
//! which returns the server's own [`AppError`] on failure, so agents see the
//! same wording the browser does — and renders the JSON.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use engine::types::{CaptionPos, Range, TitleStyle, Transition, Word};
use engine::{Edit, Op, EPS};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData as McpError, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::{ensure_bot, User};
use crate::bus::{PeerInfo, PresenceState, ServerMsg, Subscription};
use crate::error::{AppError, AppResult};
use crate::mcp_tools::{find_ranges, transcript, word_range};
use crate::ops::{ClientOp, DocState};
use crate::projects::{ensure_bot_member, find_project, member_role, Project, Role};
use crate::routes::ExportJob;
use crate::{ops, projects, routes, tokens, AppState};

/// How long an editing tool leaves the agent's cursor on the range it is
/// about to change, so people watching see where the edit came from. Tests
/// do not wait.
#[cfg(not(test))]
pub const DWELL: Duration = Duration::from_millis(400);
#[cfg(test)]
pub const DWELL: Duration = Duration::ZERO;

/// How often `export` asks whether ffmpeg has finished, and how long it
/// keeps asking before handing the job id back so a later call can resume.
/// Tests poll fast and give up early: their render fails immediately.
#[cfg(not(test))]
const EXPORT_POLL: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const EXPORT_CAP: Duration = Duration::from_secs(600);
#[cfg(test)]
const EXPORT_POLL: Duration = Duration::from_millis(10);
#[cfg(test)]
const EXPORT_CAP: Duration = Duration::from_secs(2);

/// How long a title card stays on screen when the agent does not say.
const TITLE_SECONDS: f64 = 3.0;

/// An agent that has called nothing for this long is retired, which releases
/// its subscription and clears its cursor from everyone's screen.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the reaper looks for agents that have gone quiet.
const REAP_EVERY: Duration = Duration::from_secs(30);

/// Who an MCP request acts for: the person who minted the token, and the bot
/// user that does the editing on their behalf.
#[derive(Clone)]
pub struct McpIdentity {
    pub owner: User,
    pub bot: User,
}

/// Bearer authentication in front of the MCP transport. rmcp never sees an
/// unauthenticated request, and the resolved identity rides in the request
/// extensions, which rmcp forwards to tool calls as `http::request::Parts`.
pub async fn bearer_layer(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    let Some(token) = token else {
        return AppError::unauthorized().into_response();
    };
    let owner = match tokens::verify(&state.db, &token).await {
        Ok(Some(owner)) => owner,
        Ok(None) => return AppError::unauthorized().into_response(),
        Err(e) => return e.into_response(),
    };
    let bot = match ensure_bot(&state.db, &owner).await {
        Ok(bot) => bot,
        Err(e) => return e.into_response(),
    };
    req.extensions_mut().insert(McpIdentity { owner, bot });
    next.run(req).await
}

/// What an open project gives the tools. `subscription` is held only to
/// exist as a peer: the agent never reads its own frames, and dropping it is
/// what tells everyone else the agent left.
#[derive(Default)]
pub struct Open {
    project: Option<Project>,
    role: Option<Role>,
    bot: Option<User>,
    conn_id: String,
    subscription: Option<Subscription>,
    /// The media's source duration, for the range the last word owns.
    duration: f64,
    /// The transcript, fetched once at open: words never change, only edits.
    words: Vec<Word>,
    /// Speaker index per word, when diarization succeeded.
    speakers: Option<Vec<Option<u32>>>,
}

/// The live agents, one per token owner's bot.
///
/// Every request on the 2026-07-28 lifecycle gets its own [`McpSession`], so
/// an agent cannot be a session: what it has open is kept here, on the shared
/// state, and found again by the bot id behind the Bearer token. All of one
/// person's Claude Code instances therefore drive the same agent — the same
/// open project, the same cursor, the one peer in the project. An entry is
/// created by the first tool call that needs it, and removed once it has been
/// idle for [`IDLE_TIMEOUT`], which drops its subscription and announces
/// `left`.
#[derive(Default)]
pub struct Agents {
    live: std::sync::Mutex<std::collections::HashMap<String, Agent>>,
}

/// One live agent: what it has open, and when it last did anything. The two
/// are separate locks because the reaper must read the clock without waiting
/// behind a tool call that is holding the open project for a slow render.
struct Agent {
    open: Arc<Mutex<Open>>,
    /// Tokio's clock rather than the standard one, so a test can wind it on
    /// instead of waiting out the timeout.
    last_used: std::sync::Mutex<tokio::time::Instant>,
}

impl Agents {
    /// This bot's agent, created empty if this is its first call, with its
    /// idle clock reset.
    fn touch(&self, bot_id: &str) -> Arc<Mutex<Open>> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let agent = live.entry(bot_id.to_owned()).or_insert_with(|| Agent {
            open: Arc::new(Mutex::new(Open {
                // The agent's identity in the peer list, for as long as it
                // lives: one conn_id across all its calls and projects.
                conn_id: Uuid::new_v4().to_string(),
                ..Open::default()
            })),
            last_used: std::sync::Mutex::new(tokio::time::Instant::now()),
        });
        *agent.last_used.lock().unwrap_or_else(|e| e.into_inner()) = tokio::time::Instant::now();
        agent.open.clone()
    }

    /// Retire every agent that has called nothing for `idle`, and say how
    /// many went. Dropping the entry drops the subscription with it, so the
    /// people watching the project see the agent leave.
    pub fn evict_idle(&self, idle: Duration) -> usize {
        let now = tokio::time::Instant::now();
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let before = live.len();
        live.retain(|_, agent| {
            let last = *agent.last_used.lock().unwrap_or_else(|e| e.into_inner());
            now.duration_since(last) < idle
        });
        before - live.len()
    }

    /// How many agents are live, for tests.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Retire idle agents for as long as the server runs.
pub async fn reap_idle_agents(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(REAP_EVERY).await;
        let gone = state.agents.evict_idle(IDLE_TIMEOUT);
        if gone > 0 {
            tracing::info!(gone, "retired idle MCP agents");
        }
    }
}

/// One MCP request's handler. It holds nothing of its own beyond the router:
/// the agent it acts for is looked up in [`AppState::agents`] per call, by
/// the identity the Bearer token resolved to.
pub struct McpSession {
    state: Arc<AppState>,
    /// Built per handler and dispatched through by `#[tool_handler]`.
    tool_router: ToolRouter<McpSession>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenProjectArgs {
    /// The project's id, as `list_projects` reports it.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindArgs {
    /// Words to look for; matching ignores case and punctuation.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookAtArgs {
    /// First word index, from the transcript's `i`.
    pub from: usize,
    /// Last word index, inclusive.
    pub to: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CutArgs {
    /// First word to delete, from the transcript's `i`.
    pub from: usize,
    /// Last word to delete, inclusive.
    pub to: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OverdubArgs {
    /// First word to replace, from the transcript's `i`.
    pub from: usize,
    /// Last word to replace, inclusive.
    pub to: usize,
    /// What the speaker should say instead.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTitleArgs {
    /// The title lands just after this word, from the transcript's `i`.
    /// Use `-1` for a title before the first word.
    pub after: i64,
    /// The headline.
    pub text: String,
    /// A smaller second line, if you want one.
    pub subtitle: Option<String>,
    /// `dark`, `light` or `accent`; `dark` when omitted.
    pub style: Option<String>,
    /// Seconds the card stays on screen; 3 when omitted.
    pub duration: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddCaptionArgs {
    /// First word the caption covers, from the transcript's `i`.
    pub from: usize,
    /// Last word it covers, inclusive.
    pub to: usize,
    /// The caption text.
    pub text: String,
    /// `bottomLeft`, `bottomCenter` or `topLeft`; `bottomLeft` when omitted.
    pub position: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetTransitionArgs {
    /// `none` for a hard cut, or `dip` to dip through black at every join.
    pub kind: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportArgs {
    /// `mp4`, `mp3` or `wav`; the source's own kind when omitted.
    pub format: Option<String>,
}

#[tool_router]
impl McpSession {
    #[tool(description = "Every project you can open, with your role and its duration.")]
    async fn list_projects(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_list_projects(&identity_of(&ctx)?).await)
    }

    #[tool(
        description = "Open a project and join it as a visible peer. Returns its \
                       duration, edit counts and the whole transcript; every other \
                       tool works on the project opened here."
    )]
    async fn open_project(
        &self,
        Parameters(args): Parameters<OpenProjectArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_open_project(&identity_of(&ctx)?, &args.project_id)
                .await,
        )
    }

    #[tool(
        description = "The open project's transcript, with each word's status \
                          under the current edits."
    )]
    async fn get_transcript(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_get_transcript(&identity_of(&ctx)?).await)
    }

    #[tool(
        description = "Whole-word search over the transcript, ignoring case and \
                          punctuation. Returns inclusive word-index ranges."
    )]
    async fn find(
        &self,
        Parameters(args): Parameters<FindArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_find(&identity_of(&ctx)?, &args.text).await)
    }

    #[tool(
        description = "Move your cursor to a range of words so collaborators can \
                          see where you are looking. Changes nothing."
    )]
    async fn look_at(
        &self,
        Parameters(args): Parameters<LookAtArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_look_at(&identity_of(&ctx)?, args.from, args.to)
                .await,
        )
    }

    #[tool(
        description = "Delete words `from`..`to` (inclusive transcript indices) \
                       from the video, exactly as pressing Delete on that \
                       selection would. Moves your cursor there first, so it \
                       takes about half a second. Returns the new edit counts \
                       and the words it touched."
    )]
    async fn cut(
        &self,
        Parameters(args): Parameters<CutArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_cut(&identity_of(&ctx)?, args.from, args.to).await)
    }

    #[tool(
        description = "Cut every filler word (\"um\", \"you know\") the engine \
                       finds, in one operation. Returns `applied: 0` with a \
                       message when there is nothing left to remove."
    )]
    async fn remove_fillers(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_remove_fillers(&identity_of(&ctx)?).await)
    }

    #[tool(description = "Shorten every long silence the engine finds, in one \
                       operation. Returns `applied: 0` with a message when \
                       there is nothing left to tighten.")]
    async fn tighten_pauses(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_tighten_pauses(&identity_of(&ctx)?).await)
    }

    #[tool(description = "Replace what is said over words `from`..`to` with \
                       synthesized speech saying `text`. Needs the voice \
                       service; the picture is unchanged.")]
    async fn overdub(
        &self,
        Parameters(args): Parameters<OverdubArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_overdub(&identity_of(&ctx)?, args.from, args.to, &args.text)
                .await,
        )
    }

    #[tool(
        description = "Insert a full-screen title card just after word `after` \
                       (a transcript index; `-1` puts it before the first word). \
                       `style` is dark, light or accent; `duration` is seconds, \
                       3 by default. The output gets longer by `duration`."
    )]
    async fn add_title(
        &self,
        Parameters(args): Parameters<AddTitleArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_add_title(
                &identity_of(&ctx)?,
                args.after,
                &args.text,
                args.subtitle.as_deref(),
                args.style.as_deref(),
                args.duration,
            )
            .await,
        )
    }

    #[tool(
        description = "Draw text over the picture while words `from`..`to` play. \
                       `position` is bottomLeft, bottomCenter or topLeft; \
                       bottomLeft by default. The output length is unchanged."
    )]
    async fn add_caption(
        &self,
        Parameters(args): Parameters<AddCaptionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_add_caption(
                &identity_of(&ctx)?,
                args.from,
                args.to,
                &args.text,
                args.position.as_deref(),
            )
            .await,
        )
    }

    #[tool(
        description = "How the pieces either side of every cut meet: `none` for \
                       a hard cut, `dip` to dip through black."
    )]
    async fn set_transition(
        &self,
        Parameters(args): Parameters<SetTransitionArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_set_transition(&identity_of(&ctx)?, &args.kind)
                .await,
        )
    }

    #[tool(
        description = "Undo your own most recent edit. Other people's edits are \
                       theirs to undo; this fails with \"nothing to undo\" when \
                       you have none left."
    )]
    async fn undo(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        rendered(self.tool_undo(&identity_of(&ctx)?).await)
    }

    #[tool(description = "Redo the edit you last undid.")]
    async fn redo(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        rendered(self.tool_redo(&identity_of(&ctx)?).await)
    }

    #[tool(
        description = "Render the edited project and wait for it. `format` is \
                       mp4, mp3 or wav; the source's own kind by default. \
                       Returns `{ url, duration, bytes }`, or the job id with \
                       `pending: true` if the render is still going after ten \
                       minutes."
    )]
    async fn export(
        &self,
        Parameters(args): Parameters<ExportArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(
            self.tool_export(&identity_of(&ctx)?, args.format.as_deref())
                .await,
        )
    }
}

impl McpSession {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    /// The agent this identity acts as, with its idle clock reset. The
    /// returned handle keeps the agent alive for the length of the call even
    /// if the reaper retires it meanwhile, so a slow tool never loses the
    /// project out from under itself.
    fn agent(&self, identity: &McpIdentity) -> Arc<Mutex<Open>> {
        self.state.agents.touch(&identity.bot.id)
    }

    pub(crate) async fn tool_list_projects(&self, identity: &McpIdentity) -> AppResult<Value> {
        let summaries = projects::summaries(&self.state, &identity.owner.id).await?;
        Ok(Value::Array(
            summaries
                .into_iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "title": s.title,
                        "role": s.role,
                        "duration": s.media.duration,
                    })
                })
                .collect(),
        ))
    }

    pub(crate) async fn tool_open_project(
        &self,
        identity: &McpIdentity,
        id: &str,
    ) -> AppResult<Value> {
        // The same three rejections, in the same words, as `ProjectAccess`.
        let project = find_project(&self.state.db, id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("no project with id {id}")))?;
        let owner_role = member_role(&self.state.db, &project.id, &identity.owner.id)
            .await?
            .ok_or_else(|| AppError::forbidden("you are not a member of this project"))?;
        let role =
            ensure_bot_member(&self.state.db, &project.id, &identity.bot.id, owner_role).await?;

        let meta = routes::read_meta(&self.state.config.data_dir.join(&project.media_id)).await?;
        let (words, speakers) = routes::transcript_for(&self.state, &project.media_id).await?;
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;

        let agent = self.agent(identity);
        let mut open = agent.lock().await;
        // Leave whatever was open before, so the agent is never two peers.
        open.subscription = None;
        let subscription =
            self.state
                .bus
                .subscribe(&project.id, &open.conn_id, PeerInfo::from(&identity.bot));
        open.subscription = Some(subscription);
        open.project = Some(project.clone());
        open.role = Some(role);
        open.bot = Some(identity.bot.clone());
        open.duration = meta.duration;
        open.words = words;
        open.speakers = speakers.map(|s| s.words);

        Ok(json!({
            "id": project.id,
            "title": project.title,
            "duration": meta.duration,
            "outputDuration": engine::output_duration(&engine::timeline(meta.duration, &doc.edits)),
            "cuts": doc.edits.iter().filter(|e| matches!(e, engine::Edit::Cut { .. })).count(),
            "overdubs": doc.edits.iter().filter(|e| matches!(e, engine::Edit::Overdub { .. })).count(),
            "speakerNames": doc.speaker_names,
            "transition": doc.transition,
            "role": role,
            "transcript": transcript(&open.words, open.speakers.as_deref(), &doc.edits),
        }))
    }

    pub(crate) async fn tool_get_transcript(&self, identity: &McpIdentity) -> AppResult<Value> {
        let agent = self.agent(identity);
        let open = agent.lock().await;
        let project = require_open(&open, identity)?;
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        Ok(serde_json::to_value(transcript(
            &open.words,
            open.speakers.as_deref(),
            &doc.edits,
        ))?)
    }

    pub(crate) async fn tool_find(&self, identity: &McpIdentity, text: &str) -> AppResult<Value> {
        let agent = self.agent(identity);
        let open = agent.lock().await;
        require_open(&open, identity)?;
        Ok(Value::Array(
            find_ranges(&open.words, text)
                .into_iter()
                .map(|(from, to)| json!({ "from": from, "to": to }))
                .collect(),
        ))
    }

    pub(crate) async fn tool_look_at(
        &self,
        identity: &McpIdentity,
        from: usize,
        to: usize,
    ) -> AppResult<Value> {
        let agent = self.agent(identity);
        let open = agent.lock().await;
        let project = require_open(&open, identity)?;
        let words = words_in(&open, from, to)?;
        self.move_cursor(&open, project, from, to);
        Ok(json!({ "from": from, "to": to, "text": words }))
    }

    pub(crate) async fn tool_cut(
        &self,
        identity: &McpIdentity,
        from: usize,
        to: usize,
    ) -> AppResult<Value> {
        self.edit(identity, |open| {
            let range = range_of(open, from, to)?;
            Ok((
                Some((from, to)),
                Op::Cut {
                    start: range.start,
                    end: range.end,
                },
            ))
        })
        .await
    }

    pub(crate) async fn tool_remove_fillers(&self, identity: &McpIdentity) -> AppResult<Value> {
        self.apply_suggestions(identity, Suggestion::Fillers).await
    }

    pub(crate) async fn tool_tighten_pauses(&self, identity: &McpIdentity) -> AppResult<Value> {
        self.apply_suggestions(identity, Suggestion::Pauses).await
    }

    pub(crate) async fn tool_overdub(
        &self,
        identity: &McpIdentity,
        from: usize,
        to: usize,
        text: &str,
    ) -> AppResult<Value> {
        // The handle is held for the whole call, so even a slow one outlives
        // the reaper rather than losing its project halfway through.
        let agent = self.agent(identity);
        let project = {
            let open = agent.lock().await;
            let project = require_open(&open, identity)?.clone();
            require_edit(&open)?;
            // Checked here too, so nothing is synthesized for a range that
            // cannot land; `edit` checks it again against the live guard.
            range_of(&open, from, to)?;
            project
        };
        // Synthesis is the slow part and can fail upstream; do it before the
        // cursor moves, so a failure never leaves a phantom selection.
        let audio = routes::overdub_for(&self.state, &project.media_id, text).await?;
        self.edit(identity, |open| {
            let range = range_of(open, from, to)?;
            Ok((
                Some((from, to)),
                Op::Overdub {
                    start: range.start,
                    end: range.end,
                    text: text.to_owned(),
                    audio_url: audio.audio_url,
                    audio_duration: audio.duration,
                },
            ))
        })
        .await
    }

    pub(crate) async fn tool_add_title(
        &self,
        identity: &McpIdentity,
        after: i64,
        text: &str,
        subtitle: Option<&str>,
        style: Option<&str>,
        duration: Option<f64>,
    ) -> AppResult<Value> {
        self.edit(identity, |open| {
            // `-1` is "before the first word", which is the media's start;
            // anything further back is a mistake, not a shorthand.
            let (at, selection) = match after {
                -1 => (0.0, None),
                i if i < -1 => {
                    return Err(AppError::bad_request(format!(
                        "no word {after}: use -1 for a title before the first word"
                    )))
                }
                _ => {
                    let i = after as usize;
                    let word = open.words.get(i).ok_or_else(|| {
                        AppError::bad_request(format!(
                            "no word {i} in a transcript of {}",
                            open.words.len()
                        ))
                    })?;
                    (word.end, Some((i, i)))
                }
            };
            Ok((
                selection,
                Op::AddTitle {
                    at,
                    duration: duration.unwrap_or(TITLE_SECONDS),
                    text: text.to_owned(),
                    subtitle: subtitle.map(str::to_owned),
                    style: title_style(style)?,
                },
            ))
        })
        .await
    }

    pub(crate) async fn tool_add_caption(
        &self,
        identity: &McpIdentity,
        from: usize,
        to: usize,
        text: &str,
        position: Option<&str>,
    ) -> AppResult<Value> {
        self.edit(identity, |open| {
            let range = range_of(open, from, to)?;
            Ok((
                Some((from, to)),
                Op::AddCaption {
                    start: range.start,
                    end: range.end,
                    text: text.to_owned(),
                    position: caption_position(position)?,
                },
            ))
        })
        .await
    }

    pub(crate) async fn tool_set_transition(
        &self,
        identity: &McpIdentity,
        kind: &str,
    ) -> AppResult<Value> {
        // Parsed inside the plan, so an unopened session is told to open a
        // project before it is told how to spell the transition.
        let mut out = self
            .edit(identity, |_| {
                let transition = transition_kind(kind)?;
                Ok((None, Op::SetTransition { transition }))
            })
            .await?;
        out["transition"] = serde_json::to_value(transition_kind(kind)?)?;
        Ok(out)
    }

    pub(crate) async fn tool_undo(&self, identity: &McpIdentity) -> AppResult<Value> {
        self.step(identity, false).await
    }

    pub(crate) async fn tool_redo(&self, identity: &McpIdentity) -> AppResult<Value> {
        self.step(identity, true).await
    }

    pub(crate) async fn tool_export(
        &self,
        identity: &McpIdentity,
        format: Option<&str>,
    ) -> AppResult<Value> {
        // The guard is released before the poll loop: a ten-minute render
        // must not hold the session shut.
        // The handle is held for the whole call, so even a slow one outlives
        // the reaper rather than losing its project halfway through.
        let agent = self.agent(identity);
        let project = {
            let open = agent.lock().await;
            let project = require_open(&open, identity)?.clone();
            require_edit(&open)?;
            project
        };
        let started = routes::start_export(&self.state, &project, format).await?;
        let deadline = Instant::now() + EXPORT_CAP;
        loop {
            match routes::export_job(&self.state, &project.media_id, &started.job_id) {
                Some(ExportJob::Done {
                    url,
                    duration,
                    bytes,
                    ..
                }) => {
                    return Ok(json!({ "url": url, "duration": duration, "bytes": bytes }));
                }
                // The job's own words, the same text the browser would show.
                Some(ExportJob::Error { message, .. }) => return Err(AppError::upstream(message)),
                _ => {}
            }
            if Instant::now() >= deadline {
                // Still rendering: hand back the id rather than hold the call
                // open forever. `export` again later to pick it up.
                return Ok(json!({ "jobId": started.job_id, "pending": true }));
            }
            tokio::time::sleep(EXPORT_POLL).await;
        }
    }

    /// The shared body of every editing tool: work out what to do from the
    /// open project, move the cursor onto the words about to change, dwell
    /// long enough for people watching to see it, then append the operation
    /// as the bot and report the new document.
    ///
    /// `plan` runs under the same guard that the cursor and the report use,
    /// so the word indices it returns cannot go stale: a concurrent
    /// `open_project` either swaps the transcript before `plan` validates
    /// against it, or waits until this call is finished.
    async fn edit<F>(&self, identity: &McpIdentity, plan: F) -> AppResult<Value>
    where
        F: FnOnce(&Open) -> AppResult<(Option<(usize, usize)>, Op)>,
    {
        let agent = self.agent(identity);
        let open = agent.lock().await;
        let project = require_open(&open, identity)?.clone();
        require_edit(&open)?;
        let (selection, op) = plan(&open)?;
        if let Some((from, to)) = selection {
            self.move_cursor(&open, &project, from, to);
        }
        tokio::time::sleep(DWELL).await;
        let doc = ops::apply_ops(
            &self.state,
            &project,
            &identity.bot,
            vec![ClientOp {
                op_id: Uuid::new_v4().to_string(),
                op,
            }],
        )
        .await?;
        Ok(report(&open, &doc, selection))
    }

    /// `undo` and `redo` share everything but which target they look up.
    async fn step(&self, identity: &McpIdentity, redo: bool) -> AppResult<Value> {
        // The handle is held for the whole call, so even a slow one outlives
        // the reaper rather than losing its project halfway through.
        let agent = self.agent(identity);
        let project = {
            let open = agent.lock().await;
            let project = require_open(&open, identity)?.clone();
            require_edit(&open)?;
            project
        };
        let doc = ops::doc_state(&self.state, &project, &identity.bot).await?;
        let target = if redo { doc.redoable } else { doc.undoable };
        let target = target.ok_or_else(|| {
            AppError::bad_request(if redo {
                "nothing to redo"
            } else {
                "nothing to undo"
            })
        })?;
        let op = if redo {
            Op::Redo { target_seq: target }
        } else {
            Op::Undo { target_seq: target }
        };
        self.edit(identity, |_| Ok((None, op))).await
    }

    /// `remove_fillers` and `tighten_pauses`: the engine's suggestions, minus
    /// the ones an existing cut already covers, applied as one operation.
    async fn apply_suggestions(
        &self,
        identity: &McpIdentity,
        which: Suggestion,
    ) -> AppResult<Value> {
        // The handle is held for the whole call, so even a slow one outlives
        // the reaper rather than losing its project halfway through.
        let agent = self.agent(identity);
        let project = {
            let open = agent.lock().await;
            let project = require_open(&open, identity)?.clone();
            require_edit(&open)?;
            project
        };
        let suggestions = routes::suggest_for(&self.state, &project.media_id, false).await?;
        let suggested = match which {
            Suggestion::Fillers => suggestions.fillers,
            Suggestion::Pauses => suggestions.pauses,
        };
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        let cuts: Vec<Range> = suggested
            .iter()
            .map(Edit::range)
            .filter(|s| !already_cut(*s, &doc.edits))
            .collect();
        if cuts.is_empty() {
            // Nothing to do, so nothing moves: the cursor stays where it was.
            return Ok(json!({ "applied": 0, "message": which.nothing_found() }));
        }
        let applied = cuts.len();
        let mut out = self
            .edit(identity, |open| {
                Ok((touched_by(open, &cuts), Op::ApplyCuts { cuts }))
            })
            .await?;
        out["applied"] = json!(applied);
        Ok(out)
    }

    /// Put the agent's selection on `from..=to` and tell everyone watching.
    fn move_cursor(&self, open: &Open, project: &Project, from: usize, to: usize) {
        let state = PresenceState {
            playhead: open.words.get(from).map_or(0.0, |w| w.start),
            selection: Some([from, to]),
            caret: None,
            playing: false,
        };
        if let Some(peer) = self
            .state
            .bus
            .update_presence(&project.id, &open.conn_id, state)
        {
            self.state
                .bus
                .publish(&project.id, ServerMsg::Presence(peer));
        }
    }
}

/// Which half of the engine's suggestions a tool applies.
#[derive(Clone, Copy)]
enum Suggestion {
    Fillers,
    Pauses,
}

impl Suggestion {
    fn nothing_found(self) -> &'static str {
        match self {
            Suggestion::Fillers => "no fillers found",
            Suggestion::Pauses => "no long pauses found",
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpSession {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Edit video by editing its transcript. Call `list_projects`, then \
             `open_project` — every other tool works on the project you opened, \
             and until you do they all fail. `get_transcript` returns the words \
             with an index `i`; every tool that takes `from` and `to` means those \
             indices, inclusive. `find` locates words to work on and `look_at` \
             moves your cursor, which collaborators watching the project can see. \
             You appear to them as \"Claude\", a peer with its own colour, and your \
             edits are yours to undo. The editing tools are `cut`, \
             `remove_fillers`, `tighten_pauses`, `overdub`, `add_title`, \
             `add_caption`, `set_transition`, `undo` and `redo`. Each pauses \
             about half a second before it applies, and the ones that work on a \
             range of words move your cursor onto those words first — `undo`, \
             `redo`, `set_transition` and `add_title` with `after: -1` have no \
             range to point at, so they leave it where it is. All of them return \
             the new counts and the words they touched. `export` renders and waits. Indices are always the \
             transcript's `i`, so call `get_transcript` again after an edit \
             rather than reusing stale ones.",
        )
    }
}

/// A project is open, and it is this agent's.
fn require_open<'a>(open: &'a Open, identity: &McpIdentity) -> AppResult<&'a Project> {
    let project = open
        .project
        .as_ref()
        .ok_or_else(|| AppError::bad_request("open a project first"))?;
    match &open.bot {
        Some(bot) if bot.id == identity.bot.id => Ok(project),
        // Belt and braces: agents are already keyed by bot, so nobody can
        // reach someone else's entry, and nobody inherits their project.
        _ => Err(AppError::bad_request("open a project first")),
    }
}

/// This agent's bot may edit the open project.
fn require_edit(open: &Open) -> AppResult<()> {
    match open.role {
        // The same words `ProjectAccess::require_edit` gives the browser.
        Some(role) if role.can_edit() => Ok(()),
        _ => Err(AppError::forbidden(
            "you can view this project but not edit it",
        )),
    }
}

/// The source range words `from..=to` own, or the server's own 400.
fn range_of(open: &Open, from: usize, to: usize) -> AppResult<Range> {
    word_range(&open.words, from, to, open.duration).ok_or_else(|| {
        AppError::bad_request(format!(
            "no words {from}..{to} in a transcript of {}",
            open.words.len()
        ))
    })
}

/// A suggested cut an existing cut already swallows, which is the same rule
/// the client's `pending` uses so counts fall to zero once applied.
fn already_cut(suggested: Range, edits: &[Edit]) -> bool {
    edits.iter().filter(|e| e.is_cut()).any(|e| {
        let cut = e.range();
        cut.start <= suggested.start + EPS && cut.end >= suggested.end - EPS
    })
}

/// The span of word indices `cuts` touches, for the cursor to cover.
///
/// A word is measured by the stretch it owns rather than by the time it is
/// spoken: from the end of the word before it to the start of the word after
/// it (or to the ends of the media). Pause cuts lie entirely *between* words,
/// so on spoken time alone they would touch nothing at all and the cursor
/// would never move; on owned stretches the words either side of a gap both
/// count, which is what someone watching expects to see highlighted.
fn touched_by(open: &Open, cuts: &[Range]) -> Option<(usize, usize)> {
    let mut span: Option<(usize, usize)> = None;
    for i in 0..open.words.len() {
        let from = if i == 0 { 0.0 } else { open.words[i - 1].end };
        let to = open.words.get(i + 1).map_or(open.duration, |w| w.start);
        if cuts.iter().any(|c| from < c.end && to > c.start) {
            span = Some(match span {
                Some((lo, _)) => (lo, i),
                None => (i, i),
            });
        }
    }
    span
}

fn title_style(name: Option<&str>) -> AppResult<TitleStyle> {
    match name.unwrap_or("dark") {
        "dark" => Ok(TitleStyle::Dark),
        "light" => Ok(TitleStyle::Light),
        "accent" => Ok(TitleStyle::Accent),
        other => Err(AppError::bad_request(format!(
            "unknown title style {other}: use dark, light or accent"
        ))),
    }
}

fn caption_position(name: Option<&str>) -> AppResult<CaptionPos> {
    match name.unwrap_or("bottomLeft") {
        "bottomLeft" => Ok(CaptionPos::BottomLeft),
        "bottomCenter" => Ok(CaptionPos::BottomCenter),
        "topLeft" => Ok(CaptionPos::TopLeft),
        other => Err(AppError::bad_request(format!(
            "unknown caption position {other}: use bottomLeft, bottomCenter or topLeft"
        ))),
    }
}

fn transition_kind(name: &str) -> AppResult<Transition> {
    match name {
        "none" => Ok(Transition::None),
        "dip" => Ok(Transition::Dip),
        other => Err(AppError::bad_request(format!(
            "unknown transition {other}: use none or dip"
        ))),
    }
}

/// What every editing tool reports: the document's headline numbers after
/// the edit, plus the words the edit landed on with their new status.
fn report(open: &Open, doc: &DocState, touched: Option<(usize, usize)>) -> Value {
    // `touched` comes from a plan validated under the same guard, so the
    // range always fits; `get` rather than a slice so that a future caller
    // that forgets gets an empty list instead of a panic.
    let words = match touched {
        Some((from, to)) => transcript(&open.words, open.speakers.as_deref(), &doc.edits)
            .get(from..=to)
            .map(<[_]>::to_vec)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    json!({
        "headSeq": doc.head_seq,
        "outputDuration": engine::output_duration(&engine::timeline(open.duration, &doc.edits)),
        "cuts": doc.edits.iter().filter(|e| matches!(e, Edit::Cut { .. })).count(),
        "overdubs": doc.edits.iter().filter(|e| matches!(e, Edit::Overdub { .. })).count(),
        "undoable": doc.undoable.is_some(),
        "redoable": doc.redoable.is_some(),
        "touched": words,
    })
}

/// The words `from..=to`, rejecting a range the transcript does not have.
fn words_in(open: &Open, from: usize, to: usize) -> AppResult<String> {
    if from > to || to >= open.words.len() {
        return Err(AppError::bad_request(format!(
            "no words {from}..{to} in a transcript of {}",
            open.words.len()
        )));
    }
    Ok(open.words[from..=to]
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" "))
}

/// The identity the auth layer stashed on the HTTP request rmcp forwarded.
fn identity_of(ctx: &RequestContext<RoleServer>) -> Result<McpIdentity, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<McpIdentity>())
        .cloned()
        .ok_or_else(|| McpError::invalid_request("unauthenticated", None))
}

/// A tool's JSON, or the server's own error text.
fn rendered(result: AppResult<Value>) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
        }
        Err(err) if err.status().is_server_error() => {
            Err(McpError::internal_error(err.to_string(), None))
        }
        Err(err) => Err(McpError::invalid_params(err.to_string(), None)),
    }
}

/// The MCP transport.
///
/// rmcp keeps a session only for a client that opens with `initialize`; on
/// the 2026-07-28 lifecycle, which is what Claude Code negotiates, SEP-2567
/// removes sessions altogether and every request gets a fresh handler. So
/// the handler is stateless on purpose: what the agent has open lives in
/// [`Agents`], keyed by the bot behind the token. One live agent per token
/// owner, shared by all their Claude Code instances, retired after ten idle
/// minutes. `keep_alive` below only bounds the sessions rmcp still keeps for
/// legacy clients.
pub fn mcp_service(state: Arc<AppState>) -> StreamableHttpService<McpSession, LocalSessionManager> {
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(IDLE_TIMEOUT);
    StreamableHttpService::new(
        move || Ok(McpSession::new(state.clone())),
        Arc::new(sessions),
        // rmcp's default only accepts a loopback `Host`, which is DNS-rebinding
        // protection for an unauthenticated local server. This endpoint is
        // Bearer-authenticated, so a rebound page cannot forge the header, and
        // the demo is reached through the Vite proxy or over a tailnet, where
        // the `Host` is not loopback and the default would 403 every request.
        StreamableHttpServerConfig::default().disable_allowed_hosts(),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::test_util::{
        add_member, app, call, json_req, me, owned_project, register, serve, state, with_bearer,
    };

    /// Let a spawned task run to its next await, on a paused clock.
    async fn settle() {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }

    async fn identity_for(state: &Arc<AppState>, cookie: &str) -> McpIdentity {
        let owner = me(state, cookie).await;
        let bot = ensure_bot(&state.db, &owner).await.unwrap();
        McpIdentity { owner, bot }
    }

    #[test]
    fn every_read_only_tool_is_advertised() {
        let tools = McpSession::tool_router().list_all();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "add_caption",
                "add_title",
                "cut",
                "export",
                "find",
                "get_transcript",
                "list_projects",
                "look_at",
                "open_project",
                "overdub",
                "redo",
                "remove_fillers",
                "set_transition",
                "tighten_pauses",
                "undo"
            ]
        );
    }

    #[tokio::test]
    async fn open_project_joins_as_the_bot_and_returns_the_transcript() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        let out = session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        assert_eq!(out["title"], "Clip");
        assert_eq!(out["duration"], 10.0);
        assert_eq!(out["outputDuration"], 10.0);
        assert_eq!(out["cuts"], 0);
        assert_eq!(out["transcript"].as_array().unwrap().len(), 3);
        assert_eq!(out["transcript"][0]["status"], "kept");
        // No speakers file was seeded and diarization cannot run here, so the
        // transcript still comes back — unlabelled.
        assert_eq!(out["transcript"][0]["speaker"], Value::Null);
        let peers = state.bus.peers(&project.id);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].user.display_name, "Claude");
        assert!(peers[0].user.bot);
        // Membership was created at editor.
        let role = crate::projects::member_role(&state.db, &project.id, &identity.bot.id)
            .await
            .unwrap();
        assert_eq!(role, Some(Role::Editor));
    }

    #[tokio::test]
    async fn opening_a_second_project_leaves_the_first() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let first = owned_project(&state, &ada).await;
        let second = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &first.id)
            .await
            .unwrap();
        session
            .tool_open_project(&identity, &second.id)
            .await
            .unwrap();
        assert!(state.bus.peers(&first.id).is_empty());
        assert_eq!(state.bus.peers(&second.id).len(), 1);
    }

    #[tokio::test]
    async fn look_at_moves_presence_and_find_uses_indices() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let hits = session.tool_find(&identity, "b").await.unwrap();
        assert_eq!(hits, json!([{ "from": 1, "to": 1 }]));
        session.tool_look_at(&identity, 1, 2).await.unwrap();
        assert_eq!(
            state.bus.peers(&project.id)[0].state.selection,
            Some([1, 2])
        );
        // Out of range is the server's own 400, not a panic.
        let err = session.tool_look_at(&identity, 1, 9).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_transcript_reflects_later_edits() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{}/ops", project.id),
                Some(&ada),
                Some(json!({ "ops": [
                    { "opId": "c1", "kind": "cut", "start": 1.0, "end": 2.0 }
                ] })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let words = session.tool_get_transcript(&identity).await.unwrap();
        assert_eq!(words[0]["status"], "kept");
        assert_eq!(words[1]["status"], "cut");
    }

    #[tokio::test]
    async fn tools_without_an_open_project_or_membership_fail_clearly() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let session = McpSession::new(state.clone());
        let err = session
            .tool_find(&identity_for(&state, &ada).await, "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("open a project first"));
        let err = session
            .tool_open_project(&identity_for(&state, &bob).await, &project.id)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        let err = session
            .tool_open_project(&identity_for(&state, &ada).await, "nope")
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        // Even a tool whose arguments are wrong says what is actually wrong
        // first: there is no project open.
        let err = session
            .tool_set_transition(&identity_for(&state, &ada).await, "wipe")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("open a project first"), "{err}");
    }

    #[tokio::test]
    async fn list_projects_shows_what_the_owner_can_open() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let session = McpSession::new(state.clone());
        let mine = session
            .tool_list_projects(&identity_for(&state, &ada).await)
            .await
            .unwrap();
        assert_eq!(mine.as_array().unwrap().len(), 1);
        assert_eq!(mine[0]["id"], project.id);
        assert_eq!(mine[0]["role"], "owner");
        assert_eq!(mine[0]["duration"], 10.0);
        let theirs = session
            .tool_list_projects(&identity_for(&state, &bob).await)
            .await
            .unwrap();
        assert!(theirs.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn retiring_the_agent_announces_left() {
        // A handler no longer owns anything, so dropping one changes nothing:
        // the agent leaves when its entry does.
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity_for(&state, &ada).await, &project.id)
            .await
            .unwrap();
        assert_eq!(state.bus.peers(&project.id).len(), 1);
        drop(session);
        assert_eq!(state.bus.peers(&project.id).len(), 1);
        assert_eq!(state.agents.evict_idle(Duration::ZERO), 1);
        assert_eq!(state.bus.peers(&project.id).len(), 0);
        assert_eq!(state.agents.len(), 0);
    }

    #[tokio::test]
    async fn the_reaper_retires_an_agent_that_has_gone_quiet() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        // Paused only now that the database is up: sqlx's own timeouts fire
        // instantly against a clock that jumps.
        tokio::time::pause();
        let reaper = tokio::spawn(reap_idle_agents(state.clone()));

        // Still working: a call inside the window keeps the agent alive.
        tokio::time::advance(IDLE_TIMEOUT - Duration::from_secs(1)).await;
        session.tool_find(&identity, "b").await.unwrap();
        tokio::time::advance(IDLE_TIMEOUT - Duration::from_secs(1)).await;
        settle().await;
        assert_eq!(state.bus.peers(&project.id).len(), 1, "retired too eagerly");

        // Quiet for the whole timeout: the peer list loses it.
        tokio::time::advance(IDLE_TIMEOUT + REAP_EVERY).await;
        settle().await;
        assert_eq!(state.agents.len(), 0);
        assert_eq!(state.bus.peers(&project.id).len(), 0);
        // And the next call starts a fresh agent, with nothing open.
        assert!(session
            .tool_find(&identity, "b")
            .await
            .unwrap_err()
            .to_string()
            .contains("open a project first"));
        reaper.abort();
    }

    #[tokio::test]
    async fn two_handlers_for_one_token_share_the_open_project() {
        // Every request on the current lifecycle builds its own handler, so
        // what the agent has open has to be found by who it is, not by which
        // handler happens to be asking.
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        McpSession::new(state.clone())
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let out = McpSession::new(state.clone())
            .tool_find(&identity, "b")
            .await
            .unwrap();
        assert_eq!(out, json!([{ "from": 1, "to": 1 }]));
        // Still one peer: the second handler joined nothing of its own.
        assert_eq!(state.bus.peers(&project.id).len(), 1);
        // Somebody else's token is somebody else's agent.
        let err = McpSession::new(state.clone())
            .tool_find(&identity_for(&state, &bob).await, "b")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("open a project first"), "{err}");
    }

    #[tokio::test]
    async fn the_endpoint_is_closed_without_a_live_token() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

        let (status, err, _) = call(
            app(&state),
            json_req(Method::POST, "/mcp", None, Some(body.clone())),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(err["error"], "sign in first");

        let (status, _, _) = call(
            app(&state),
            with_bearer(
                json_req(Method::POST, "/mcp", None, Some(body.clone())),
                "tns_not-a-real-token",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // A live token gets past the layer: rmcp answers, not the 401.
        let (_, minted, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/tokens",
                Some(&ada),
                Some(json!({ "label": "laptop" })),
            ),
        )
        .await;
        let token = minted["token"].as_str().unwrap().to_owned();
        let (status, _, _) = call(
            app(&state),
            with_bearer(json_req(Method::POST, "/mcp", None, Some(body)), &token),
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);
        // Using the token minted the bot that the tools act as.
        let owner = me(&state, &ada).await;
        assert_eq!(
            ensure_bot(&state.db, &owner).await.unwrap().display_name,
            "Claude"
        );
    }

    #[tokio::test]
    async fn cut_moves_presence_then_appends_and_reports_touched_words() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let out = session.tool_cut(&identity, 1, 1).await.unwrap();
        assert_eq!(out["headSeq"], 1);
        assert_eq!(out["touched"][0]["status"], "cut");
        assert_eq!(
            state.bus.peers(&project.id)[0].state.selection,
            Some([1, 1])
        );
        let (_, got, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{}", project.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(
            got["doc"]["edits"][0],
            json!({ "kind": "cut", "start": 1.0, "end": 2.0 })
        );
        // The bot's undo is its own.
        let out = session.tool_undo(&identity).await.unwrap();
        assert_eq!(out["cuts"], 0);
        assert!(session
            .tool_undo(&identity)
            .await
            .unwrap_err()
            .to_string()
            .contains("nothing to undo"));
        // And what it undid, it can redo.
        let out = session.tool_redo(&identity).await.unwrap();
        assert_eq!(out["cuts"], 1);
    }

    #[tokio::test]
    async fn viewer_bot_cannot_edit_and_gets_the_403_text() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        add_member(&state, &ada, &project.id, "bob@example.com", "viewer").await;
        let identity = identity_for(&state, &bob).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let err = session.tool_cut(&identity, 0, 0).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(err.to_string(), "you can view this project but not edit it");
        // Nothing moved: a refused edit leaves no cursor behind.
        assert_eq!(state.bus.peers(&project.id)[0].state.selection, None);
    }

    #[tokio::test]
    async fn title_caption_transition_and_suggestions() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        session
            .tool_add_title(&identity, -1, "Intro", None, None, None)
            .await
            .unwrap();
        session
            .tool_add_title(
                &identity,
                0,
                "Part two",
                Some("sub"),
                Some("accent"),
                Some(2.0),
            )
            .await
            .unwrap();
        session
            .tool_add_caption(&identity, 1, 2, "Ada", Some("topLeft"))
            .await
            .unwrap();
        let out = session.tool_set_transition(&identity, "dip").await.unwrap();
        assert_eq!(out["transition"], "dip");
        let (_, got, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{}", project.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        let kinds: Vec<&str> = got["doc"]["edits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["title", "title", "caption"]);
        assert_eq!(got["doc"]["edits"][1]["at"], 0.5); // after word 0 (end 0.5)
        assert_eq!(got["doc"]["edits"][1]["style"], "accent");
        assert_eq!(got["doc"]["edits"][2]["position"], "topLeft");
        // Suggestions on the seeded words: no fillers, so a no-op with a clear message.
        let out = session.tool_remove_fillers(&identity).await.unwrap();
        assert_eq!(out["applied"], 0);
        assert_eq!(out["message"], "no fillers found");
    }

    #[tokio::test]
    async fn tighten_pauses_cuts_the_gaps_and_then_finds_nothing_left() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        // The seeded words are a second apart, which is no pause at all;
        // push the second one out so there is a silence worth tightening.
        let words = vec![
            engine::Word {
                id: "w0".into(),
                text: "a".into(),
                start: 0.0,
                end: 0.5,
            },
            engine::Word {
                id: "w1".into(),
                text: "b".into(),
                start: 5.0,
                end: 5.5,
            },
        ];
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(&project.media_id)
                .join(crate::routes::WORDS_CACHE),
            serde_json::to_vec(&words).unwrap(),
        )
        .await
        .unwrap();
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let out = session.tool_tighten_pauses(&identity).await.unwrap();
        assert!(out["applied"].as_u64().unwrap() >= 1, "{out}");
        assert!(out["outputDuration"].as_f64().unwrap() < 10.0);
        // A pause lies strictly between two words, so measuring words by the
        // time they are spoken would light up nothing at all. The cursor has
        // to land on the words either side of the gap, and the reply has to
        // name them.
        assert_eq!(
            state.bus.peers(&project.id)[0].state.selection,
            Some([0, 1]),
            "the cursor must cover the words around the tightened pause"
        );
        let touched: Vec<&str> = out["touched"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["text"].as_str().unwrap())
            .collect();
        assert_eq!(touched, ["a", "b"]);
        // Applied once, the same suggestions are already covered.
        let again = session.tool_tighten_pauses(&identity).await.unwrap();
        assert_eq!(again["applied"], 0);
        assert_eq!(again["message"], "no long pauses found");
    }

    #[tokio::test]
    async fn an_index_validated_against_a_swapped_transcript_is_a_400_not_a_panic() {
        // Every editing tool now works out what to do inside `edit`, under the
        // one guard that also moves the cursor and builds the report, so an
        // index can never be checked against one transcript and used against
        // another. Opening a shorter project is how that used to go wrong;
        // here the plan simply sees the new, shorter word list.
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let long = owned_project(&state, &ada).await;
        let short = owned_project(&state, &ada).await;
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(&short.media_id)
                .join(crate::routes::WORDS_CACHE),
            serde_json::to_vec(&[engine::Word {
                id: "w0".into(),
                text: "a".into(),
                start: 0.0,
                end: 0.5,
            }])
            .unwrap(),
        )
        .await
        .unwrap();
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &long.id)
            .await
            .unwrap();
        session
            .tool_open_project(&identity, &short.id)
            .await
            .unwrap();
        // Word 2 existed in the project that was open a moment ago.
        let err = session.tool_cut(&identity, 2, 2).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("transcript of 1"), "{err}");
        let err = session
            .tool_add_caption(&identity, 2, 2, "x", None)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        let err = session
            .tool_add_title(&identity, 2, "x", None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("transcript of 1"), "{err}");
    }

    #[tokio::test]
    async fn bad_style_position_and_transition_names_are_400s() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let err = session
            .tool_add_title(&identity, 0, "x", None, Some("neon"), None)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("dark, light or accent"));
        let err = session
            .tool_add_caption(&identity, 0, 0, "x", Some("middle"))
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        let err = session
            .tool_set_transition(&identity, "wipe")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("use none or dip"));
        // A title before the start is `-1`; anything further back is a slip.
        let err = session
            .tool_add_title(&identity, -2, "x", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("use -1"), "{err}");
        // An index the transcript does not have is the server's own 400.
        let err = session.tool_cut(&identity, 0, 9).await.unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("transcript of 3"));
    }

    #[tokio::test]
    async fn overdub_surfaces_the_upstream_failure() {
        // No VoiceStudio is reachable from a test, so the tool must pass the
        // synthesis failure through rather than swallow it.
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let err = session
            .tool_overdub(&identity, 1, 1, "hello there")
            .await
            .unwrap_err();
        assert!(err.status().is_server_error(), "{err}");
        // Nothing was appended, and the cursor never moved.
        let (_, doc) = ops::load_doc(&state, &project.id).await.unwrap();
        assert!(doc.edits.is_empty());
        assert_eq!(state.bus.peers(&project.id)[0].state.selection, None);
        // Empty text is rejected before anything is synthesized at all.
        let err = session
            .tool_overdub(&identity, 1, 1, "  ")
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "overdub text is empty");
    }

    #[tokio::test]
    async fn export_polls_to_completion_or_returns_the_job() {
        // seed_media has no real source, so ffmpeg fails fast: assert the tool
        // surfaces the job error text.
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let err = session.tool_export(&identity, None).await.unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("ffmpeg")
                || err.to_string().contains("export failed"),
            "{err}"
        );
        // An unknown format never starts a job at all.
        let err = session
            .tool_export(&identity, Some("gif"))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown format gif");
    }

    #[tokio::test]
    async fn end_to_end_over_streamable_http_with_a_browser_peer_watching() {
        use rmcp::model::{CallToolRequestParams, ProtocolVersion};
        use rmcp::ClientLifecycleMode;
        use rmcp::ClientServiceExt;

        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let (_, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/tokens",
                Some(&ada),
                Some(json!({ "label": "t" })),
            ),
        )
        .await;
        let token = body["token"].as_str().unwrap().to_owned();
        let base = serve(&state).await;

        // A browser-style peer.
        let mut ws = crate::ws::tests_support::connect(&base, &project.id, &ada)
            .await
            .unwrap();
        crate::ws::tests_support::next_json(&mut ws).await; // hello
        crate::ws::tests_support::next_json(&mut ws).await; // ada's own presence echo

        // The agent.
        let client = reqwest13::Client::builder()
            .default_headers({
                let mut h = reqwest13::header::HeaderMap::new();
                h.insert(
                    reqwest13::header::AUTHORIZATION,
                    format!("Bearer {token}").parse().unwrap(),
                );
                h
            })
            .build()
            .unwrap();
        let transport = rmcp::transport::StreamableHttpClientTransport::with_client(
            client,
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                format!("{base}/mcp"),
            ),
        );
        // Claude Code negotiates 2026-07-28, whose lifecycle has no sessions
        // at all (SEP-2567): every request is served statelessly, with a
        // fresh handler. Say so explicitly rather than take rmcp's default
        // `initialize`, or the test would exercise a lifecycle no real client
        // of ours uses and would miss the bug entirely.
        let agent = ()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .unwrap();
        let tools = agent.list_tools(Default::default()).await.unwrap();
        assert!(tools.tools.iter().any(|t| t.name == "cut"));
        agent
            .call_tool(
                CallToolRequestParams::new("open_project")
                    .with_arguments(rmcp::object!({ "project_id": project.id })),
            )
            .await
            .unwrap();
        let joined = crate::ws::tests_support::next_json(&mut ws).await;
        assert_eq!(joined["t"], "presence");
        assert_eq!(joined["user"]["displayName"], "Claude");
        let bot_conn = joined["connId"].as_str().unwrap().to_owned();

        agent
            .call_tool(
                CallToolRequestParams::new("cut")
                    .with_arguments(rmcp::object!({ "from": 1, "to": 1 })),
            )
            .await
            .unwrap();
        // presence (selection) then doc — and crucially no `left` in between,
        // which is what proves rmcp kept one session across both calls rather
        // than building a fresh handler per request.
        let mut saw_doc = false;
        for _ in 0..4 {
            let f = crate::ws::tests_support::next_json(&mut ws).await;
            assert_ne!(f["t"], "left", "the agent's peer churned between calls");
            if f["t"] == "doc" {
                assert_eq!(f["headSeq"], 1);
                saw_doc = true;
                break;
            }
        }
        assert!(saw_doc);
        assert_eq!(state.bus.peers(&project.id).len(), 2);

        // Nothing tells the server an agent has stopped: on this lifecycle
        // there is no session to end, and closing the client is invisible.
        // What retires it is going quiet, which is the reaper's job; evict
        // with a zero idle rather than wait ten minutes for it.
        agent.cancel().await.unwrap();
        assert_eq!(state.agents.evict_idle(Duration::ZERO), 1);
        let left = loop {
            let f = crate::ws::tests_support::next_json(&mut ws).await;
            if f["t"] == "left" {
                break f;
            }
        };
        assert_eq!(left["connId"], bot_conn);

        // Without a token the endpoint is closed.
        let resp = reqwest::Client::new()
            .post(format!("{base}/mcp"))
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
    }
}
