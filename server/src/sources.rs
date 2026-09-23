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

use anyhow::Context;
use axum::extract::{Multipart, State};
use axum::Json;
use engine::{
    all_sources, stitched_duration, Edit, MediaKind, Op, ProjectDoc, Source, SpeakerTurn, Word, EPS,
};
use serde::Serialize;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};
use uuid::Uuid;

use crate::auth::User;
use crate::error::{AppError, AppResult};
use crate::ops::{apply_ops, ClientOp};
use crate::projects::{Project, ProjectAccess};
use crate::routes::{
    read_meta, speakers_item, suggest_for, Meta, Speakers, Suggestions, WORDS_CACHE,
};
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

/// Every media id uploaded into the project, in upload order. Only the
/// tests read the whole list; the server asks `in_registry` about one.
#[cfg(test)]
pub async fn registry(db: &SqlitePool, project_id: &str) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT media_id FROM project_sources WHERE project_id = ? ORDER BY position",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?)
}

/// Where a source's transcript is.
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

/// A background transcription the server started and has not cached yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptJob {
    Running,
    /// Kept until the server restarts, so a client polling `/transcribe`
    /// cannot set off an endless retry. The cause is in the log.
    Failed,
}

/// A media item's transcript: ready once its words are cached, otherwise
/// whatever its background job says, and pending when there is none.
pub fn transcript_status(state: &AppState, media_id: &str) -> TranscriptStatus {
    if state
        .config
        .data_dir
        .join(media_id)
        .join(WORDS_CACHE)
        .is_file()
    {
        return TranscriptStatus::Ready;
    }
    match state
        .transcripts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(media_id)
    {
        Some(TranscriptJob::Running) => TranscriptStatus::Running,
        Some(TranscriptJob::Failed) => TranscriptStatus::Error,
        None => TranscriptStatus::Pending,
    }
}

/// Transcribe `media_id` in the background unless it is cached, running or
/// has failed. The words land in the per-media cache, exactly as a
/// foreground `transcribe_item` would leave them.
pub fn start_transcription(state: &Arc<AppState>, media_id: &str) {
    if transcript_status(state, media_id) != TranscriptStatus::Pending {
        return;
    }
    {
        let mut jobs = state.transcripts.lock().unwrap_or_else(|e| e.into_inner());
        // Checked again under the lock: two callers may both have seen pending.
        if jobs.contains_key(media_id) {
            return;
        }
        jobs.insert(media_id.to_owned(), TranscriptJob::Running);
    }
    let state = state.clone();
    let id = media_id.to_owned();
    tokio::spawn(async move {
        let result = crate::routes::transcribe_item(&state, &id).await;
        let mut jobs = state.transcripts.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            // The cache file now answers `ready`.
            Ok(_) => {
                jobs.remove(&id);
            }
            Err(e) => {
                tracing::warn!(id, "background transcription failed: {e:?}");
                jobs.insert(id, TranscriptJob::Failed);
            }
        }
    });
}

/// A media item's cached words, or none while it is still being transcribed.
async fn cached_words(state: &AppState, media_id: &str) -> AppResult<Vec<Word>> {
    match tokio::fs::read_to_string(state.config.data_dir.join(media_id).join(WORDS_CACHE)).await {
        Ok(json) => Ok(serde_json::from_str(&json).context("parsing cached words")?),
        Err(_) => Ok(Vec::new()),
    }
}

/// The project's words: each ready source's words shifted by its offset, in
/// source order. A source still transcribing contributes nothing yet.
pub async fn stitched_words(state: &AppState, sources: &[Source]) -> AppResult<Vec<Word>> {
    let mut per_source = Vec::with_capacity(sources.len());
    for source in sources {
        per_source.push(cached_words(state, &source.media).await?);
    }
    let parts: Vec<(&Source, &[Word])> = sources
        .iter()
        .zip(&per_source)
        .map(|(s, w)| (s, w.as_slice()))
        .collect();
    Ok(engine::stitch_words(&parts))
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

/// One source's share of the stitched speaker list: where it sits, how many
/// words it has, and its diarization if there is one to use.
pub struct SpeakerPart<'a> {
    pub offset: f64,
    pub words: usize,
    pub speakers: Option<&'a Speakers>,
}

/// Concatenate per-source speaker labels, namespaced so speaker 0 of one
/// file and speaker 0 of the next are different people. The labels and the
/// count come from `engine::stitch_speakers`, the one implementation of the
/// base rule. This adds only what the engine does not know about: each
/// source's labels padded or cut to its word count, and its turns shifted
/// into stitched time by the same base as its labels. A part without
/// diarization labels its words `None` and adds no speakers.
pub fn stitch_speaker_labels(parts: &[SpeakerPart]) -> Speakers {
    // A cache out of step with its words is padded or cut to fit, so the
    // list stays parallel to the stitched words.
    let labels: Vec<(u32, Vec<Option<u32>>)> = parts
        .iter()
        .map(|part| match part.speakers {
            Some(s) => (
                s.count,
                (0..part.words)
                    .map(|i| s.words.get(i).copied().flatten())
                    .collect(),
            ),
            None => (0, vec![None; part.words]),
        })
        .collect();
    let slices: Vec<(u32, &[Option<u32>])> = labels
        .iter()
        .map(|(count, words)| (*count, words.as_slice()))
        .collect();
    let (count, words) = engine::stitch_speakers(&slices);
    let mut turns = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let Some(s) = part.speakers else {
            continue;
        };
        // The base the engine gave this part's labels: the stitched count of
        // the parts before it.
        let (base, _) = engine::stitch_speakers(&slices[..k]);
        turns.extend(s.turns.iter().map(|t| SpeakerTurn {
            start: t.start + part.offset,
            end: t.end + part.offset,
            speaker: t.speaker + base,
        }));
    }
    Speakers {
        count,
        words,
        turns,
    }
}

/// Speaker labels for the project's stitched words.
///
/// Labels stop at the first source that is not transcribed yet: only the
/// ready prefix is passed with its diarization, and every source after it
/// is passed with no labels (`None` for each word, no speakers) until it is. Otherwise that source's count,
/// once known, would shift every later base and move a name someone typed
/// onto a different voice. A source whose diarization fails counts as no
/// speakers. Fails only when no source could be diarized at all, with the
/// same error the single-file route gave.
pub async fn stitched_speakers(state: &AppState, sources: &[Source]) -> AppResult<Speakers> {
    let mut counts = Vec::with_capacity(sources.len());
    let mut diarized = Vec::with_capacity(sources.len());
    let mut labelled = true;
    let mut any = false;
    let mut first_error = None;
    for source in sources {
        counts.push(cached_words(state, &source.media).await?.len());
        labelled &= transcript_status(state, &source.media) == TranscriptStatus::Ready;
        let speakers = if labelled {
            match speakers_item(state, &source.media).await {
                Ok(s) => {
                    any = true;
                    Some(s)
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                    None
                }
            }
        } else {
            None
        };
        diarized.push(speakers);
    }
    if !any {
        return Err(
            first_error.unwrap_or_else(|| AppError::not_found("transcribe this media first"))
        );
    }
    let parts: Vec<SpeakerPart> = sources
        .iter()
        .zip(&counts)
        .zip(&diarized)
        .map(|((s, n), sp)| SpeakerPart {
            offset: s.offset,
            words: *n,
            speakers: sp.as_ref(),
        })
        .collect();
    Ok(stitch_speaker_labels(&parts))
}

/// A suggested cut moved from a source's own time into stitched time.
fn shifted(edit: Edit, by: f64) -> Edit {
    match edit {
        Edit::Cut {
            start,
            end,
            transition,
        } => Edit::Cut {
            start: start + by,
            end: end + by,
            transition,
        },
        // The suggesters only ever produce cuts.
        other => other,
    }
}

/// Filler and pause cuts for every ready source, each computed on its own
/// words and silences and shifted by its offset. A cut never crosses a join:
/// each is bounded by its own file's duration. 404 while nothing is ready.
pub async fn suggest_project(
    state: &AppState,
    sources: &[Source],
    two_word: bool,
) -> AppResult<Suggestions> {
    let mut fillers = Vec::new();
    let mut pauses = Vec::new();
    let mut any = false;
    for source in sources {
        if transcript_status(state, &source.media) != TranscriptStatus::Ready {
            continue;
        }
        let part = suggest_for(state, &source.media, two_word).await?;
        any = true;
        fillers.extend(part.fillers.into_iter().map(|e| shifted(e, source.offset)));
        pauses.extend(part.pauses.into_iter().map(|e| shifted(e, source.offset)));
    }
    if !any {
        return Err(AppError::not_found("transcribe this media first"));
    }
    Ok(Suggestions { fillers, pauses })
}

/// Everything an agent reads at open: the project's sources, its stitched
/// words and its stitched speaker labels (`None` when diarization is not
/// available). The first source is transcribed first, as it always was; the
/// others are started in the background.
pub async fn project_transcript(
    state: &Arc<AppState>,
    project: &Project,
) -> AppResult<(Vec<Source>, Vec<Word>, Option<Speakers>)> {
    crate::routes::transcribe_item(state, &project.media_id).await?;
    let (_, doc) = crate::ops::load_doc(state, &project.id).await?;
    let sources = timeline(state, project, &doc).await?;
    for source in sources.iter().skip(1) {
        start_transcription(state, &source.media);
    }
    let words = stitched_words(state, &sources).await?;
    let speakers = stitched_speakers(state, &sources).await.ok();
    Ok((sources, words, speakers))
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
    register(
        &state.db,
        &project.id,
        &meta.id,
        stitched_duration(&sources),
        meta.duration,
    )
    .await?;
    append_at_end(state, project, user, meta, sources).await
}

/// Append `meta` at the end of `sources`, the timeline as last read. The
/// offset is checked again under the write lock, so a concurrent add that
/// moved the end first makes this one fail; it then reads the timeline again
/// and retries once at the new end, rather than failing late for a reason
/// the caller could not help.
async fn append_at_end(
    state: &Arc<AppState>,
    project: &Project,
    user: &User,
    meta: &Meta,
    mut sources: Vec<Source>,
) -> AppResult<SourceView> {
    let mut retried = false;
    // The same media may already be on the track (an asset added twice
    // shares one id), so "did this add land" is "is there one more of it".
    let copies = |sources: &[Source]| sources.iter().filter(|s| s.media == meta.id).count();
    let before = copies(&sources);
    let offset = loop {
        let offset = stitched_duration(&sources);
        let appended = apply_ops(
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
        .await;
        let Err(e) = appended else {
            break offset;
        };
        let (_, doc) = crate::ops::load_doc(state, &project.id).await?;
        let now = timeline(state, project, &doc).await?;
        // The failure may have come after the commit, or an earlier attempt
        // landed: the source is already live, so this is a success, and
        // appending again would add it twice (and a caller cleaning up
        // after an error would delete a live source's files).
        if copies(&now) > before {
            let index = now
                .iter()
                .rposition(|s| s.media == meta.id)
                .expect("counted above");
            let offset = now[index].offset;
            sources = now;
            sources.truncate(index);
            break offset;
        }
        if retried || (stitched_duration(&now) - offset).abs() <= EPS {
            // Tried twice, or the end did not move: the refusal is about
            // this media.
            return Err(e);
        }
        check_room(now.len())?;
        sources = now;
        retried = true;
    };
    start_transcription(state, &meta.id);
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

/// Undo `register` for media whose `AddSource` never landed. Only for media
/// this request uploaded: nothing else can refer to it yet.
async fn unregister(db: &SqlitePool, project_id: &str, media_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM project_sources WHERE project_id = ? AND media_id = ?")
        .bind(project_id)
        .bind(media_id)
        .execute(db)
        .await?;
    Ok(())
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
/// `index` is the op's place in its batch, for the error when a registry
/// media's files are gone.
pub async fn layer_media(
    tx: &mut Transaction<'_, Sqlite>,
    state: &AppState,
    project: &Project,
    index: usize,
    media: &str,
) -> AppResult<Option<(MediaKind, f64)>> {
    Ok(match resolve_layer_media(tx, &project.id, media).await? {
        Some(LayerMedia::Asset { kind, duration }) => Some((kind, duration)),
        Some(LayerMedia::Source) => {
            let meta = read_meta(&state.config.data_dir.join(media))
                .await
                .map_err(|_| AppError::bad_request_at(index, "that media is missing"))?;
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

/// The media an agent means by `media`: a media item already in the
/// project's registry, or a project asset promoted to a media item of its
/// own. A source needs its own directory (transcript cache, `meta.json`,
/// `/data` URL), and assets live inside the first media's, so an asset is
/// hard-linked (copied across filesystems) into one. Its id is derived from
/// the asset's, so adding the same asset twice shares one transcript.
pub async fn source_media(state: &AppState, project: &Project, media: &str) -> AppResult<Meta> {
    if in_registry(&state.db, &project.id, media).await? {
        return read_meta(&state.config.data_dir.join(media)).await;
    }
    let asset = crate::assets::list_for(&state.db, project)
        .await?
        .into_iter()
        .find(|a| a.id == media)
        .ok_or_else(|| {
            AppError::bad_request(format!(
                "{media} is neither an asset from list_assets nor one of this project's videos"
            ))
        })?;
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("type-n-stitch:asset:{}", asset.id).as_bytes(),
    )
    .to_string();
    let dir = state.config.data_dir.join(&id);
    if dir.join("meta.json").is_file() {
        return read_meta(&dir).await;
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating media dir")?;
    let source_name = format!("source.{}", asset.ext);
    let from =
        crate::assets::assets_dir(state, project).join(format!("{}.{}", asset.id, asset.ext));
    // Staged under a name of its own and renamed into place, so two agents
    // promoting the same asset at once never copy onto a link to the asset
    // itself, which would truncate it.
    let partial = dir.join(format!("{source_name}.tmp-{}", Uuid::new_v4()));
    if tokio::fs::hard_link(&from, &partial).await.is_err() {
        tokio::fs::copy(&from, &partial)
            .await
            .context("copying the asset")?;
    }
    tokio::fs::rename(&partial, dir.join(&source_name))
        .await
        .context("placing the asset")?;
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename: asset.name,
        ext: asset.ext,
        duration: asset.duration,
        kind: asset.kind,
        // The asset row keeps no frame rate; the export re-probes, as it
        // does for any media stored before dimensions were recorded.
        video: None,
    };
    crate::routes::write_json_atomic(&dir.join("meta.json"), &serde_json::to_vec_pretty(&meta)?)
        .await?;
    Ok(meta)
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
        .map_err(|e| AppError::multipart(&e, ""))?
    {
        if field.name() == Some("file") {
            let meta = crate::routes::store_upload(&state, field).await?;
            return match add_source(&state, &access.project, &access.user, &meta).await {
                Ok(view) => Ok(Json(view)),
                Err(e) => {
                    // Refused after the file landed: leave nothing behind.
                    if let Err(cleanup) = unregister(&state.db, &access.project.id, &meta.id).await
                    {
                        tracing::warn!(id = meta.id, "could not unregister: {cleanup:?}");
                    }
                    let _ = tokio::fs::remove_dir_all(state.config.data_dir.join(&meta.id)).await;
                    Err(e)
                }
            };
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
    use crate::test_util::seed_words;
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
    async fn undoing_a_source_is_refused_while_anyone_has_edited_after_it() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let bob = sign_up(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        crate::test_util::add_member(&state, &ada, &project.id, "bob@example.com", "editor").await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap(); // seq 1, 10..14

        // Bob puts a title inside Ada's new video.
        let title = json!({ "opId": "t", "kind": "addtitle", "at": 12.0, "duration": 2.0,
                            "text": "Hi", "subtitle": null, "style": "dark" });
        let (status, body) = post_ops(&state, &bob, &project.id, vec![title]).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let undo = json!({ "opId": "u", "kind": "undo", "targetSeq": 1 });
        let (status, body) = post_ops(&state, &ada, &project.id, vec![undo]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("changed since"),
            "{body}"
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
        multipart_req(
            &format!("/api/projects/{project}/sources"),
            cookie,
            "file",
            filename,
            bytes,
        )
    }

    /// A one-field multipart POST of `bytes` as `filename` in field `field`.
    fn multipart_req(
        uri: &str,
        cookie: &str,
        field: &str,
        filename: &str,
        bytes: &[u8],
    ) -> axum::http::Request<axum::body::Body> {
        let mut body = format!(
            "--x\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n--x--\r\n");
        axum::http::Request::builder()
            .method(Method::POST)
            .uri(uri)
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

    async fn transcribe(state: &Arc<AppState>, cookie: &str, project: &str) -> Value {
        let (status, body, _) = call(
            app(state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/transcribe"),
                Some(cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn ids(body: &Value) -> Vec<String> {
        body["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap().to_owned())
            .collect()
    }

    fn statuses(body: &Value) -> Vec<String> {
        body["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["transcript"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn transcribe_stitches_ready_sources_and_reports_each_status() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        // A third whose transcription is already under way.
        let third = seed_media(&state, 2.0).await;
        let third = read_meta(&state.config.data_dir.join(&third))
            .await
            .unwrap();
        state
            .transcripts
            .lock()
            .unwrap()
            .insert(third.id.clone(), TranscriptJob::Running);
        add_source(&state, &project, &owner, &third).await.unwrap();

        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(ids(&body), ["w0", "w1", "w2", "1:w0", "1:w1"]);
        assert_eq!(body["words"][3]["start"], 10.0);
        assert_eq!(body["words"][4]["end"], 11.5);
        assert_eq!(statuses(&body), ["ready", "ready", "running"]);
        assert_eq!(body["sources"][2]["offset"], 14.0);

        // It finishes: the words are cached and the job is cleared.
        seed_words(&state, &third.id, &["f"]).await;
        state.transcripts.lock().unwrap().remove(&third.id);
        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(ids(&body).last().unwrap(), "2:w0");
        assert_eq!(body["words"][5]["start"], 14.0);
        assert_eq!(statuses(&body), ["ready", "ready", "ready"]);
    }

    #[tokio::test]
    async fn a_failed_transcription_reports_error_and_polling_does_not_retry_it() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        // No words and no source file: the job's ffmpeg step fails.
        let broken = seed_media(&state, 4.0).await;
        let broken = read_meta(&state.config.data_dir.join(&broken))
            .await
            .unwrap();
        let view = add_source(&state, &project, &owner, &broken).await.unwrap();
        assert_eq!(view.transcript, TranscriptStatus::Running);
        for _ in 0..200 {
            if transcript_status(&state, &broken.id) != TranscriptStatus::Running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(
            transcript_status(&state, &broken.id),
            TranscriptStatus::Error
        );

        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(statuses(&body), ["ready", "error"]);
        assert_eq!(
            ids(&body).len(),
            3,
            "the first source's words still come back"
        );
        assert_eq!(
            transcript_status(&state, &broken.id),
            TranscriptStatus::Error,
            "polling must not start it again"
        );
    }

    /// Pre-seed `media`'s diarization cache.
    async fn seed_speakers(state: &Arc<AppState>, media: &str, speakers: &Speakers) {
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(media)
                .join(crate::routes::SPEAKERS_CACHE),
            serde_json::to_vec(speakers).unwrap(),
        )
        .await
        .unwrap();
    }

    fn turn(start: f64, end: f64, speaker: u32) -> engine::SpeakerTurn {
        engine::SpeakerTurn {
            start,
            end,
            speaker,
        }
    }

    #[test]
    fn speaker_labels_are_namespaced_by_the_counts_before_them() {
        let a = Speakers {
            count: 2,
            words: vec![Some(0), Some(1), None],
            turns: vec![turn(0.0, 1.0, 0), turn(1.0, 2.0, 1)],
        };
        let b = Speakers {
            count: 1,
            words: vec![Some(0)],
            turns: vec![turn(0.0, 1.0, 0)],
        };
        let out = stitch_speaker_labels(&[
            SpeakerPart {
                offset: 0.0,
                words: 3,
                speakers: Some(&a),
            },
            SpeakerPart {
                offset: 10.0,
                words: 2,
                speakers: None,
            },
            SpeakerPart {
                offset: 14.0,
                words: 1,
                speakers: Some(&b),
            },
        ]);
        assert_eq!(out.count, 3);
        assert_eq!(out.words, [Some(0), Some(1), None, None, None, Some(2)]);
        assert_eq!(out.turns.last(), Some(&turn(14.0, 15.0, 2)));

        // The base rule is the engine's: a count below the labels a source
        // uses never lets two sources share a speaker, in labels or turns.
        let low = Speakers {
            count: 1,
            words: vec![Some(2)],
            turns: vec![turn(0.0, 1.0, 2)],
        };
        let out = stitch_speaker_labels(&[
            SpeakerPart {
                offset: 0.0,
                words: 1,
                speakers: Some(&low),
            },
            SpeakerPart {
                offset: 5.0,
                words: 1,
                speakers: Some(&b),
            },
        ]);
        assert_eq!(out.words, [Some(2), Some(3)]);
        assert_eq!(out.turns.last(), Some(&turn(5.0, 6.0, 3)));
        assert_eq!(out.count, 4);
    }

    #[tokio::test]
    async fn speakers_read_through_every_source_up_to_the_first_untranscribed_one() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        seed_speakers(
            &state,
            &project.media_id,
            &Speakers {
                count: 2,
                words: vec![Some(0), Some(1), Some(0)],
                turns: vec![turn(0.0, 0.9, 0), turn(0.9, 1.9, 1), turn(1.9, 3.0, 0)],
            },
        )
        .await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        seed_speakers(
            &state,
            &second.id,
            &Speakers {
                count: 1,
                words: vec![Some(0), Some(0)],
                turns: vec![turn(0.0, 2.0, 0)],
            },
        )
        .await;
        add_source(&state, &project, &owner, &second).await.unwrap();

        let speakers = |state: Arc<AppState>| {
            let (ada, id) = (ada.clone(), project.id.clone());
            async move {
                let (status, body, _) = call(
                    app(&state),
                    json_req(
                        Method::POST,
                        &format!("/api/projects/{id}/speakers"),
                        Some(&ada),
                        None,
                    ),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                body
            }
        };
        let body = speakers(state.clone()).await;
        assert_eq!(body["count"], 3);
        assert_eq!(body["words"], json!([0, 1, 0, 2, 2]));
        assert_eq!(
            body["turns"][3],
            json!({ "start": 10.0, "end": 12.0, "speaker": 2 })
        );

        // A third source still transcribing, then a fourth that is ready:
        // the fourth gets no labels until the third is done, so its base
        // cannot move under a name.
        let third = seed_media(&state, 2.0).await;
        let third = read_meta(&state.config.data_dir.join(&third))
            .await
            .unwrap();
        state
            .transcripts
            .lock()
            .unwrap()
            .insert(third.id.clone(), TranscriptJob::Running);
        add_source(&state, &project, &owner, &third).await.unwrap();
        let fourth = seed_source(&state, 3.0, &["g"]).await;
        seed_speakers(
            &state,
            &fourth.id,
            &Speakers {
                count: 1,
                words: vec![Some(0)],
                turns: vec![turn(0.0, 1.0, 0)],
            },
        )
        .await;
        add_source(&state, &project, &owner, &fourth).await.unwrap();
        let body = speakers(state.clone()).await;
        assert_eq!(body["words"], json!([0, 1, 0, 2, 2, null]));
    }

    #[tokio::test]
    async fn suggestions_cover_every_ready_source_in_stitched_time() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["um", "hello"]).await;
        // A silence in the second file, cached so nothing shells out.
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(&second.id)
                .join("silences-v1.json"),
            serde_json::to_vec(&[engine::Range::new(1.5, 3.2)]).unwrap(),
        )
        .await
        .unwrap();
        add_source(&state, &project, &owner, &second).await.unwrap();

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{}/suggest", project.id),
                Some(&ada),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // "um" is word 0 of the second file: [0, 1) there, [10, 11) here.
        assert_eq!(
            body["fillers"],
            json!([{ "kind": "cut", "start": 10.0, "end": 11.0 }])
        );
        // The pauses are the second file's own, shifted by its offset.
        let local = crate::routes::suggest_for(&state, &second.id, false)
            .await
            .unwrap();
        assert!(!local.pauses.is_empty());
        let shifted: Vec<Value> = local
            .pauses
            .iter()
            .map(|e| {
                let r = e.range();
                json!({ "kind": "cut", "start": r.start + 10.0, "end": r.end + 10.0 })
            })
            .collect();
        assert_eq!(body["pauses"], json!(shifted));
    }

    /// A two-second 440 Hz tone as WAV bytes, made by ffmpeg.
    async fn tone(dir: &std::path::Path) -> Vec<u8> {
        let wav = dir.join("tone.wav");
        let made = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=2")
            .args(["-ac", "1", "-ar", "16000"])
            .arg(&wav)
            .status()
            .await
            .expect("ffmpeg on PATH");
        assert!(made.success());
        tokio::fs::read(&wav).await.unwrap()
    }

    #[tokio::test]
    async fn an_oversized_upload_is_413_on_both_upload_routes() {
        let (mut state, _d) = state().await;
        Arc::get_mut(&mut state).unwrap().config.max_upload_bytes = 1024;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let before = media_dirs(&state).await;
        let big = vec![0u8; 8 * 1024];
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "a.mp4", &big)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        let (status, body, _) = call(
            app(&state),
            multipart_req("/api/projects", &ada, "file", "a.mp4", &big),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
        assert_eq!(media_dirs(&state).await, before, "nothing left behind");
    }

    #[tokio::test]
    async fn a_source_racing_another_add_retries_at_the_new_end() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        // The timeline as the loser read it, before the winner landed.
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        let stale = timeline(&state, &project, &doc).await.unwrap();
        let winner = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &winner).await.unwrap();

        let loser = seed_source(&state, 2.0, &["e"]).await;
        register(&state.db, &project.id, &loser.id, 10.0, 2.0)
            .await
            .unwrap();
        let view = append_at_end(&state, &project, &owner, &loser, stale)
            .await
            .unwrap();
        assert_eq!(view.index, 2);
        assert_eq!(view.offset, 14.0);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert_eq!(doc.sources.len(), 2);
        assert_eq!(doc.sources[1].offset, 14.0);
    }

    #[tokio::test]
    async fn a_source_is_refused_when_the_splits_are_full_and_leaves_nothing_behind() {
        let (state, dir) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let splits: Vec<Value> = (1..=engine::MAX_SPLITS)
            .map(|i| json!({ "opId": format!("s{i}"), "kind": "split", "at": i as f64 * 0.1 }))
            .collect();
        let (status, body) = post_ops(&state, &ada, &project.id, splits).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // The op itself is refused with a message naming the cause.
        let loose = seed_source(&state, 4.0, &["d"]).await;
        register(&state.db, &project.id, &loose.id, 10.0, 4.0)
            .await
            .unwrap();
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("a", &loose.id, 10.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("splits"), "{body}");

        // Through the route, a real file that fails late is cleaned up:
        // no media directory and no registry row for it.
        let rows = registry(&state.db, &project.id).await.unwrap().len();
        let before = media_dirs(&state).await;
        let bytes = tone(dir.path()).await;
        let (status, body, _) = call(
            app(&state),
            upload_req(&project.id, &ada, "tone.wav", &bytes),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("splits"), "{body}");
        assert_eq!(media_dirs(&state).await, before);
        assert_eq!(registry(&state.db, &project.id).await.unwrap().len(), rows);
    }

    #[tokio::test]
    async fn an_undo_or_redo_in_a_batch_refolds_before_the_next_op() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap(); // seq 1
        let cut = json!({ "opId": "c1", "kind": "cut", "start": 12.0, "end": 13.0 });

        // Once the source is undone, 12-13 s is past the end.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![
                json!({ "opId": "u1", "kind": "undo", "targetSeq": 1 }),
                cut.clone(),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["index"], 1, "{body}");

        // Undo alone (seq 2), then a redo brings the range back in the same batch.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "u2", "kind": "undo", "targetSeq": 1 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "r1", "kind": "redo", "targetSeq": 2 }), cut],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn sources_route_refuses_a_commenter_and_a_body_without_a_file() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let cara = sign_up(&state, "cara@example.com").await;
        let project = owned_project(&state, &ada).await;
        crate::test_util::add_member(&state, &ada, &project.id, "cara@example.com", "commenter")
            .await;
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &cara, "a.mp4", b"x")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body, _) = call(
            app(&state),
            multipart_req(
                &format!("/api/projects/{}/sources", project.id),
                &ada,
                "notes",
                "a.mp4",
                b"x",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("missing `file`"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn registry_media_whose_files_are_gone_is_a_400_for_sources_and_layers() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let gone = Uuid::new_v4().to_string();
        register(&state.db, &project.id, &gone, 10.0, 4.0)
            .await
            .unwrap();
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![add_op("a", &gone, 10.0, 4.0)],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("that media is missing"),
            "{body}"
        );

        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![
                json!({ "opId": "l", "kind": "addlayer", "track": 2, "start": 1.0, "end": 2.0,
                         "media": gone, "offset": 0.0, "frame": "full", "audio": null }),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["index"], 0, "{body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("that media is missing"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn tests_never_find_a_real_whisper_or_diarizer() {
        let (state, _d) = state().await;
        assert!(!std::path::Path::new(&state.config.whisper_bin).is_file());
        assert!(
            !state.config.diarize_bin.is_file(),
            "{}",
            state.config.diarize_bin.display()
        );
    }

    #[tokio::test]
    async fn a_retry_after_the_add_already_landed_does_not_append_it_twice() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        let stale = timeline(&state, &project, &doc).await.unwrap();
        // The add committed, but its caller saw an error and tries again.
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        let view = append_at_end(&state, &project, &owner, &second, stale)
            .await
            .unwrap();
        assert_eq!(view.index, 1);
        assert_eq!(view.offset, 10.0);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert_eq!(doc.sources.len(), 1, "appended once");
    }
}
