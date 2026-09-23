//! HTTP handlers for a project's media. Each media item lives in `data/<id>/`:
//! `source.<ext>`, `meta.json`, `whisper.wav`, `words-v2.json`,
//! `overdub-<n>.wav` and `export-<n>.<ext>`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Multipart, Path as UrlPath, State};
use axum::Json;
use engine::{
    assign_speakers, build_ffmpeg_args, canvas, filler_cuts, output_duration, pause_cuts,
    silence_pause_cuts, sources_kind, stitched_duration, text, thumbnail_args, thumbnail_sheet,
    timeline_with, Edit, ExportError, ExportOptions, MediaKind, OutputFormat, ProjectDoc, Range,
    Source, SourceInput, SpeakerTurn, SuggestOptions, ThumbnailSheet, VideoInfo, Word,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::ops::load_doc;
use crate::projects::{create_project, summary, Project, ProjectAccess, ProjectSummary};
use crate::{media, tts, AppState};

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "mp4", "mov", "aac", "ogg", "webm"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub id: String,
    pub filename: String,
    pub ext: String,
    pub duration: f64,
    pub kind: MediaKind,
    pub url: String,
    /// Frame size and rate of the picture, when known. Items stored before
    /// dimensions were recorded have none until the next export re-probes.
    #[serde(default)]
    pub video: Option<VideoInfo>,
}

/// A media item's directory, validated so `id` can't escape `data/`.
pub(crate) fn media_dir(state: &AppState, id: &str) -> AppResult<PathBuf> {
    Uuid::parse_str(id).map_err(|_| AppError::bad_request("malformed media id"))?;
    let dir = state.config.data_dir.join(id);
    if !dir.is_dir() {
        return Err(AppError::not_found(format!("no media with id {id}")));
    }
    Ok(dir)
}

pub async fn read_meta(dir: &Path) -> AppResult<Meta> {
    let json = tokio::fs::read_to_string(dir.join("meta.json"))
        .await
        .context("reading meta.json")?;
    Ok(serde_json::from_str(&json).context("parsing meta.json")?)
}

/// Next unused `prefix-<n>.<ext>` in `dir`.
async fn next_numbered(dir: &Path, prefix: &str, ext: &str) -> anyhow::Result<(PathBuf, String)> {
    for n in 0.. {
        let name = format!("{prefix}-{n}.{ext}");
        let path = dir.join(&name);
        if tokio::fs::try_exists(&path).await? {
            continue;
        }
        return Ok((path, name));
    }
    unreachable!()
}

pub async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// `POST /api/projects` — multipart with one `file` field; creates the
/// media item and a project owned by the caller.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    mut multipart: Multipart,
) -> AppResult<Json<ProjectSummary>> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::multipart(&e, ""))?
    {
        if field.name() == Some("file") {
            let meta = store_upload(&state, field).await?;
            let project = create_project(&state.db, &user, &meta.id, &meta.filename).await?;
            return Ok(Json(
                summary(&state, &project, crate::projects::Role::Owner).await?,
            ));
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}

/// Store a multipart `file` field as a new media item and probe it. On any
/// failure after the directory exists, the directory is removed, so a
/// rejected upload leaves nothing behind.
pub(crate) async fn store_upload(
    state: &AppState,
    mut field: axum::extract::multipart::Field<'_>,
) -> AppResult<Meta> {
    let filename = field.file_name().unwrap_or("upload").to_owned();
    let ext = Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(AppError::bad_request(format!(
            "unsupported file type .{ext}; use one of {}",
            ALLOWED_EXTENSIONS.join(", ")
        )));
    }

    let id = Uuid::new_v4().to_string();
    let dir = state.config.data_dir.join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating media dir")?;
    let source_name = format!("source.{ext}");
    let source = dir.join(&source_name);

    let stored = async {
        let mut file = tokio::fs::File::create(&source)
            .await
            .context("creating upload")?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::multipart(&e, "upload interrupted"))?
        {
            file.write_all(&chunk).await.context("writing upload")?;
        }
        file.flush().await.context("flushing upload")?;
        media::probe(&source)
            .await
            .map_err(|e| AppError::bad_request(format!("could not read media: {e:#}")))
    }
    .await;
    let probe = match stored {
        Ok(probe) => probe,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
    };
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename,
        ext,
        duration: probe.duration,
        kind: probe.kind,
        video: probe.video,
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)
        .await
        .context("writing meta.json")?;
    tracing::info!(id = meta.id, duration = meta.duration, kind = ?meta.kind, "uploaded");
    Ok(meta)
}

/// Transcript cache file. Bump the version when the whisper invocation changes
/// in a way that alters the words (v2: disfluency prompt keeps "um"/"uh").
pub(crate) const WORDS_CACHE: &str = "words-v2.json";

#[derive(Serialize)]
pub struct Transcript {
    /// Every ready source's words in stitched time.
    words: Vec<Word>,
    /// Every source, with how far its transcript has got.
    sources: Vec<crate::sources::SourceView>,
}

/// `POST /api/projects/:id/transcribe` — the project's stitched words, and
/// each source's transcript status.
///
/// The first source is transcribed before answering, as it always was, so
/// a client that predates sources gets its words exactly as before. Every
/// other source is transcribed in the background; the client asks again
/// while any is `pending` or `running`.
///
/// Deliberately open to every member, viewers included: the transcript *is*
/// the document, so a viewer cannot see the project without it. The result is
/// cached per media, so a viewer cannot force repeated work either.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Transcript>> {
    transcribe_item(&state, &access.project.media_id).await?;
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    for source in sources.iter().skip(1) {
        crate::sources::start_transcription(&state, &source.media);
    }
    let words = crate::sources::stitched_words(&state, &sources).await?;
    let sources = crate::sources::views(&state, &sources).await?;
    Ok(Json(Transcript { words, sources }))
}

/// Words for a media item, running whisper.cpp only if nothing is cached.
pub async fn transcribe_item(state: &AppState, id: &str) -> AppResult<Vec<Word>> {
    let dir = media_dir(state, id)?;
    let cached = dir.join(WORDS_CACHE);
    if let Ok(json) = tokio::fs::read_to_string(&cached).await {
        return Ok(serde_json::from_str(&json).context("parsing cached words")?);
    }

    let meta = read_meta(&dir).await?;
    let source = dir.join(format!("source.{}", meta.ext));
    let wav = dir.join("whisper.wav");
    media::to_whisper_wav(&source, &wav)
        .await
        .map_err(|e| AppError::upstream(format!("{e:#}")))?;
    let words = media::transcribe(
        &state.config.whisper_bin,
        &state.config.whisper_model,
        &wav,
        &dir.join("whisper"),
    )
    .await
    .map_err(|e| AppError::upstream(format!("{e:#}")))?;

    // Atomically: a client polling `/transcribe` must never read half a file.
    write_json_atomic(&cached, &serde_json::to_vec(&words)?)
        .await
        .context("caching words")?;
    tracing::info!(id, words = words.len(), "transcribed");
    Ok(words)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Speakers {
    /// Number of distinct speakers found.
    pub count: u32,
    /// Speaker index for each transcript word, parallel to the word list.
    pub words: Vec<Option<u32>>,
    pub turns: Vec<SpeakerTurn>,
}

/// Speaker cache file. Bump the version when diarization settings change.
pub(crate) const SPEAKERS_CACHE: &str = "speakers-v1.json";

/// `POST /api/projects/:id/speakers` — who says each stitched word (cached
/// per media), namespaced per source.
///
/// Readable by every member, viewers included: speaker turns are part of
/// viewing the transcript, and the answer is cached per media.
pub async fn speakers(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Speakers>> {
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    crate::sources::stitched_speakers(&state, &sources)
        .await
        .map(Json)
}

/// Speaker labels for a transcribed item, running the diarizer only if
/// nothing is cached.
pub async fn speakers_item(state: &AppState, id: &str) -> AppResult<Speakers> {
    let dir = media_dir(state, id)?;
    let cached = dir.join(SPEAKERS_CACHE);
    if let Ok(json) = tokio::fs::read_to_string(&cached).await {
        return Ok(serde_json::from_str(&json).context("parsing cached speakers")?);
    }
    let words = read_words(&dir).await?;
    let turns = media::diarize(&state.config, &dir.join("whisper.wav"))
        .await
        .map_err(|e| AppError::upstream(format!("{e:#}")))?;
    let speakers = Speakers {
        count: turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0),
        words: assign_speakers(&words, &turns),
        turns,
    };
    write_json_atomic(&cached, &serde_json::to_vec(&speakers)?)
        .await
        .context("caching speakers")?;
    tracing::info!(id, speakers = speakers.count, "diarized");
    Ok(speakers)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnails {
    url: String,
    #[serde(flatten)]
    sheet: ThumbnailSheet,
}

/// Sprite sheet cache file. Bump the version when the sheet layout changes.
const THUMBS_CACHE: &str = "thumbs-v1.jpg";

#[derive(Default, Deserialize)]
pub struct ThumbnailsRequest {
    /// Which of the project's sources; the first when omitted.
    media: Option<String>,
}

/// `POST /api/projects/:id/thumbnails` — a sprite sheet of frames for the
/// scrubber preview, rendered once per video and cached. The body is
/// optional; `{ "media": id }` picks one of the project's sources.
///
/// Readable by every member, viewers included: the scrubber preview is part
/// of playback, and the sheet is rendered once per media and cached.
pub async fn thumbnails(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<ThumbnailsRequest>>,
) -> AppResult<Json<Thumbnails>> {
    let id = match body.and_then(|Json(b)| b.media) {
        Some(media) => {
            if !crate::sources::in_registry(&state.db, &access.project.id, &media).await? {
                return Err(AppError::bad_request(
                    "that media is not one of this project's videos",
                ));
            }
            media
        }
        None => access.project.media_id.clone(),
    };
    let dir = media_dir(&state, &id)?;
    let meta = read_meta(&dir).await?;
    if meta.kind != MediaKind::Video {
        return Err(AppError::bad_request("audio has no frames to preview"));
    }
    let sheet = thumbnail_sheet(meta.duration);
    let output = dir.join(THUMBS_CACHE);
    if !output.is_file() {
        // Render to a temp name so a failed or concurrent run never leaves a
        // half-written sheet behind under the cache name.
        let partial = dir.join(format!("thumbs-{}.jpg", Uuid::new_v4()));
        let source = dir.join(format!("source.{}", meta.ext));
        let args = thumbnail_args(
            &source.to_string_lossy(),
            &partial.to_string_lossy(),
            &sheet,
        );
        let rendered = media::ffmpeg(&args).await;
        if let Err(e) = rendered {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(AppError::upstream(format!("{e:#}")));
        }
        tokio::fs::rename(&partial, &output)
            .await
            .context("caching thumbnails")?;
        tracing::info!(id, count = sheet.count, "rendered thumbnails");
    }
    Ok(Json(Thumbnails {
        url: format!("/data/{id}/{THUMBS_CACHE}"),
        sheet,
    }))
}

/// Cached transcript, or a 404 if the item has not been transcribed yet.
async fn read_words(dir: &Path) -> AppResult<Vec<Word>> {
    let json = tokio::fs::read_to_string(dir.join(WORDS_CACHE))
        .await
        .map_err(|_| AppError::not_found("transcribe this media first"))?;
    Ok(serde_json::from_str(&json).context("parsing cached words")?)
}

/// Silence map for the item, computed once from `whisper.wav` and cached.
async fn read_silences(dir: &Path, duration: f64) -> anyhow::Result<Vec<Range>> {
    let cached = dir.join("silences-v1.json");
    if let Ok(json) = tokio::fs::read_to_string(&cached).await {
        return Ok(serde_json::from_str(&json)?);
    }
    let silences = media::silences(&dir.join("whisper.wav"), duration).await?;
    tokio::fs::write(&cached, serde_json::to_vec(&silences)?).await?;
    Ok(silences)
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestRequest {
    #[serde(default)]
    two_word_fillers: bool,
}

#[derive(Serialize)]
pub struct Suggestions {
    pub fillers: Vec<Edit>,
    pub pauses: Vec<Edit>,
}

/// `POST /api/projects/:id/suggest` — filler-word and long-pause cuts across
/// every ready source, in stitched time, which the client can apply as one
/// batch. The body is optional. Unlike the other read-only media routes this
/// one prepares edits, so it needs edit rights.
pub async fn suggest(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<SuggestRequest>>,
) -> AppResult<Json<Suggestions>> {
    access.require_edit()?;
    let two_word_fillers = body.is_some_and(|Json(b)| b.two_word_fillers);
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    crate::sources::suggest_project(&state, &sources, two_word_fillers)
        .await
        .map(Json)
}

/// Filler-word and long-pause cut suggestions for a media item.
pub async fn suggest_for(
    state: &AppState,
    media_id: &str,
    two_word: bool,
) -> AppResult<Suggestions> {
    let dir = media_dir(state, media_id)?;
    let meta = read_meta(&dir).await?;
    let words = read_words(&dir).await?;
    let opts = SuggestOptions {
        two_word_fillers: two_word,
        ..SuggestOptions::default()
    };
    let pauses = match read_silences(&dir, meta.duration).await {
        Ok(silences) => silence_pause_cuts(&silences, meta.duration, &opts),
        Err(e) => {
            tracing::warn!(
                id = media_id,
                "silence detection failed, using word gaps: {e:#}"
            );
            pause_cuts(&words, &opts)
        }
    };
    Ok(Suggestions {
        fillers: filler_cuts(&words, meta.duration, &opts),
        pauses,
    })
}

#[derive(Deserialize)]
pub struct OverdubRequest {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverdubResponse {
    pub audio_url: String,
    pub duration: f64,
}

/// `POST /api/projects/:id/overdub` — synthesize replacement speech.
pub async fn overdub(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    Json(req): Json<OverdubRequest>,
) -> AppResult<Json<OverdubResponse>> {
    access.require_edit()?;
    let id = access.project.media_id.clone();
    overdub_for(&state, &id, &req.text).await.map(Json)
}

/// Synthesize replacement speech for a media item, writing the audio into
/// its directory and returning its URL and duration.
pub async fn overdub_for(
    state: &AppState,
    media_id: &str,
    text: &str,
) -> AppResult<OverdubResponse> {
    let dir = media_dir(state, media_id)?;
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::bad_request("overdub text is empty"));
    }
    let (wav, name) = next_numbered(&dir, "overdub", "wav").await?;
    tts::synthesize(
        &state.http,
        &state.config.tts_base_url,
        &state.config.tts_voice,
        text,
        &wav,
    )
    .await?;
    let duration = media::duration(&wav)
        .await
        .map_err(|e| AppError::upstream(format!("synthesized audio unreadable: {e:#}")))?;
    tracing::info!(id = media_id, duration, "overdub synthesized");
    Ok(OverdubResponse {
        audio_url: format!("/data/{media_id}/{name}"),
        duration,
    })
}

#[derive(Default, Deserialize)]
pub struct ExportRequest {
    /// `mp4` (video sources only), `mp3` or `wav`. Defaults by source kind.
    format: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStarted {
    pub job_id: String,
    /// Planned output length in seconds, for the client's progress bar.
    pub planned: f64,
}

/// One export, from the moment ffmpeg starts until the client has read the
/// result. `progress` is a fraction of the planned output length.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ExportJob {
    Running {
        #[serde(skip)]
        media_id: String,
        progress: f64,
    },
    Done {
        #[serde(skip)]
        media_id: String,
        progress: f64,
        url: String,
        duration: f64,
        bytes: u64,
    },
    Error {
        #[serde(skip)]
        media_id: String,
        message: String,
    },
}

impl ExportJob {
    fn media_id(&self) -> &str {
        match self {
            ExportJob::Running { media_id, .. }
            | ExportJob::Done { media_id, .. }
            | ExportJob::Error { media_id, .. } => media_id,
        }
    }
}

/// Resolve each overdub's `/data/<id>/overdub-n.wav` URL to a file in `dir`.
fn overdub_files(id: &str, dir: &Path, edits: &[Edit]) -> AppResult<HashMap<String, PathBuf>> {
    let prefix = format!("/data/{id}/");
    let mut files = HashMap::new();
    for edit in edits {
        if let Edit::Overdub { audio_url, .. } = edit {
            let name = audio_url
                .strip_prefix(&prefix)
                .filter(|n| n.starts_with("overdub-") && n.ends_with(".wav") && !n.contains('/'))
                .ok_or_else(|| {
                    AppError::bad_request(format!(
                        "overdub audio {audio_url} does not belong to this media"
                    ))
                })?;
            let path = dir.join(name);
            if !path.is_file() {
                return Err(AppError::bad_request(format!(
                    "overdub audio {audio_url} is missing"
                )));
            }
            files.insert(audio_url.clone(), path);
        }
    }
    Ok(files)
}

/// A short, stable name-part for a rendered image, so a re-export with the
/// same text and frame size reuses the file it wrote last time.
fn image_hash(parts: &[&str], video: VideoInfo) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    video.width.hash(&mut hasher);
    video.height.hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

/// `write_atomic` for a file the handler rewrites off the async path.
async fn write_json_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let partial = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    tokio::fs::write(&partial, bytes)
        .await
        .with_context(|| format!("writing {}", partial.display()))?;
    tokio::fs::rename(&partial, path)
        .await
        .with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

/// Write `bytes` to a temporary neighbour and rename it into place, so a
/// concurrent export never hands ffmpeg a half-written image.
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let partial = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    std::fs::write(&partial, bytes).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

/// Rendered title PNGs by edit index, and caption PNGs with their placement.
type TextImages = (HashMap<usize, PathBuf>, HashMap<usize, (PathBuf, u32, u32)>);

/// Rasterise every title and caption into `dir`, returning the maps the
/// planner wants. Runs on a blocking thread: drawing glyphs is CPU work.
fn render_text_images(dir: &Path, edits: &[Edit], video: VideoInfo) -> anyhow::Result<TextImages> {
    let mut titles = HashMap::new();
    let mut captions = HashMap::new();
    for (index, edit) in edits.iter().enumerate() {
        match edit {
            Edit::Title {
                text,
                subtitle,
                style,
                ..
            } => {
                let style_name = format!("{style:?}");
                let hash = image_hash(
                    &[text, subtitle.as_deref().unwrap_or(""), &style_name],
                    video,
                );
                let path = dir.join(format!("title-{index}-{hash}.png"));
                if !path.is_file() {
                    let png = text::render_title(text, subtitle.as_deref(), *style, video).to_png();
                    write_atomic(&path, &png)?;
                }
                titles.insert(index, path);
            }
            Edit::Caption { text, position, .. } => {
                let position_name = format!("{position:?}");
                let hash = image_hash(&[text, &position_name], video);
                let path = dir.join(format!("caption-{index}-{hash}.png"));
                // The box is rendered either way: `overlay` needs its
                // placement, which only the raster knows.
                let drawn = text::render_caption(text, *position, video);
                if !path.is_file() {
                    write_atomic(&path, &drawn.raster.to_png())?;
                }
                captions.insert(index, (path, drawn.x, drawn.y));
            }
            _ => {}
        }
    }
    Ok((titles, captions))
}

fn set_job(state: &AppState, job_id: &str, job: ExportJob) {
    state
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(job_id.to_owned(), job);
}

/// `POST /api/projects/:id/export` — fold the log, plan the render, start
/// ffmpeg in the background and return a job id to poll.
pub async fn export(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<ExportRequest>>,
) -> AppResult<Json<ExportStarted>> {
    access.require_edit()?;
    let format = body.and_then(|Json(b)| b.format);
    start_export(&state, &access.project, format.as_deref())
        .await
        .map(Json)
}

/// A media item's source file and meta. Media probed before frame sizes were
/// recorded is probed once more and the answer remembered, so later exports
/// skip the extra ffprobe. A failure is not fatal — the render falls back to
/// `DEFAULT_VIDEO` — but it is worth saying out loud.
async fn source_meta(state: &AppState, id: &str) -> AppResult<(PathBuf, Meta)> {
    let (path, mut meta) = crate::sources::source_file(state, id).await?;
    if meta.kind == MediaKind::Video && meta.video.is_none() {
        match media::probe(&path).await {
            Ok(probe) => {
                meta.video = probe.video;
                let json = serde_json::to_vec_pretty(&meta)?;
                if let Err(e) = write_json_atomic(&path.with_file_name("meta.json"), &json).await {
                    tracing::warn!(id, "could not record the frame size: {e:#}");
                }
            }
            Err(e) => tracing::warn!(
                id,
                "could not re-probe the frame size, rendering at the default: {e:#}"
            ),
        }
    }
    Ok((path, meta))
}

/// Every main-track file of `project` in stitched order, as the planner wants
/// them, and the stitched transcript for ducking. The project's own media is
/// source 0; `doc_sources` (the fold's) follow. A source with no transcript
/// yet contributes no words: ducking is best-effort.
pub(crate) async fn export_sources(
    state: &AppState,
    project: &Project,
    doc_sources: &[Source],
) -> AppResult<(Vec<SourceInput>, Vec<Word>)> {
    let doc = ProjectDoc {
        sources: doc_sources.to_vec(),
        ..ProjectDoc::default()
    };
    let placed = crate::sources::timeline(state, project, &doc).await?;
    let mut inputs = Vec::with_capacity(placed.len());
    for source in &placed {
        let (path, meta) = source_meta(state, &source.media).await?;
        inputs.push(SourceInput {
            source: source.clone(),
            path,
            kind: meta.kind,
            video: meta.video,
        });
    }
    let words = crate::sources::stitched_words(state, &placed)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(project = project.id, "exporting without ducking: {e:?}");
            Vec::new()
        });
    Ok((inputs, words))
}

/// Fold the log, plan the render and start ffmpeg in the background for
/// `project`, returning a job id to poll. `format` overrides the source
/// kind's default (`mp4`, `mp3` or `wav`).
pub async fn start_export(
    state: &Arc<AppState>,
    project: &Project,
    format: Option<&str>,
) -> AppResult<ExportStarted> {
    let id = project.media_id.clone();
    // Overdub WAVs and the rendered file live under the project's first media.
    let dir = media_dir(state, &id)?;
    let (_, doc) = load_doc(state, &project.id).await?;
    let (sources, words) = export_sources(state, project, &doc.sources).await?;
    let kind = sources_kind(&sources);
    let video = canvas(&sources);
    let duration = stitched_duration(&sources.iter().map(|s| s.source.clone()).collect::<Vec<_>>());
    // A title or caption with nothing but whitespace draws nothing; dropping
    // it here keeps the planner from asking for an image that would be blank.
    // `validate` rejects blank text on the way in, so this only catches
    // anything logged before that check existed.
    let edits: Vec<Edit> = doc
        .edits
        .into_iter()
        .filter(|e| match e {
            Edit::Caption { text, .. } | Edit::Title { text, .. } => !text.trim().is_empty(),
            _ => true,
        })
        .collect();
    let format = match format {
        None => OutputFormat::for_kind(kind),
        Some("mp4") => OutputFormat::Mp4,
        Some("mp3") => OutputFormat::Mp3,
        Some("wav") => OutputFormat::Wav,
        Some(other) => return Err(AppError::bad_request(format!("unknown format {other}"))),
    };
    let overdub_audio = overdub_files(&id, &dir, &edits)?;
    let asset_files = crate::assets::asset_files(state, project, &edits).await?;
    let (output, name) = next_numbered(&dir, "export", format.extension()).await?;

    // An audio-only render draws no picture, so it needs no images at all.
    // Text is rasterised at the canvas, the frame every source is fitted to.
    let render_video = kind == MediaKind::Video && format == OutputFormat::Mp4;
    let (title_images, caption_images) = if render_video {
        let dir = dir.clone();
        let edits = edits.clone();
        tokio::task::spawn_blocking(move || render_text_images(&dir, &edits, video))
            .await
            .context("rendering title and caption images")??
    } else {
        (HashMap::new(), HashMap::new())
    };

    let args = build_ffmpeg_args(
        &sources,
        &edits,
        &ExportOptions {
            kind,
            format,
            output: &output,
            overdub_audio: &overdub_audio,
            title_images: &title_images,
            caption_images: &caption_images,
            transition: doc.transition,
            splits: &doc.splits,
            order: &doc.order,
            assets: &asset_files,
            words: &words,
        },
    )
    .map_err(|e| match e {
        // The planner and the renderer disagreed about the edit indices:
        // nothing the client sent can fix that.
        ExportError::MissingTitleImage(_) | ExportError::MissingCaptionImage(_) => {
            AppError::internal(e.to_string())
        }
        other => AppError::bad_request(other.to_string()),
    })?;
    let planned = output_duration(&timeline_with(duration, &edits, &doc.splits, &doc.order));

    let job_id = Uuid::new_v4().to_string();
    set_job(
        state,
        &job_id,
        ExportJob::Running {
            media_id: id.clone(),
            progress: 0.0,
        },
    );
    tracing::info!(
        id,
        job = job_id,
        edits = edits.len(),
        planned,
        "rendering with ffmpeg"
    );

    let task_state = state.clone();
    let task_job = job_id.clone();
    let url = format!("/data/{id}/{name}");
    tokio::spawn(async move {
        let media_id = id.clone();
        let on_progress = |progress: f64| {
            set_job(
                &task_state,
                &task_job,
                ExportJob::Running {
                    media_id: media_id.clone(),
                    progress,
                },
            );
        };
        let job = match media::ffmpeg_with_progress(&args, planned, on_progress).await {
            Ok(()) => {
                let duration = media::duration(&output).await.unwrap_or(planned);
                let bytes = tokio::fs::metadata(&output).await.map_or(0, |m| m.len());
                tracing::info!(id, job = task_job, duration, bytes, "export done");
                ExportJob::Done {
                    media_id: id,
                    progress: 1.0,
                    url,
                    duration,
                    bytes,
                }
            }
            Err(e) => {
                tracing::error!(id, job = task_job, "export failed: {e:#}");
                ExportJob::Error {
                    media_id: id,
                    message: format!("{e:#}"),
                }
            }
        };
        set_job(&task_state, &task_job, job);
    });

    Ok(ExportStarted { job_id, planned })
}

/// `GET /api/projects/:id/export/:job/progress` — poll an export.
pub async fn export_progress(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    UrlPath((_, job_id)): UrlPath<(String, String)>,
) -> AppResult<Json<ExportJob>> {
    let id = access.project.media_id;
    export_job(&state, &id, &job_id)
        .map(Json)
        .ok_or_else(|| AppError::not_found(format!("no export job {job_id}")))
}

/// The export job `job_id` for `media_id`, if one exists and belongs to it.
pub fn export_job(state: &AppState, media_id: &str, job_id: &str) -> Option<ExportJob> {
    state
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(job_id)
        .filter(|j| j.media_id() == media_id)
        .cloned()
}
