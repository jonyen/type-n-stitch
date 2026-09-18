//! The starter library: clips listed in `samples/library.json` that the user
//! can open without uploading anything. Files are fetched separately by
//! `samples/fetch-library.mjs` into `samples/library/`; entries whose file is
//! missing are listed as unavailable.
//!
//! Opening a clip imports it like an upload, but under a deterministic media
//! id (derived from the slug and file size), so the transcript cache under
//! `data/<id>/` survives restarts and the startup warm-up can pre-transcribe.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path as UrlPath, State};
use axum::Json;
use engine::MediaKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::projects::{create_project, summary, Project, ProjectSummary, Role};
use crate::routes::{read_meta, speakers_item, transcribe_item, Meta};
use crate::{media, AppState};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    slug: String,
    title: String,
    blurb: String,
    author: String,
    source_title: String,
    /// YouTube id for downloaded clips; absent for local files like the synthetic demo.
    youtube_id: Option<String>,
    /// File relative to the samples directory. Defaults to `library/<slug>.<mp4|mp3>`.
    file: Option<String>,
    #[serde(default)]
    audio_only: bool,
    start: Option<f64>,
    end: Option<f64>,
}

impl Entry {
    fn path(&self, samples: &Path) -> PathBuf {
        match &self.file {
            Some(file) => samples.join(file),
            None => {
                let ext = if self.audio_only { "mp3" } else { "mp4" };
                samples.join("library").join(format!("{}.{ext}", self.slug))
            }
        }
    }

    fn poster(&self, samples: &Path) -> Option<String> {
        let name = format!("{}.jpg", self.slug);
        samples
            .join("library")
            .join(&name)
            .is_file()
            .then(|| format!("/library/{name}"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    slug: String,
    title: String,
    blurb: String,
    author: String,
    source_title: String,
    source_url: Option<String>,
    kind: MediaKind,
    /// Clip length in seconds, from the manifest window when known.
    duration: Option<f64>,
    poster: Option<String>,
    available: bool,
}

async fn read_entries(samples: &Path) -> anyhow::Result<Vec<Entry>> {
    let manifest = samples.join("library.json");
    let json = match tokio::fs::read_to_string(&manifest).await {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading library.json"),
    };
    serde_json::from_str(&json).context("parsing library.json")
}

/// `GET /api/library` — every manifest entry, available or not.
pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Vec<LibraryItem>>> {
    let samples = &state.config.samples_dir;
    let items = read_entries(samples)
        .await?
        .into_iter()
        .map(|e| LibraryItem {
            available: e.path(samples).is_file(),
            poster: e.poster(samples),
            source_url: e
                .youtube_id
                .as_ref()
                .map(|id| format!("https://www.youtube.com/watch?v={id}")),
            kind: if e.audio_only {
                MediaKind::Audio
            } else {
                MediaKind::Video
            },
            duration: e.start.zip(e.end).map(|(s, t)| t - s),
            slug: e.slug,
            title: e.title,
            blurb: e.blurb,
            author: e.author,
            source_title: e.source_title,
        })
        .collect();
    Ok(Json(items))
}

/// `POST /api/library/:slug` — import a library clip (once) and open a
/// project on it for the caller, reusing their existing one if any.
pub async fn open(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    UrlPath(slug): UrlPath<String>,
) -> AppResult<Json<ProjectSummary>> {
    let entries = read_entries(&state.config.samples_dir).await?;
    let entry = entries
        .iter()
        .find(|e| e.slug == slug)
        .ok_or_else(|| AppError::not_found(format!("no library clip {slug}")))?;
    let meta = import(&state, entry).await?;
    let existing: Option<Project> = sqlx::query_as(
        "SELECT p.id, p.media_id, p.owner_id, p.title, p.created_at FROM projects p
         JOIN project_members m ON m.project_id = p.id
         WHERE p.media_id = ? AND m.user_id = ? ORDER BY p.created_at LIMIT 1",
    )
    .bind(&meta.id)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await?;
    let project = match existing {
        Some(p) => p,
        None => create_project(&state.db, &user, &meta.id, &entry.title).await?,
    };
    let role: Option<(String,)> =
        sqlx::query_as("SELECT role FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(&project.id)
            .bind(&user.id)
            .fetch_optional(&state.db)
            .await?;
    let role = role
        .and_then(|(r,)| Role::parse(&r))
        .unwrap_or(Role::Viewer);
    Ok(Json(summary(&state, &project, role).await?))
}

async fn import(state: &AppState, entry: &Entry) -> AppResult<Meta> {
    let file = entry.path(&state.config.samples_dir);
    let size = tokio::fs::metadata(&file)
        .await
        .map_err(|_| {
            AppError::not_found(format!(
                "{} is not downloaded yet; run `npm run library`",
                entry.title
            ))
        })?
        .len();
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("type-n-stitch:library:{}:{size}", entry.slug).as_bytes(),
    )
    .to_string();
    let dir = state.config.data_dir.join(&id);
    if dir.join("meta.json").is_file() {
        return read_meta(&dir).await;
    }

    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating media dir")?;
    let source_name = format!("source.{ext}");
    let source = dir.join(&source_name);
    tokio::fs::copy(&file, &source)
        .await
        .context("copying library clip")?;
    let probe = media::probe(&source)
        .await
        .map_err(|e| AppError::bad_request(format!("could not read media: {e:#}")))?;
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename: format!("{}.{ext}", entry.slug),
        ext,
        duration: probe.duration,
        kind: probe.kind,
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)
        .await
        .context("writing meta.json")?;
    tracing::info!(slug = entry.slug, id = meta.id, "imported library clip");
    Ok(meta)
}

/// Import, transcribe and diarize every downloaded clip in the background, one at a
/// time, so opening a library clip is instant.
pub async fn warm(state: Arc<AppState>) {
    let entries = match read_entries(&state.config.samples_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("library warm-up skipped: {e:#}");
            return;
        }
    };
    for entry in entries {
        if !entry.path(&state.config.samples_dir).is_file() {
            continue;
        }
        let result = async {
            let meta = import(&state, &entry).await?;
            transcribe_item(&state, &meta.id).await?;
            speakers_item(&state, &meta.id).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(slug = entry.slug, "library warm-up failed: {e:?}");
        }
    }
    tracing::info!("library warm-up done");
}
