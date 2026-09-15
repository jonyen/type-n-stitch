//! HTTP handlers. Each media item lives in `data/<id>/`:
//! `source.<ext>`, `meta.json`, `whisper.wav`, `words.json`,
//! `overdub-<n>.wav` and `export-<n>.<ext>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Multipart, Path as UrlPath, State};
use axum::Json;
use engine::{
    build_ffmpeg_args, filler_cuts, output_duration, pause_cuts, timeline, Edit, ExportOptions,
    MediaKind, OutputFormat, SuggestOptions, Word,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::{media, tts, AppState};

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "mp4", "mov", "aac", "ogg", "webm"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    id: String,
    filename: String,
    ext: String,
    duration: f64,
    kind: MediaKind,
    url: String,
}

/// A media item's directory, validated so `id` can't escape `data/`.
fn item_dir(state: &AppState, id: &str) -> AppResult<PathBuf> {
    Uuid::parse_str(id).map_err(|_| AppError::bad_request("malformed media id"))?;
    let dir = state.config.data_dir.join(id);
    if !dir.is_dir() {
        return Err(AppError::not_found(format!("no media with id {id}")));
    }
    Ok(dir)
}

async fn read_meta(dir: &Path) -> AppResult<Meta> {
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

/// `POST /api/media` — multipart with one `file` field.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> AppResult<Json<Meta>> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
    {
        if field.name() == Some("file") {
            return store_upload(&state, field).await.map(Json);
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}

async fn store_upload(
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

    let mut file = tokio::fs::File::create(&source)
        .await
        .context("creating upload")?;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::bad_request(format!("upload interrupted: {e}")))?
    {
        file.write_all(&chunk).await.context("writing upload")?;
    }
    file.flush().await.context("flushing upload")?;

    let probe = media::probe(&source)
        .await
        .map_err(|e| AppError::bad_request(format!("could not read media: {e:#}")))?;
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename,
        ext,
        duration: probe.duration,
        kind: probe.kind,
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)
        .await
        .context("writing meta.json")?;
    tracing::info!(id = meta.id, duration = meta.duration, kind = ?meta.kind, "uploaded");
    Ok(meta)
}

#[derive(Serialize)]
pub struct Transcript {
    words: Vec<Word>,
}

/// `POST /api/media/:id/transcribe` — whisper.cpp word timestamps (cached).
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> AppResult<Json<Transcript>> {
    let dir = item_dir(&state, &id)?;
    let cached = dir.join("words.json");
    if let Ok(json) = tokio::fs::read_to_string(&cached).await {
        let words: Vec<Word> = serde_json::from_str(&json).context("parsing cached words")?;
        return Ok(Json(Transcript { words }));
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

    tokio::fs::write(&cached, serde_json::to_vec(&words)?)
        .await
        .context("caching words")?;
    tracing::info!(id, words = words.len(), "transcribed");
    Ok(Json(Transcript { words }))
}

/// Cached transcript, or a 404 if the item has not been transcribed yet.
async fn read_words(dir: &Path) -> AppResult<Vec<Word>> {
    let json = tokio::fs::read_to_string(dir.join("words.json"))
        .await
        .map_err(|_| AppError::not_found("transcribe this media first"))?;
    Ok(serde_json::from_str(&json).context("parsing cached words")?)
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestRequest {
    #[serde(default)]
    two_word_fillers: bool,
}

#[derive(Serialize)]
pub struct Suggestions {
    fillers: Vec<Edit>,
    pauses: Vec<Edit>,
}

/// `POST /api/media/:id/suggest` — filler-word and long-pause cuts the
/// client can apply as one batch. The body is optional.
pub async fn suggest(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    body: Option<Json<SuggestRequest>>,
) -> AppResult<Json<Suggestions>> {
    let dir = item_dir(&state, &id)?;
    let meta = read_meta(&dir).await?;
    let words = read_words(&dir).await?;
    let opts = SuggestOptions {
        two_word_fillers: body.is_some_and(|Json(b)| b.two_word_fillers),
        ..SuggestOptions::default()
    };
    Ok(Json(Suggestions {
        fillers: filler_cuts(&words, meta.duration, &opts),
        pauses: pause_cuts(&words, &opts),
    }))
}

#[derive(Deserialize)]
pub struct OverdubRequest {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverdubResponse {
    audio_url: String,
    duration: f64,
}

/// `POST /api/media/:id/overdub` — synthesize replacement speech.
pub async fn overdub(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(req): Json<OverdubRequest>,
) -> AppResult<Json<OverdubResponse>> {
    let dir = item_dir(&state, &id)?;
    let text = req.text.trim();
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
    tracing::info!(id, duration, "overdub synthesized");
    Ok(Json(OverdubResponse {
        audio_url: format!("/data/{id}/{name}"),
        duration,
    }))
}

#[derive(Deserialize)]
pub struct ExportRequest {
    edits: Vec<Edit>,
    /// `mp4` (video sources only), `mp3` or `wav`. Defaults by source kind.
    format: Option<String>,
}

#[derive(Serialize)]
pub struct ExportResponse {
    url: String,
    duration: f64,
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

/// `POST /api/media/:id/export` — render the edit list with ffmpeg.
pub async fn export(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(req): Json<ExportRequest>,
) -> AppResult<Json<ExportResponse>> {
    let dir = item_dir(&state, &id)?;
    let meta = read_meta(&dir).await?;
    let format = match req.format.as_deref() {
        None => OutputFormat::for_kind(meta.kind),
        Some("mp4") => OutputFormat::Mp4,
        Some("mp3") => OutputFormat::Mp3,
        Some("wav") => OutputFormat::Wav,
        Some(other) => return Err(AppError::bad_request(format!("unknown format {other}"))),
    };
    let overdub_audio = overdub_files(&id, &dir, &req.edits)?;
    let (output, name) = next_numbered(&dir, "export", format.extension()).await?;
    let source = dir.join(format!("source.{}", meta.ext));

    let args = build_ffmpeg_args(
        &source,
        &req.edits,
        &ExportOptions {
            duration: meta.duration,
            kind: meta.kind,
            format,
            output: &output,
            overdub_audio: &overdub_audio,
        },
    )
    .map_err(|e| AppError::bad_request(e.to_string()))?;

    tracing::info!(id, edits = req.edits.len(), "rendering with ffmpeg");
    media::ffmpeg(&args)
        .await
        .map_err(|e| AppError::upstream(format!("{e:#}")))?;

    let planned = output_duration(&timeline(meta.duration, &req.edits));
    let duration = media::duration(&output).await.unwrap_or(planned);
    tracing::info!(id, duration, planned, "export done");
    Ok(Json(ExportResponse {
        url: format!("/data/{id}/{name}"),
        duration,
    }))
}
