//! The MCP endpoint: an agent joins a project as a visible peer.
//!
//! `/mcp` is rmcp's streamable-HTTP transport nested into the same router as
//! the browser API. A tower layer in front of it turns an
//! `Authorization: Bearer` token into the person who minted it plus the bot
//! user that acts on their behalf; rmcp then builds one [`McpSession`] per
//! MCP session, which holds the open project and the hub subscription that
//! makes the agent show up in everyone's peer list. When rmcp drops the
//! session the subscription goes with it and the hub announces `left`.
//!
//! Every `#[tool]` here is a thin wrapper: it pulls the identity out of the
//! HTTP request parts rmcp forwards, calls the matching `tool_*` function —
//! which returns the server's own [`AppError`] on failure, so agents see the
//! same wording the browser does — and renders the JSON.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use engine::types::Word;
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
use crate::mcp_tools::{find_ranges, transcript};
use crate::projects::{ensure_bot_member, find_project, member_role, Project, Role};
use crate::{ops, projects, routes, tokens, AppState};

/// How long an editing tool leaves the agent's cursor on the range it is
/// about to change, so people watching see where the edit came from. Tests
/// do not wait.
#[cfg(not(test))]
#[allow(dead_code)] // The editing tools that dwell land with the next task.
pub const DWELL: Duration = Duration::from_millis(400);
#[cfg(test)]
#[allow(dead_code)]
pub const DWELL: Duration = Duration::ZERO;

/// An MCP session with no traffic for this long is dropped, which releases
/// the agent's subscription and clears its cursor from everyone's screen.
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

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
/// exist as a peer: the session never reads its own frames, and dropping it
/// is what tells everyone else the agent left.
#[derive(Default)]
struct Open {
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

/// One MCP session. rmcp may clone the handler, so the mutable half lives
/// behind an `Arc<Mutex<_>>`; the subscription drops with the last clone.
pub struct McpSession {
    state: Arc<AppState>,
    inner: Arc<Mutex<Open>>,
    /// Built once per session and dispatched through by `#[tool_handler]`.
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
}

impl McpSession {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            inner: Arc::new(Mutex::new(Open {
                conn_id: Uuid::new_v4().to_string(),
                ..Open::default()
            })),
            tool_router: Self::tool_router(),
        }
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

        let mut open = self.inner.lock().await;
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
        let open = self.inner.lock().await;
        let project = require_open(&open, identity)?;
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        Ok(serde_json::to_value(transcript(
            &open.words,
            open.speakers.as_deref(),
            &doc.edits,
        ))?)
    }

    pub(crate) async fn tool_find(&self, identity: &McpIdentity, text: &str) -> AppResult<Value> {
        let open = self.inner.lock().await;
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
        let open = self.inner.lock().await;
        let project = require_open(&open, identity)?;
        let words = words_in(&open, from, to)?;
        let state = PresenceState {
            playhead: open.words[from].start,
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
        Ok(json!({ "from": from, "to": to, "text": words }))
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
             edits are yours to undo.",
        )
    }
}

/// A project is open and this session is the one that opened it.
fn require_open<'a>(open: &'a Open, identity: &McpIdentity) -> AppResult<&'a Project> {
    let project = open
        .project
        .as_ref()
        .ok_or_else(|| AppError::bad_request("open a project first"))?;
    match &open.bot {
        Some(bot) if bot.id == identity.bot.id => Ok(project),
        // The token changed mid-session: whoever this is has not opened
        // anything, and must not inherit the previous agent's project.
        _ => Err(AppError::bad_request("open a project first")),
    }
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

/// The MCP transport, one session per connected agent.
pub fn mcp_service(state: Arc<AppState>) -> StreamableHttpService<McpSession, LocalSessionManager> {
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(IDLE_TIMEOUT);
    StreamableHttpService::new(
        move || Ok(McpSession::new(state.clone())),
        Arc::new(sessions),
        StreamableHttpServerConfig::default(),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::test_util::{app, call, json_req, me, owned_project, register, state, with_bearer};

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
                "find",
                "get_transcript",
                "list_projects",
                "look_at",
                "open_project"
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
    async fn dropping_the_session_announces_left() {
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
        assert_eq!(state.bus.peers(&project.id).len(), 0);
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
}
