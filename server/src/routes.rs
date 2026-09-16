//! HTTP handlers. Each media item lives in `data/<id>/`:
//! `source.<ext>`, `meta.json`, `whisper.wav`, `words-v2.json`,
//! `overdub-<n>.wav` and `export-<n>.<ext>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Multipart, Path as UrlPath, State};
use axum::Json;
use engine::{
    assign_speakers, build_ffmpeg_args, filler_cuts, output_duration, pause_cuts,
    silence_pause_cuts, thumbnail_args, thumbnail_sheet, timeline, Edit, ExportOptions, MediaKind,
    OutputFormat, Range, SpeakerTurn, SuggestOptions, ThumbnailSheet, Word,
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
    pub id: String,
    pub filename: String,
    pub ext: String,
    pub duration: f64,
    pub kind: MediaKind,
    pub url: String,
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

/// Transcript cache file. Bump the version when the whisper invocation changes
/// in a way that alters the words (v2: disfluency prompt keeps "um"/"uh").
const WORDS_CACHE: &str = "words-v2.json";

/// `POST /api/media/:id/transcribe` — whisper.cpp word timestamps (cached).
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> AppResult<Json<Transcript>> {
    let words = transcribe_item(&state, &id).await?;
    Ok(Json(Transcript { words }))
}

/// Words for a media item, running whisper.cpp only if nothing is cached.
pub async fn transcribe_item(state: &AppState, id: &str) -> AppResult<Vec<Word>> {
    let dir = item_dir(state, id)?;
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

    tokio::fs::write(&cached, serde_json::to_vec(&words)?)
        .await
        .context("caching words")?;
    tracing::info!(id, words = words.len(), "transcribed");
    Ok(words)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Speakers {
    /// Number of distinct speakers found.
    count: u32,
    /// Speaker index for each transcript word, parallel to the word list.
    words: Vec<Option<u32>>,
    turns: Vec<SpeakerTurn>,
}

/// Speaker cache file. Bump the version when diarization settings change.
const SPEAKERS_CACHE: &str = "speakers-v1.json";

/// `POST /api/media/:id/speakers` — who says each word (cached).
pub async fn speakers(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> AppResult<Json<Speakers>> {
    speakers_item(&state, &id).await.map(Json)
}

/// Speaker labels for a transcribed item, running the diarizer only if
/// nothing is cached.
pub async fn speakers_item(state: &AppState, id: &str) -> AppResult<Speakers> {
    let dir = item_dir(state, id)?;
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
    tokio::fs::write(&cached, serde_json::to_vec(&speakers)?)
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

/// `POST /api/media/:id/thumbnails` — a sprite sheet of frames for the
/// scrubber preview, rendered once per video and cached.
pub async fn thumbnails(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
) -> AppResult<Json<Thumbnails>> {
    let dir = item_dir(&state, &id)?;
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
    let pauses = match read_silences(&dir, meta.duration).await {
        Ok(silences) => silence_pause_cuts(&silences, meta.duration, &opts),
        Err(e) => {
            tracing::warn!(id, "silence detection failed, using word gaps: {e:#}");
            pause_cuts(&words, &opts)
        }
    };
    Ok(Json(Suggestions {
        fillers: filler_cuts(&words, meta.duration, &opts),
        pauses,
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
#[serde(rename_all = "camelCase")]
pub struct ExportStarted {
    job_id: String,
    /// Planned output length in seconds, for the client's progress bar.
    planned: f64,
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

fn set_job(state: &AppState, job_id: &str, job: ExportJob) {
    state
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(job_id.to_owned(), job);
}

/// `POST /api/media/:id/export` — plan the render, start ffmpeg in the
/// background and return a job id to poll.
pub async fn export(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(req): Json<ExportRequest>,
) -> AppResult<Json<ExportStarted>> {
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
    let planned = output_duration(&timeline(meta.duration, &req.edits));

    let job_id = Uuid::new_v4().to_string();
    set_job(
        &state,
        &job_id,
        ExportJob::Running {
            media_id: id.clone(),
            progress: 0.0,
        },
    );
    tracing::info!(
        id,
        job = job_id,
        edits = req.edits.len(),
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

    Ok(Json(ExportStarted { job_id, planned }))
}

/// `GET /api/media/:id/export/:job/progress` — poll an export.
pub async fn export_progress(
    State(state): State<Arc<AppState>>,
    UrlPath((id, job_id)): UrlPath<(String, String)>,
) -> AppResult<Json<ExportJob>> {
    item_dir(&state, &id)?;
    let job = state
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&job_id)
        .filter(|j| j.media_id() == id)
        .cloned()
        .ok_or_else(|| AppError::not_found(format!("no export job {job_id}")))?;
    Ok(Json(job))
}
