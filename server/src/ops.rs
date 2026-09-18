//! The operation log behind every project: append, validate, fold, cache.
//! Both the REST route here and (later) the socket call `apply_ops`.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use engine::{fold, Edit, Op, ProjectDoc, SeqOp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Sqlite, Transaction};

use crate::auth::User;
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::projects::{summary, Project, ProjectAccess};
use crate::routes::read_meta;
use crate::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientOp {
    pub op_id: String,
    #[serde(flatten)]
    pub op: Op,
}

/// The folded project as the client sees it, plus what this user may undo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocState {
    pub head_seq: i64,
    pub edits: Vec<Edit>,
    pub speaker_names: Vec<String>,
    /// Seq of this user's most recent live operation, if any.
    pub undoable: Option<i64>,
    /// Seq of this user's most recent undo that has not been redone, if any.
    pub redoable: Option<i64>,
}

/// Every stored operation for a project, in order. Takes any executor so the
/// caller may run it inside a transaction and share a snapshot with another
/// read (see `load_doc`).
async fn read_log<'e, E>(exec: E, project_id: &str) -> AppResult<Vec<SeqOp>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let rows: Vec<(i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT seq, author_id, op, undone_by FROM edit_ops WHERE project_id = ? ORDER BY seq",
    )
    .bind(project_id)
    .fetch_all(exec)
    .await?;
    let mut ops = Vec::with_capacity(rows.len());
    for (seq, author_id, op, undone_by) in rows {
        let op: Op =
            serde_json::from_str(&op).map_err(|e| anyhow::anyhow!("corrupt op {seq}: {e}"))?;
        ops.push(SeqOp {
            seq,
            author_id,
            op,
            undone: undone_by.is_some(),
        });
    }
    Ok(ops)
}

/// The folded document and head seq, from the cache when it is current. The
/// head and the log are read inside one transaction so they share a
/// snapshot: two unsynchronised reads could otherwise see a head seq from
/// after a concurrent append but a log from before it, understating the fold.
pub async fn load_doc(state: &AppState, project_id: &str) -> AppResult<(i64, ProjectDoc)> {
    let mut tx = state.db.begin().await?;
    let (head,): (Option<i64>,) =
        sqlx::query_as("SELECT MAX(seq) FROM edit_ops WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
    let head = head.unwrap_or(0);
    if let Some((seq, doc)) = state
        .folds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(project_id)
    {
        if *seq == head {
            return Ok((head, doc.clone()));
        }
    }
    let doc = fold(&read_log(&mut *tx, project_id).await?);
    state
        .folds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(project_id.to_owned(), (head, doc.clone()));
    Ok((head, doc))
}

fn invalidate(state: &AppState, project_id: &str) {
    state
        .folds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(project_id);
}

/// What `user` may undo or redo next: their newest live op, and their newest
/// undo that has not itself been undone by a redo.
async fn undo_targets(
    db: &sqlx::SqlitePool,
    project_id: &str,
    user_id: &str,
) -> AppResult<(Option<i64>, Option<i64>)> {
    let undoable: Option<(i64,)> = sqlx::query_as(
        "SELECT seq FROM edit_ops WHERE project_id = ? AND author_id = ? AND undone_by IS NULL
         AND json_extract(op, '$.kind') NOT IN ('undo', 'redo') ORDER BY seq DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    let redoable: Option<(i64,)> = sqlx::query_as(
        "SELECT seq FROM edit_ops WHERE project_id = ? AND author_id = ? AND undone_by IS NULL
         AND json_extract(op, '$.kind') = 'undo' ORDER BY seq DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(db)
    .await?;
    Ok((undoable.map(|r| r.0), redoable.map(|r| r.0)))
}

pub async fn doc_state(state: &AppState, project: &Project, user: &User) -> AppResult<DocState> {
    let (head_seq, doc) = load_doc(state, &project.id).await?;
    let (undoable, redoable) = undo_targets(&state.db, &project.id, &user.id).await?;
    Ok(DocState {
        head_seq,
        edits: doc.edits,
        speaker_names: doc.speaker_names,
        undoable,
        redoable,
    })
}

/// Reject operations that cannot apply to this media: ranges outside the
/// duration, overdub audio that is not this media's, undo of someone else's.
/// `duration` is read once by the caller and shared across the whole batch.
async fn validate(
    tx: &mut Transaction<'_, Sqlite>,
    state: &AppState,
    project: &Project,
    user: &User,
    index: usize,
    op: &Op,
    duration: f64,
) -> AppResult<()> {
    let dir = state.config.data_dir.join(&project.media_id);
    let check_range = |start: f64, end: f64| -> AppResult<()> {
        if !(0.0..=duration).contains(&start) || !(0.0..=duration).contains(&end) || end < start {
            return Err(AppError::bad_request_at(
                index,
                format!("range {start}-{end} is outside the media"),
            ));
        }
        Ok(())
    };
    match op {
        Op::Cut { start, end } => check_range(*start, *end),
        Op::ApplyCuts { cuts } => cuts.iter().try_for_each(|r| check_range(r.start, r.end)),
        Op::Overdub {
            start,
            end,
            audio_url,
            ..
        } => {
            check_range(*start, *end)?;
            let prefix = format!("/data/{}/", project.media_id);
            let name = audio_url
                .strip_prefix(&prefix)
                .filter(|n| n.starts_with("overdub-") && n.ends_with(".wav") && !n.contains('/'));
            let ok = match name {
                Some(n) => tokio::fs::metadata(dir.join(n))
                    .await
                    .map(|m| m.is_file())
                    .unwrap_or(false),
                None => false,
            };
            if ok {
                Ok(())
            } else {
                Err(AppError::bad_request_at(
                    index,
                    "overdub audio does not belong to this media",
                ))
            }
        }
        Op::RenameSpeaker { name, .. } => {
            if name.chars().count() > 80 {
                return Err(AppError::bad_request_at(index, "speaker name is too long"));
            }
            Ok(())
        }
        Op::Undo { target_seq } | Op::Redo { target_seq } => {
            let want_undo_row = matches!(op, Op::Redo { .. });
            let row: Option<(String, Option<i64>, String)> = sqlx::query_as(
                "SELECT author_id, undone_by, json_extract(op, '$.kind') FROM edit_ops WHERE project_id = ? AND seq = ?",
            )
            .bind(&project.id)
            .bind(target_seq)
            .fetch_optional(&mut **tx)
            .await?;
            let Some((author, undone_by, kind)) = row else {
                return Err(AppError::bad_request_at(
                    index,
                    format!("no operation {target_seq}"),
                ));
            };
            if author != user.id {
                return Err(AppError::bad_request_at(
                    index,
                    "you can only undo your own changes",
                ));
            }
            if undone_by.is_some() {
                return Err(AppError::bad_request_at(
                    index,
                    "that change is already undone",
                ));
            }
            let is_undo_row = kind == "undo";
            if kind == "redo" || is_undo_row != want_undo_row {
                return Err(AppError::bad_request_at(
                    index,
                    "that operation cannot be targeted",
                ));
            }
            Ok(())
        }
    }
}

/// Append `ops` for `user`, in one transaction. Replays (same `op_id`) are
/// skipped so a client may resend after a lost response.
///
/// The transaction opens with `BEGIN IMMEDIATE` rather than sqlx's default
/// `BEGIN DEFERRED`: a deferred transaction only takes a write lock on its
/// first write, and this one's first statements are reads, so two concurrent
/// appends could both proceed past those reads and then race on the insert,
/// one of them failing with `SQLITE_BUSY_SNAPSHOT` (which `busy_timeout`
/// does not retry). Taking the write lock up front serialises appends
/// instead of letting one fail.
pub async fn apply_ops(
    state: &AppState,
    project: &Project,
    user: &User,
    ops: Vec<ClientOp>,
) -> AppResult<DocState> {
    let dir = state.config.data_dir.join(&project.media_id);
    let duration = read_meta(&dir).await?.duration;
    let mut tx = state.db.begin_with("BEGIN IMMEDIATE").await?;
    for (index, client_op) in ops.iter().enumerate() {
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT seq FROM edit_ops WHERE project_id = ? AND op_id = ?")
                .bind(&project.id)
                .bind(&client_op.op_id)
                .fetch_optional(&mut *tx)
                .await?;
        if exists.is_some() {
            continue;
        }
        validate(
            &mut tx,
            state,
            project,
            user,
            index,
            &client_op.op,
            duration,
        )
        .await?;
        let (head,): (Option<i64>,) =
            sqlx::query_as("SELECT MAX(seq) FROM edit_ops WHERE project_id = ?")
                .bind(&project.id)
                .fetch_one(&mut *tx)
                .await?;
        let seq = head.unwrap_or(0) + 1;
        sqlx::query(
            "INSERT INTO edit_ops (project_id, seq, op_id, author_id, op, undone_by, created_at) VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(&project.id)
        .bind(seq)
        .bind(&client_op.op_id)
        .bind(&user.id)
        .bind(serde_json::to_string(&client_op.op)?)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        match &client_op.op {
            Op::Undo { target_seq } => {
                sqlx::query("UPDATE edit_ops SET undone_by = ? WHERE project_id = ? AND seq = ?")
                    .bind(seq)
                    .bind(&project.id)
                    .bind(target_seq)
                    .execute(&mut *tx)
                    .await?;
            }
            Op::Redo { target_seq } => {
                // Retire the undo row and revive the op it had undone.
                sqlx::query("UPDATE edit_ops SET undone_by = ? WHERE project_id = ? AND seq = ?")
                    .bind(seq)
                    .bind(&project.id)
                    .bind(target_seq)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "UPDATE edit_ops SET undone_by = NULL WHERE project_id = ? AND undone_by = ?",
                )
                .bind(&project.id)
                .bind(target_seq)
                .execute(&mut *tx)
                .await?;
            }
            _ => {}
        }
    }
    tx.commit().await?;
    invalidate(state, &project.id);
    doc_state(state, project, user).await
}

#[derive(Deserialize)]
pub struct OpsRequest {
    ops: Vec<ClientOp>,
}

/// `POST /api/projects/:id/ops`.
pub async fn submit(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    Json(req): Json<OpsRequest>,
) -> AppResult<Json<DocState>> {
    access.require_edit()?;
    Ok(Json(
        apply_ops(&state, &access.project, &access.user, req.ops).await?,
    ))
}

/// `GET /api/projects/:id` — the project summary plus its folded document.
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Value>> {
    let project = summary(&state, &access.project, access.role).await?;
    let doc = doc_state(&state, &access.project, &access.user).await?;
    Ok(Json(json!({ "project": project, "doc": doc })))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::projects::create_project;
    use crate::projects::test_support::seed_media;
    use crate::test_util::{app, call, json_req, register, state};

    async fn me(state: &Arc<AppState>, cookie: &str) -> User {
        let (_, me, _) = call(
            app(state),
            json_req(Method::GET, "/api/me", Some(cookie), None),
        )
        .await;
        serde_json::from_value(me).unwrap()
    }

    /// A 10 s project owned by ada, with bob as `role` (or not a member).
    async fn setup(
        role: Option<&str>,
    ) -> (Arc<AppState>, tempfile::TempDir, String, String, String) {
        let (state, dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let media = seed_media(&state, 10.0).await;
        let owner = me(&state, &ada).await;
        let project = create_project(&state.db, &owner, &media, "Clip")
            .await
            .unwrap();
        if let Some(role) = role {
            call(
                app(&state),
                json_req(
                    Method::POST,
                    &format!("/api/projects/{}/members", project.id),
                    Some(&ada),
                    Some(json!({ "email": "bob@example.com", "role": role })),
                ),
            )
            .await;
        }
        (state, dir, ada, bob, project.id)
    }

    fn cut(op_id: &str, start: f64, end: f64) -> Value {
        json!({ "opId": op_id, "kind": "cut", "start": start, "end": end })
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

    #[tokio::test]
    async fn get_project_returns_summary_and_empty_doc() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{project}"),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["project"]["role"], "owner");
        assert_eq!(body["project"]["media"]["duration"], 10.0);
        assert_eq!(body["doc"]["headSeq"], 0);
        assert_eq!(body["doc"]["edits"], json!([]));
        assert_eq!(body["doc"]["undoable"], Value::Null);
    }

    #[tokio::test]
    async fn ops_append_fold_and_report_head() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![cut("a", 1.0, 2.0), cut("b", 3.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["headSeq"], 2);
        assert_eq!(body["edits"].as_array().unwrap().len(), 2);
        assert_eq!(body["undoable"], 2);
    }

    #[tokio::test]
    async fn replayed_op_id_is_idempotent() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0)]).await;
        let (status, body) = post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0)]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["headSeq"], 1);
        assert_eq!(body["edits"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn viewer_cannot_submit_ops_but_can_read() {
        let (state, _d, _ada, bob, project) = setup(Some("viewer")).await;
        let (status, _) = post_ops(&state, &bob, &project, vec![cut("a", 1.0, 2.0)]).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{project}"),
                Some(&bob),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn editor_can_submit_ops() {
        let (state, _d, _ada, bob, project) = setup(Some("editor")).await;
        let (status, _) = post_ops(&state, &bob, &project, vec![cut("a", 1.0, 2.0)]).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn out_of_range_op_is_400_with_index_and_nothing_is_stored() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![cut("a", 1.0, 2.0), cut("b", 5.0, 50.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["index"], 1);
        let (_, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{project}"),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(body["doc"]["headSeq"], 0);
    }

    #[tokio::test]
    async fn overdub_with_foreign_audio_is_400() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let od = json!({ "opId": "o", "kind": "overdub", "start": 1.0, "end": 2.0, "text": "hi",
                         "audioUrl": "/data/other/overdub-0.wav", "audioDuration": 1.0 });
        let (status, body) = post_ops(&state, &ada, &project, vec![od]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["index"], 0);
    }

    #[tokio::test]
    async fn undo_and_redo_only_touch_own_ops() {
        let (state, _d, ada, bob, project) = setup(Some("editor")).await;
        post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0)]).await; // seq 1, ada
        post_ops(&state, &bob, &project, vec![cut("b", 3.0, 4.0)]).await; // seq 2, bob

        // Bob may not undo Ada's op.
        let (status, body) = post_ops(
            &state,
            &bob,
            &project,
            vec![json!({ "opId": "u1", "kind": "undo", "targetSeq": 1 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        // Ada undoes her own; the fold drops it, bob's stays.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "u2", "kind": "undo", "targetSeq": 1 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["headSeq"], 3);
        assert_eq!(body["edits"].as_array().unwrap().len(), 1);
        assert_eq!(body["edits"][0]["start"], 3.0);
        assert_eq!(body["undoable"], Value::Null);
        assert_eq!(body["redoable"], 3);

        // Undoing an already-undone op is rejected.
        let (status, _) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "u3", "kind": "undo", "targetSeq": 1 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Redo brings it back.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "r1", "kind": "redo", "targetSeq": 3 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["edits"].as_array().unwrap().len(), 2);
        assert_eq!(body["undoable"], 1);
        assert_eq!(body["redoable"], Value::Null);
    }

    #[tokio::test]
    async fn rename_speaker_is_project_state() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (_, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "s", "kind": "renamespeaker", "speaker": 1, "name": "Ada" })],
        )
        .await;
        assert_eq!(body["speakerNames"], json!(["", "Ada"]));
    }

    #[tokio::test]
    async fn undoing_an_undo_row_is_rejected() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0)]).await; // seq 1
        post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "u1", "kind": "undo", "targetSeq": 1 })],
        )
        .await; // seq 2, an undo row
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "u2", "kind": "undo", "targetSeq": 2 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn redoing_a_plain_edit_is_rejected() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0)]).await; // seq 1, a cut
        let (status, body) = post_ops(
            &state,
            &ada,
            &project,
            vec![json!({ "opId": "r1", "kind": "redo", "targetSeq": 1 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn commenter_cannot_submit_ops() {
        let (state, _d, _ada, bob, project) = setup(Some("commenter")).await;
        let (status, _) = post_ops(&state, &bob, &project, vec![cut("a", 1.0, 2.0)]).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn empty_batch_is_a_no_op() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body) = post_ops(&state, &ada, &project, vec![]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["headSeq"], 0);
        assert_eq!(body["edits"], json!([]));
    }

    #[tokio::test]
    async fn media_routes_require_membership_and_upload_creates_a_project() {
        let (state, _d, ada, bob, project) = setup(None).await;
        // suggest needs a transcript; without one the route answers 404 for a
        // member, but 403 for a non-member — the extractor runs first.
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/suggest"),
                Some(&bob),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/suggest"),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // The old media routes are gone.
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, "/api/media/x/suggest", Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn export_uses_the_fold_and_viewers_may_not_export() {
        let (state, _d, ada, bob, project) = setup(Some("viewer")).await;
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/export"),
                Some(&bob),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Owner: the seeded media has no real source file, so ffmpeg planning
        // still succeeds (it only builds args) but the job errors later; we
        // assert the request itself is accepted with the planned duration
        // reflecting the fold's cut.
        post_ops(&state, &ada, &project, vec![cut("a", 0.0, 4.0)]).await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/export"),
                Some(&ada),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["planned"], 6.0);
    }

    #[tokio::test]
    async fn concurrent_appends_all_succeed_and_head_seq_matches_count() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let n = 8;
        let mut handles = Vec::new();
        for i in 0..n {
            let state = state.clone();
            let ada = ada.clone();
            let project = project.clone();
            handles.push(tokio::spawn(async move {
                post_ops(
                    &state,
                    &ada,
                    &project,
                    vec![cut(&format!("op{i}"), 0.0, 1.0)],
                )
                .await
            }));
        }
        for handle in handles {
            let (status, body) = handle.await.unwrap();
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let (_, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{project}"),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(body["doc"]["headSeq"], n as i64);
    }
}
