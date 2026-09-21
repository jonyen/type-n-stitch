//! Per-project B-roll and music files: rows in `project_assets`, files under
//! `<media dir>/assets/<id>.<ext>`, served like the main media under `/data/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Multipart, Path as UrlPath, State};
use axum::Json;
use engine::{Edit, MediaKind};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Sqlite, SqlitePool};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::projects::{Project, ProjectAccess};
use crate::{media, AppState};

const ALLOWED_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "mp4", "mov", "aac", "ogg", "webm"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub kind: MediaKind,
    pub name: String,
    pub ext: String,
    pub duration: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub created_at: i64,
    pub url: String,
    pub poster: Option<String>,
}

type Row = (
    String,
    String,
    String,
    String,
    f64,
    Option<u32>,
    Option<u32>,
    i64,
);

fn kind_of(s: &str) -> MediaKind {
    if s == "video" {
        MediaKind::Video
    } else {
        MediaKind::Audio
    }
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    }
}

fn asset_from(media_id: &str, row: Row) -> Asset {
    let (id, kind, name, ext, duration, width, height, created_at) = row;
    let kind = kind_of(&kind);
    Asset {
        url: format!("/data/{media_id}/assets/{id}.{ext}"),
        poster: (kind == MediaKind::Video).then(|| format!("/data/{media_id}/assets/{id}.jpg")),
        id,
        kind,
        name,
        ext,
        duration,
        width,
        height,
        created_at,
    }
}

pub async fn list_for(db: &SqlitePool, project: &Project) -> AppResult<Vec<Asset>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, kind, name, ext, duration, width, height, created_at \
         FROM project_assets WHERE project_id = ? ORDER BY created_at, id",
    )
    .bind(&project.id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| asset_from(&project.media_id, r))
        .collect())
}

/// Not called yet — Task 5's validation and export wiring use this.
#[allow(dead_code)]
pub async fn find<'e, E>(
    exec: E,
    project_id: &str,
    asset_id: &str,
) -> AppResult<Option<(MediaKind, f64)>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<(String, f64)> =
        sqlx::query_as("SELECT kind, duration FROM project_assets WHERE project_id = ? AND id = ?")
            .bind(project_id)
            .bind(asset_id)
            .fetch_optional(exec)
            .await?;
    Ok(row.map(|(k, d)| (kind_of(&k), d)))
}

pub fn references(edits: &[Edit], asset_id: &str) -> (usize, usize) {
    edits.iter().fold((0, 0), |(b, a), e| match e {
        Edit::Broll { media, .. } if media == asset_id => (b + 1, a),
        Edit::Audio { media, .. } if media == asset_id => (b, a + 1),
        _ => (b, a),
    })
}

fn assets_dir(state: &AppState, project: &Project) -> PathBuf {
    state.config.data_dir.join(&project.media_id).join("assets")
}

/// Not called yet — Task 5's export wiring uses this.
#[allow(dead_code)]
pub async fn asset_files(
    state: &AppState,
    project: &Project,
    edits: &[Edit],
) -> AppResult<HashMap<String, PathBuf>> {
    let by_id: HashMap<String, Asset> = list_for(&state.db, project)
        .await?
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();
    let mut files = HashMap::new();
    for edit in edits {
        let (Edit::Broll { media, .. } | Edit::Audio { media, .. }) = edit else {
            continue;
        };
        let asset = by_id.get(media).ok_or_else(|| {
            AppError::bad_request(format!("asset {media} does not belong to this project"))
        })?;
        let path = assets_dir(state, project).join(format!("{}.{}", asset.id, asset.ext));
        if !path.is_file() {
            return Err(AppError::bad_request(format!(
                "the file for {} is missing",
                asset.name
            )));
        }
        files.insert(media.clone(), path);
    }
    Ok(files)
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Vec<Asset>>> {
    Ok(Json(list_for(&state.db, &access.project).await?))
}

pub async fn upload(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    mut multipart: Multipart,
) -> AppResult<Json<Asset>> {
    access.require_edit()?;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
    {
        if field.name() == Some("file") {
            return Ok(Json(store(&state, &access.project, field).await?));
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}

async fn store(
    state: &AppState,
    project: &Project,
    mut field: axum::extract::multipart::Field<'_>,
) -> AppResult<Asset> {
    let name = field.file_name().unwrap_or("asset").to_owned();
    let ext = Path::new(&name)
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
    let dir = assets_dir(state, project);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating assets dir")?;
    let path = dir.join(format!("{id}.{ext}"));
    // From here on, any failure leaves a file on disk with no row for it, so
    // every error path below removes it before returning.
    if let Err(e) = write_body(&path, &mut field).await {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    let probe = match media::probe(&path).await {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(AppError::bad_request(format!(
                "could not read media: {e:#}"
            )));
        }
    };
    if probe.kind == MediaKind::Video {
        if let Err(e) = media::poster_frame(&path, &dir.join(format!("{id}.jpg"))).await {
            tracing::warn!(id, "no poster frame: {e:#}");
        }
    }
    let (width, height) = probe
        .video
        .map_or((None, None), |v| (Some(v.width), Some(v.height)));
    let created_at = now();
    if let Err(e) = sqlx::query(
        "INSERT INTO project_assets (id, project_id, kind, name, ext, duration, width, height, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&project.id)
    .bind(kind_name(probe.kind))
    .bind(&name)
    .bind(&ext)
    .bind(probe.duration)
    .bind(width)
    .bind(height)
    .bind(created_at)
    .execute(&state.db)
    .await
    {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e.into());
    }
    Ok(asset_from(
        &project.media_id,
        (
            id,
            kind_name(probe.kind).to_owned(),
            name,
            ext,
            probe.duration,
            width,
            height,
            created_at,
        ),
    ))
}

/// Writes `field`'s chunks into the already-created file at `path`. Split out
/// so `store` can remove the on-disk file on any failure here, not just the
/// probe branch's.
async fn write_body(path: &Path, field: &mut axum::extract::multipart::Field<'_>) -> AppResult<()> {
    let mut file = tokio::fs::File::create(path)
        .await
        .context("creating asset")?;
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::bad_request(format!("upload interrupted: {e}")))?
    {
        file.write_all(&chunk).await.context("writing asset")?;
    }
    file.flush().await.context("flushing asset")?;
    Ok(())
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    UrlPath((_, asset_id)): UrlPath<(String, String)>,
) -> AppResult<Json<Value>> {
    access.require_edit()?;
    let project = &access.project;
    let asset = list_for(&state.db, project)
        .await?
        .into_iter()
        .find(|a| a.id == asset_id)
        .ok_or_else(|| AppError::not_found(format!("no asset {asset_id}")))?;
    let (_, doc) = crate::ops::load_doc(&state, &project.id).await?;
    let (brolls, audios) = references(&doc.edits, &asset.id);
    if brolls + audios > 0 {
        return Err(AppError::conflict(format!(
            "{} is used by {brolls} B-roll and {audios} music edit{}; remove them first",
            asset.name,
            if brolls + audios == 1 { "" } else { "s" }
        )));
    }
    sqlx::query("DELETE FROM project_assets WHERE project_id = ? AND id = ?")
        .bind(&project.id)
        .bind(&asset.id)
        .execute(&state.db)
        .await?;
    let dir = assets_dir(&state, project);
    let _ = tokio::fs::remove_file(dir.join(format!("{}.{}", asset.id, asset.ext))).await;
    let _ = tokio::fs::remove_file(dir.join(format!("{}.jpg", asset.id))).await;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// A row plus an empty file, so nothing shells out to ffprobe.
    pub async fn seed_asset(
        state: &Arc<AppState>,
        project: &Project,
        kind: MediaKind,
        duration: f64,
    ) -> Asset {
        let id = Uuid::new_v4().to_string();
        let ext = if kind == MediaKind::Video {
            "mp4"
        } else {
            "mp3"
        };
        let dir = assets_dir(state, project);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(format!("{id}.{ext}")), b"")
            .await
            .unwrap();
        let created_at = now();
        sqlx::query(
            "INSERT INTO project_assets (id, project_id, kind, name, ext, duration, width, height, created_at) VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?)",
        )
        .bind(&id)
        .bind(&project.id)
        .bind(kind_name(kind))
        .bind(format!("{id}.{ext}"))
        .bind(ext)
        .bind(duration)
        .bind(created_at)
        .execute(&state.db)
        .await
        .unwrap();
        asset_from(
            &project.media_id,
            (
                id.clone(),
                kind_name(kind).to_owned(),
                format!("{id}.{ext}"),
                ext.to_owned(),
                duration,
                None,
                None,
                created_at,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::test_support::seed_asset;
    use super::*;
    use crate::test_util::{add_member, app, call, json_req, owned_project, register, state};

    #[tokio::test]
    async fn members_list_assets_and_only_editors_delete() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        add_member(&state, &ada, &project.id, "bob@example.com", "viewer").await;
        let asset = seed_asset(&state, &project, MediaKind::Video, 4.0).await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{}/assets", project.id),
                Some(&bob),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body[0]["id"], asset.id);
        assert_eq!(body[0]["kind"], "video");
        assert_eq!(
            body[0]["url"],
            format!("/data/{}/assets/{}.mp4", project.media_id, asset.id)
        );
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::DELETE,
                &format!("/api/projects/{}/assets/{}", project.id, asset.id),
                Some(&bob),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::DELETE,
                &format!("/api/projects/{}/assets/{}", project.id, asset.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(list_for(&state.db, &project).await.unwrap().is_empty());
        assert!(!state
            .config
            .data_dir
            .join(&project.media_id)
            .join("assets")
            .join(format!("{}.mp4", asset.id))
            .exists());
    }

    #[tokio::test]
    async fn deleting_a_referenced_asset_is_409_naming_the_edits() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let asset = seed_asset(&state, &project, MediaKind::Audio, 30.0).await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{}/ops", project.id),
                Some(&ada),
                Some(json!({ "ops": [{ "opId": "a", "kind": "addaudio", "start": 0.0, "end": 5.0, "media": asset.id }] })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::DELETE,
                &format!("/api/projects/{}/assets/{}", project.id, asset.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body["error"].as_str().unwrap().contains("1 music"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn upload_rejects_unknown_extensions_and_viewers() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        add_member(&state, &ada, &project.id, "bob@example.com", "viewer").await;
        let req = |cookie: &str| {
            let body = "--x\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\r\nhi\r\n--x--\r\n";
            axum::http::Request::builder()
                .method(Method::POST)
                .uri(format!("/api/projects/{}/assets", project.id))
                .header(axum::http::header::COOKIE, cookie)
                .header(
                    axum::http::header::CONTENT_TYPE,
                    "multipart/form-data; boundary=x",
                )
                .body(axum::body::Body::from(body))
                .unwrap()
        };
        let (status, _, _) = call(app(&state), req(&bob)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body, _) = call(app(&state), req(&ada)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    /// A file with an allowed extension but unreadable content fails at the
    /// ffprobe step (in `store`, after the body is fully written), so this
    /// exercises the same on-any-error cleanup path a mid-write failure
    /// would take: nothing is left on disk for a row that was never inserted.
    #[tokio::test]
    async fn upload_that_fails_to_probe_leaves_no_orphaned_file() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let body = "--x\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.mp3\"\r\n\r\nnot really audio\r\n--x--\r\n";
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri(format!("/api/projects/{}/assets", project.id))
            .header(axum::http::header::COOKIE, &ada)
            .header(
                axum::http::header::CONTENT_TYPE,
                "multipart/form-data; boundary=x",
            )
            .body(axum::body::Body::from(body))
            .unwrap();
        let (status, body, _) = call(app(&state), req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(list_for(&state.db, &project).await.unwrap().is_empty());
        let dir = state.config.data_dir.join(&project.media_id).join("assets");
        let leftover = match tokio::fs::read_dir(&dir).await {
            Ok(mut entries) => entries.next_entry().await.unwrap().is_some(),
            Err(_) => false,
        };
        assert!(!leftover, "expected no files left in {}", dir.display());
    }

    #[test]
    fn references_count_by_kind() {
        let edits = [
            Edit::Broll {
                start: 0.0,
                end: 1.0,
                media: "a".into(),
                offset: 0.0,
            },
            Edit::Audio {
                start: 0.0,
                end: 1.0,
                media: "a".into(),
                offset: 0.0,
                gain: 0.0,
                duck: true,
            },
            Edit::Audio {
                start: 2.0,
                end: 3.0,
                media: "b".into(),
                offset: 0.0,
                gain: 0.0,
                duck: true,
            },
        ];
        assert_eq!(references(&edits, "a"), (1, 1));
        assert_eq!(references(&edits, "b"), (0, 1));
    }
}
