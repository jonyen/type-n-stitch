//! A project's sources: the videos laid end to end on its main track.
//!
//! Two records describe them, on purpose. `project_sources` is a registry of
//! every media item uploaded into the project — for listing, membership
//! checks and `/data` lookups. `POST /sources` appends to it and nothing ever
//! removes a row, not even an undo. What is on the timeline is the fold's:
//! the project's own media first, then every live `AddSource`
//! (`engine::all_sources`). Registry position 0 is the project's own media;
//! its timing lives in `meta.json`, so that row records zeros.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Multipart, State};
use axum::Json;
use engine::{all_sources, stitched_duration, MediaKind, Op, ProjectDoc, Source};
use serde::Serialize;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};
use uuid::Uuid;

use crate::auth::User;
use crate::error::{AppError, AppResult};
use crate::ops::{apply_ops, ClientOp};
use crate::projects::{Project, ProjectAccess};
use crate::routes::{read_meta, Meta, WORDS_CACHE};
use crate::AppState;

/// Most videos one project's main track may hold.
pub const MAX_SOURCES: usize = 20;

/// Record `media_id` as uploaded into `project_id`, after every earlier row.
/// One statement, so two concurrent uploads cannot take the same position.
pub async fn register(
    db: &SqlitePool,
    project_id: &str,
    media_id: &str,
    start_at: f64,
    duration: f64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO project_sources (project_id, position, media_id, start_at, duration)
         SELECT ?, COALESCE(MAX(position), -1) + 1, ?, ?, ?
         FROM project_sources WHERE project_id = ?",
    )
    .bind(project_id)
    .bind(media_id)
    .bind(start_at)
    .bind(duration)
    .bind(project_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Whether `media_id` was ever uploaded into `project_id`. Takes any
/// executor so `validate` can ask inside its write transaction.
pub async fn in_registry<'e, E>(exec: E, project_id: &str, media_id: &str) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM project_sources WHERE project_id = ? AND media_id = ? LIMIT 1",
    )
    .bind(project_id)
    .bind(media_id)
    .fetch_optional(exec)
    .await?;
    Ok(row.is_some())
}

/// Every media id uploaded into the project, in upload order.
// Only the tests read the whole registry so far.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn registry(db: &SqlitePool, project_id: &str) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT media_id FROM project_sources WHERE project_id = ? ORDER BY position",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?)
}

/// Where a source's transcript is.
// `Running` and `Error` come from the background transcription (Task 4).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptStatus {
    Pending,
    Running,
    Ready,
    Error,
}

/// One source as the client sees it: the fold's placement plus the file's
/// name, kind and URL, and how far its transcript has got.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub index: usize,
    pub media_id: String,
    pub url: String,
    pub filename: String,
    pub kind: MediaKind,
    pub offset: f64,
    pub duration: f64,
    pub transcript: TranscriptStatus,
}

/// The project's own media as source 0.
pub async fn first_source(state: &AppState, project: &Project) -> AppResult<Source> {
    let meta = read_meta(&state.config.data_dir.join(&project.media_id)).await?;
    Ok(Source {
        media: project.media_id.clone(),
        offset: 0.0,
        duration: meta.duration,
    })
}

/// Every source on the main track, in stitched order, the first included.
pub async fn timeline(
    state: &AppState,
    project: &Project,
    doc: &ProjectDoc,
) -> AppResult<Vec<Source>> {
    Ok(all_sources(
        first_source(state, project).await?,
        &doc.sources,
    ))
}

/// A media item's transcript: ready once its words are cached.
pub fn transcript_status(state: &AppState, media_id: &str) -> TranscriptStatus {
    if state
        .config
        .data_dir
        .join(media_id)
        .join(WORDS_CACHE)
        .is_file()
    {
        TranscriptStatus::Ready
    } else {
        TranscriptStatus::Pending
    }
}

/// `sources` with each file's name, kind, URL and transcript status.
pub async fn views(state: &AppState, sources: &[Source]) -> AppResult<Vec<SourceView>> {
    let mut out = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let meta = read_meta(&state.config.data_dir.join(&source.media)).await?;
        out.push(SourceView {
            index,
            media_id: source.media.clone(),
            url: meta.url,
            filename: meta.filename,
            kind: meta.kind,
            offset: source.offset,
            duration: source.duration,
            transcript: transcript_status(state, &source.media),
        });
    }
    Ok(out)
}

/// Refuse a project whose main track already holds `count` sources, if that
/// is the most it may hold.
pub fn check_room(count: usize) -> AppResult<()> {
    if count >= MAX_SOURCES {
        Err(AppError::bad_request(format!(
            "a project holds at most {MAX_SOURCES} videos, and this one is full"
        )))
    } else {
        Ok(())
    }
}

/// Register `meta` with the project and append it to the end of the main
/// track as `user`. Validation in `apply_ops` checks the offset, the
/// registry, the duration and the cap again under the write lock, so a
/// concurrent add fails there rather than overlapping.
pub async fn add_source(
    state: &Arc<AppState>,
    project: &Project,
    user: &User,
    meta: &Meta,
) -> AppResult<SourceView> {
    let (_, doc) = crate::ops::load_doc(state, &project.id).await?;
    let sources = timeline(state, project, &doc).await?;
    check_room(sources.len())?;
    let offset = stitched_duration(&sources);
    register(&state.db, &project.id, &meta.id, offset, meta.duration).await?;
    apply_ops(
        state,
        project,
        user,
        vec![ClientOp {
            op_id: Uuid::new_v4().to_string(),
            op: Op::AddSource {
                media: meta.id.clone(),
                offset,
                duration: meta.duration,
            },
        }],
    )
    .await?;
    Ok(SourceView {
        index: sources.len(),
        media_id: meta.id.clone(),
        url: meta.url.clone(),
        filename: meta.filename.clone(),
        kind: meta.kind,
        offset,
        duration: meta.duration,
        transcript: transcript_status(state, &meta.id),
    })
}

/// What a layer's `media` names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerMedia {
    /// A project asset, with its kind and length.
    Asset { kind: MediaKind, duration: f64 },
    /// A media item in the project's registry: one of its own sources.
    Source,
}

/// The one rule for what a layer may show, shared by op validation
/// (`layer_media`) and the export's file lookup (`assets::asset_files`): a
/// project asset first, else any media in the project's registry. So a
/// stretch of video 2 can go over video 1, and a layer over an undone source
/// still renders, because the registry never shrinks. `None` when it is
/// neither.
pub async fn resolve_layer_media(
    conn: &mut SqliteConnection,
    project_id: &str,
    media: &str,
) -> AppResult<Option<LayerMedia>> {
    if let Some((kind, duration)) = crate::assets::find(&mut *conn, project_id, media).await? {
        return Ok(Some(LayerMedia::Asset { kind, duration }));
    }
    if in_registry(&mut *conn, project_id, media).await? {
        return Ok(Some(LayerMedia::Source));
    }
    Ok(None)
}

/// Kind and length of a media a layer may show (see `resolve_layer_media`),
/// read inside `validate`'s write transaction.
pub async fn layer_media(
    tx: &mut Transaction<'_, Sqlite>,
    state: &AppState,
    project: &Project,
    media: &str,
) -> AppResult<Option<(MediaKind, f64)>> {
    Ok(match resolve_layer_media(tx, &project.id, media).await? {
        Some(LayerMedia::Asset { kind, duration }) => Some((kind, duration)),
        Some(LayerMedia::Source) => {
            let meta = read_meta(&state.config.data_dir.join(media)).await?;
            Some((meta.kind, meta.duration))
        }
        None => None,
    })
}

/// A registry media's source file and metadata, for the export planner.
/// Callers check registry membership first; `media` is then one of ours.
pub async fn source_file(state: &AppState, media: &str) -> AppResult<(PathBuf, Meta)> {
    let dir = state.config.data_dir.join(media);
    let meta = read_meta(&dir).await?;
    Ok((dir.join(format!("source.{}", meta.ext)), meta))
}

/// `POST /api/projects/:id/sources` — multipart with one `file` field.
/// Stores the media, probes it, registers it and appends `AddSource` at the
/// current end of the main track. Editors and owners only. A full project is
/// refused before a byte of the body is read; an unreadable file is refused
/// with nothing registered, appended or left on disk.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    mut multipart: Multipart,
) -> AppResult<Json<SourceView>> {
    access.require_edit()?;
    let (_, doc) = crate::ops::load_doc(&state, &access.project.id).await?;
    check_room(1 + doc.sources.len())?;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
    {
        if field.name() == Some("file") {
            let meta = crate::routes::store_upload(&state, field).await?;
            return Ok(Json(
                add_source(&state, &access.project, &access.user, &meta).await?,
            ));
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use crate::projects::test_support::seed_media;
    use crate::routes::{read_meta, Meta};
    use crate::test_util::seed_words;
    use crate::AppState;

    /// A `duration`-second media item whose transcript cache already holds
    /// `words`, one a second from 0. Not yet registered with any project.
    pub async fn seed_source(state: &Arc<AppState>, duration: f64, words: &[&str]) -> Meta {
        let id = seed_media(state, duration).await;
        seed_words(state, &id, words).await;
        read_meta(&state.config.data_dir.join(&id)).await.unwrap()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use engine::MediaKind;
    use serde_json::{json, Value};

    use super::test_support::seed_source;
    use super::*;
    use crate::assets::test_support::seed_asset;
    use crate::projects::test_support::seed_media;
    use crate::test_util::{app, call, json_req, me, owned_project, register as sign_up, state};

    #[tokio::test]
    async fn a_new_project_registers_its_own_media_at_position_zero() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone()]
        );
        register(&state.db, &project.id, "m2", 10.0, 4.0)
            .await
            .unwrap();
        register(&state.db, &project.id, "m3", 14.0, 1.0)
            .await
            .unwrap();
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone(), "m2".to_owned(), "m3".to_owned()]
        );
        assert!(in_registry(&state.db, &project.id, "m2").await.unwrap());
        assert!(!in_registry(&state.db, &project.id, "elsewhere")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn adopting_orphans_skips_media_that_is_already_a_source() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let second = seed_media(&state, 4.0).await;
        register(&state.db, &project.id, &second, 10.0, 4.0)
            .await
            .unwrap();
        let owner = me(&state, &ada).await;
        crate::projects::adopt_orphans(&state, &owner.id)
            .await
            .unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(
            n, 1,
            "a source's media must not become a project of its own"
        );
    }

    async fn post_ops(
        state: &Arc<AppState>,
        cookie: &str,
        project: &str,
        ops: Vec<Value>,
    ) -> (StatusCode, Value) {
        let (status, body, _) = call(
            app(state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/ops"),
                Some(cookie),
                Some(json!({ "ops": ops })),
            ),
        )
        .await;
        (status, body)
    }

    fn add_op(id: &str, media: &str, offset: f64, duration: f64) -> Value {
        json!({ "opId": id, "kind": "addsource", "media": media, "offset": offset, "duration": duration })
    }

    #[tokio::test]
    async fn adding_a_source_appends_at_the_stitched_end_and_shows_in_the_project() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        let view = add_source(&state, &project, &owner, &second).await.unwrap();
        assert_eq!(view.index, 1);
        assert_eq!(view.offset, 10.0);
        assert_eq!(view.duration, 4.0);
        assert_eq!(view.transcript, TranscriptStatus::Ready);
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone(), second.id.clone()]
        );

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{}", project.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let sources = body["project"]["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["index"], 0);
        assert_eq!(sources[0]["mediaId"], project.media_id.as_str());
        assert_eq!(sources[0]["offset"], 0.0);
        assert_eq!(sources[0]["duration"], 10.0);
        assert_eq!(sources[0]["transcript"], "ready");
        assert_eq!(sources[1]["mediaId"], second.id.as_str());
        assert_eq!(sources[1]["url"], second.url.as_str());
        assert_eq!(sources[1]["filename"], "clip.mp4");
        assert_eq!(sources[1]["kind"], "video");
        assert_eq!(sources[1]["offset"], 10.0);
        assert_eq!(
            body["project"]["media"]["id"],
            project.media_id.as_str(),
            "media stays for one release"
        );
        // The doc carries the fold's appended sources only; project.sources lists all.
        assert_eq!(
            body["doc"]["sources"],
            json!([{ "media": second.id, "offset": 10.0, "duration": 4.0 }])
        );
        assert_eq!(body["doc"]["splits"], json!([10.0]), "the join is a split");

        // Past the end of the first file is now inside the project.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "c", "kind": "cut", "start": 12.0, "end": 13.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "c2", "kind": "cut", "start": 13.0, "end": 14.5 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "past the stitched end");
    }

    #[tokio::test]
    async fn add_source_ops_need_the_end_the_registry_and_the_real_duration() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let loose = seed_source(&state, 4.0, &["d"]).await;

        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("a", &loose.id, 10.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("not uploaded"),
            "{body}"
        );

        register(&state.db, &project.id, &loose.id, 10.0, 4.0)
            .await
            .unwrap();
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("b", &loose.id, 9.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("at the end"),
            "{body}"
        );
        let (status, _) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("c", &loose.id, 10.0, 5.0)],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a duration the media does not have"
        );

        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("d", &loose.id, 10.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["sources"].as_array().unwrap().len(),
            1,
            "appended sources only"
        );
    }

    #[tokio::test]
    async fn the_twenty_first_source_is_refused() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        for _ in 1..MAX_SOURCES {
            let m = seed_source(&state, 1.0, &[]).await;
            add_source(&state, &project, &owner, &m).await.unwrap();
        }
        let extra = seed_source(&state, 1.0, &[]).await;
        let err = add_source(&state, &project, &owner, &extra)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("20"), "{err}");
        // The log refuses it too, not just the helper.
        register(&state.db, &project.id, &extra.id, 29.0, 1.0)
            .await
            .unwrap();
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("x", &extra.id, 29.0, 1.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn undo_and_redo_of_sources_keep_the_main_track_contiguous() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let a = seed_source(&state, 4.0, &["d"]).await;
        let b = seed_source(&state, 2.0, &["e"]).await;
        let c = seed_source(&state, 3.0, &["f"]).await;
        add_source(&state, &project, &owner, &a).await.unwrap(); // seq 1
        add_source(&state, &project, &owner, &b).await.unwrap(); // seq 2

        let undo = |id: &str, seq: i64| json!({ "opId": id, "kind": "undo", "targetSeq": seq });
        let redo = |id: &str, seq: i64| json!({ "opId": id, "kind": "redo", "targetSeq": seq });
        let (status, body) = post_ops(&state, &ada, &project.id, vec![undo("u1", 1)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("added after"),
            "{body}"
        );

        let (status, body) = post_ops(&state, &ada, &project.id, vec![undo("u2", 2)]).await; // seq 3
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["sources"].as_array().unwrap().len(),
            1,
            "appended sources only"
        );

        // c takes the end b left; b cannot come back at its old place.
        add_source(&state, &project, &owner, &c).await.unwrap(); // seq 4, offset 14
        let (status, body) = post_ops(&state, &ada, &project.id, vec![redo("r1", 3)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("old place"),
            "{body}"
        );
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap().len(),
            4,
            "the registry never shrinks"
        );
    }

    #[tokio::test]
    async fn layers_check_track_media_length_sound_and_their_target() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let clip = seed_asset(&state, &project, MediaKind::Video, 3.0).await;
        let song = seed_asset(&state, &project, MediaKind::Audio, 30.0).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        let unrelated = seed_media(&state, 4.0).await;

        let layer = |id: &str, track: u8, media: &str, offset: f64, audio: Value| {
            json!({ "opId": id, "kind": "addlayer", "track": track, "start": 1.0, "end": 2.0,
                    "media": media, "offset": offset, "frame": "pipTopRight", "audio": audio })
        };
        let bad = [
            layer("b1", 1, &clip.id, 0.0, Value::Null), // the main track is not a layer
            layer("b2", 4, &clip.id, 0.0, Value::Null), // no V4
            layer("b3", 2, &song.id, 0.0, Value::Null), // audio is not a picture
            layer("b4", 2, "nope", 0.0, Value::Null),   // not ours
            layer("b5", 2, &unrelated, 0.0, Value::Null), // a media, but not this project's
            layer("b6", 2, &clip.id, 0.0, json!(20.0)), // too loud
            layer("b7", 3, &second.id, 3.5, Value::Null), // runs past the source's end
        ];
        for op in bad {
            let (status, body) = post_ops(&state, &ada, &project.id, vec![op.clone()]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{op}: {body}");
        }
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![
                layer("ok1", 2, &clip.id, 0.0, Value::Null),
                layer("ok2", 3, &second.id, 0.5, json!(-6.0)), // a stretch of video 2 over video 1
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let layers = |body: &Value| -> Vec<Value> {
            body["edits"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "layer")
                .cloned()
                .collect()
        };
        assert_eq!(layers(&body).len(), 2);

        let set = |id: &str, track: u8, start: f64, to: u8| {
            json!({ "opId": id, "kind": "setlayer", "track": track, "start": start,
                    "toTrack": to, "frame": "full", "audio": null })
        };
        for op in [
            set("s1", 3, 1.0, 2), // V2 already has a layer starting at 1.0
            set("s2", 3, 1.0, 1), // not a layer track
            set("s3", 2, 5.0, 2), // nothing starts there
        ] {
            let (status, body) = post_ops(&state, &ada, &project.id, vec![op.clone()]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{op}: {body}");
        }
        let (status, body) = post_ops(&state, &ada, &project.id, vec![set("s4", 3, 1.0, 3)]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let v3 = layers(&body).into_iter().find(|l| l["track"] == 3).unwrap();
        assert_eq!(v3["frame"], "full");
        assert_eq!(v3["audio"], Value::Null);

        let (status, _) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "r0", "kind": "removelayer", "track": 5, "start": 1.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "r1", "kind": "removelayer", "track": 3, "start": 1.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(layers(&body).len(), 1);
    }

    /// A one-field multipart upload of `bytes` as `filename` to `/sources`.
    fn upload_req(
        project: &str,
        cookie: &str,
        filename: &str,
        bytes: &[u8],
    ) -> axum::http::Request<axum::body::Body> {
        let mut body = format!(
            "--x\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n--x--\r\n");
        axum::http::Request::builder()
            .method(Method::POST)
            .uri(format!("/api/projects/{project}/sources"))
            .header(axum::http::header::COOKIE, cookie)
            .header(
                axum::http::header::CONTENT_TYPE,
                "multipart/form-data; boundary=x",
            )
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    async fn media_dirs(state: &Arc<AppState>) -> usize {
        let mut entries = tokio::fs::read_dir(&state.config.data_dir).await.unwrap();
        let mut n = 0;
        while let Some(e) = entries.next_entry().await.unwrap() {
            if e.path().is_dir() {
                n += 1;
            }
        }
        n
    }

    #[tokio::test]
    async fn sources_route_checks_role_type_and_readability() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let bob = sign_up(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        crate::test_util::add_member(&state, &ada, &project.id, "bob@example.com", "viewer").await;

        let (status, _, _) = call(app(&state), upload_req(&project.id, &bob, "a.mp4", b"x")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "a.txt", b"hi")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let before = media_dirs(&state).await;
        let (status, body, _) = call(
            app(&state),
            upload_req(&project.id, &ada, "a.mp4", b"not really video"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("could not read media"),
            "{body}"
        );
        // Nothing appended: no registry row, no op, no directory left behind.
        assert_eq!(registry(&state.db, &project.id).await.unwrap().len(), 1);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert!(doc.sources.is_empty());
        assert_eq!(media_dirs(&state).await, before);
    }

    #[tokio::test]
    async fn sources_route_refuses_a_full_project_before_reading_the_file() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        for _ in 1..MAX_SOURCES {
            let m = seed_source(&state, 1.0, &[]).await;
            add_source(&state, &project, &owner, &m).await.unwrap();
        }
        // Unreadable, so only the cap can explain a message naming 20.
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "a.mp4", b"junk")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("at most 20"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn sources_route_appends_a_real_file_at_the_end() {
        let (state, dir) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let wav = dir.path().join("tone.wav");
        let made = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=2")
            .args(["-ac", "1", "-ar", "16000"])
            .arg(&wav)
            .status()
            .await
            .expect("ffmpeg on PATH");
        assert!(made.success());
        let bytes = tokio::fs::read(&wav).await.unwrap();

        let (status, body, _) = call(
            app(&state),
            upload_req(&project.id, &ada, "tone.wav", &bytes),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["index"], 1);
        assert_eq!(body["offset"], 10.0);
        assert_eq!(body["kind"], "audio");
        assert_eq!(body["filename"], "tone.wav");
        assert!(
            (body["duration"].as_f64().unwrap() - 2.0).abs() < 0.05,
            "{body}"
        );
        let media = body["mediaId"].as_str().unwrap().to_owned();
        assert_eq!(body["url"], format!("/data/{media}/source.wav"));
        assert_eq!(registry(&state.db, &project.id).await.unwrap()[1], media);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert_eq!(doc.sources.len(), 1);
        assert_eq!(doc.sources[0].media, media);
    }

    #[tokio::test]
    async fn thumbnails_accept_any_registry_media_and_nothing_else() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        let stranger = seed_media(&state, 4.0).await;
        let thumbs = |media: &str| {
            json_req(
                Method::POST,
                &format!("/api/projects/{}/thumbnails", project.id),
                Some(&ada),
                Some(json!({ "media": media })),
            )
        };
        let (status, body, _) = call(app(&state), thumbs(&stranger)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("not one of this project's"),
            "{body}"
        );
        // A source gets past the check; the seeded file has no frames, so
        // ffmpeg fails and the route answers 502, not 400.
        let (status, body, _) = call(app(&state), thumbs(&second.id)).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    }
}
