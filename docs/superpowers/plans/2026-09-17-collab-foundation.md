# Collaboration Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give type-n-stitch accounts, sessions, shared projects, and a server-owned edit list folded from an append-only operation log, so the client stops being the source of truth for what gets exported.

**Architecture:** SQLite (via `sqlx`) holds users, sessions, projects, membership and `edit_ops`; media files stay on disk under `DATA_DIR`. A new `engine::ops` module defines `Op` and a pure `fold` that turns a sequence of operations into the `Vec<Edit>` the existing engine already understands. Axum extractors (`CurrentUser`, `ProjectAccess`) authorize every route in its signature. The client keeps its optimistic reducer but sends every change as an operation and reconciles with the server's fold.

**Tech Stack:** Rust 1.98, axum 0.8, sqlx 0.9 (sqlite, runtime-tokio, migrate), argon2 0.5, uuid (v4, v5, v7); React 19, TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-09-17-collab-foundation-design.md` — this plan implements the **Foundation** project (section "Scope", item 1) plus the pieces of "Operations", "Authentication and authorization", "HTTP API", "Client changes", "Data flow for one edit", "Error handling" and "Testing" that do not need a socket. Realtime, comments, and workflow are later plans.

## Global Constraints

- Roles are exactly `owner`, `editor`, `commenter`, `viewer`; all four enforced now.
- Every operation carries a client-generated `op_id` (UUID v7); `unique (project_id, op_id)` makes replay idempotent.
- `Undo { target_seq }` and `Redo { target_seq }` may only target the submitting user's own operations.
- Unauthenticated → 401; wrong role → 403; missing project → 404. Never conflate.
- A malformed operation → 400 with the failing operation's index.
- Appending a batch of operations is one SQLite transaction.
- Media files, overdub WAVs and exports stay on disk; the database holds metadata and operations only.
- Session cookie: HttpOnly, SameSite=Lax, opaque token, server-side `sessions` row.
- Password hashing: argon2id.
- The engine remains the source of truth for export; `client/src/editlist.ts` stays a preview mirror.
- `npm test` (`cargo test && vitest run`) and `npm run lint` must pass after every task.
- Commit messages are plain prose in the style of the existing log (`git log --oneline -5`), ending with the attribution lines shown in the session's system reminder.

## Conventions used by every task

- Run cargo with `export PATH="$HOME/.cargo/bin:$PATH"` first; it is not on the default PATH.
- Rust tests: `cargo test -p engine` / `cargo test -p server`. Client tests: `npx vitest run`.
- Server unit tests live in `#[cfg(test)] mod tests` blocks inside the module they test and use the helpers in `server/src/test_util.rs` (Task 3).
- Timestamps are `i64` unix seconds from `crate::db::now()`.
- Ids are UUID strings.

## File structure

Created:

- `server/migrations/0001_foundation.sql` — schema.
- `server/src/db.rs` — pool open (WAL, foreign keys), migrations, `now()`.
- `server/src/app.rs` — `router(state)` so tests and `main` build the same app.
- `server/src/auth.rs` — password hashing, cookie parsing, `CurrentUser` extractor, auth handlers.
- `server/src/projects.rs` — `Role`, `Project`, `ProjectAccess` extractor, project and member handlers, orphan adoption.
- `server/src/ops.rs` — `apply_ops`, fold cache, validation, `POST /ops`, `GET /projects/:id`.
- `server/src/test_util.rs` — temp state, request/response helpers (`#[cfg(test)]`).
- `engine/src/ops.rs` — `Op`, `SeqOp`, `ProjectDoc`, `fold`.
- `client/src/ops.ts` — action → operation mapping.
- `client/src/session.ts` — `useSession` hook.
- `client/src/components/Login.tsx`, `client/src/components/Projects.tsx`.

Modified:

- `server/Cargo.toml`, `engine/src/lib.rs`, `server/src/main.rs`, `server/src/config.rs`, `server/src/error.rs`, `server/src/routes.rs`, `server/src/library.rs`.
- `client/src/types.ts`, `client/src/api.ts`, `client/src/editor.ts`, `client/src/editor.test.ts`, `client/src/App.tsx`, `client/src/components/Toolbar.tsx`, `client/src/styles.css`.
- `README.md`, `.gitignore`.

---

### Task 1: Database module and schema

**Files:**
- Modify: `server/Cargo.toml`
- Create: `server/migrations/0001_foundation.sql`
- Create: `server/src/db.rs`
- Modify: `server/src/config.rs`
- Modify: `server/src/main.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `db::open(url: &str) -> anyhow::Result<SqlitePool>`, `db::now() -> i64`, `Config.database_url: String`, `Config.admin_email: Option<String>`, `Config.admin_password: Option<String>`, `AppState.db: SqlitePool`.

- [ ] **Step 1: Add dependencies**

In `server/Cargo.toml` `[dependencies]` add:

```toml
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "migrate"] }
argon2 = "0.5"
```

Change the uuid line to `uuid = { version = "1", features = ["v4", "v5", "v7"] }`.

Add a `[dev-dependencies]` section:

```toml
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
tempfile = "3"
```

Run: `cargo fetch` — Expected: resolves. If `sqlx` rejects the feature names (0.9 renamed something), run `cargo info sqlx` and use the sqlite + tokio runtime features it lists; the API used below (`SqlitePoolOptions`, `SqliteConnectOptions`, `sqlx::migrate!`, `sqlx::query`) is stable across 0.8 and 0.9.

- [ ] **Step 2: Write the migration**

`server/migrations/0001_foundation.sql`:

```sql
CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    color         TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

CREATE TABLE projects (
    id         TEXT PRIMARY KEY,
    media_id   TEXT NOT NULL,
    owner_id   TEXT NOT NULL REFERENCES users(id),
    title      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX projects_media ON projects(media_id);

CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'commenter', 'viewer')),
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE edit_ops (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    op_id      TEXT NOT NULL,
    author_id  TEXT NOT NULL REFERENCES users(id),
    op         TEXT NOT NULL,
    undone_by  INTEGER,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, seq),
    UNIQUE (project_id, op_id)
);
```

- [ ] **Step 3: Write the failing test for `db::open`**

`server/src/db.rs`:

```rust
//! SQLite connection pool and migrations. The database holds metadata and
//! the operation log; media files stay on disk under `DATA_DIR`.

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Open (creating if needed) the database at `url` and apply migrations.
pub async fn open(url: &str) -> anyhow::Result<SqlitePool> {
    todo!()
}

/// Unix seconds, the timestamp format used by every table.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        let pool = open(&url).await.unwrap();
        let tables: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        for expected in ["users", "sessions", "projects", "project_members", "edit_ops"] {
            assert!(names.contains(&expected), "missing table {expected} in {names:?}");
        }
    }

    #[tokio::test]
    async fn open_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        open(&url).await.unwrap();
        open(&url).await.unwrap();
    }
}
```

Add `mod db;` to `server/src/main.rs`.

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p server db::` — Expected: panics with `not yet implemented`.

- [ ] **Step 5: Implement `open`**

```rust
pub async fn open(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p server db::` — Expected: 2 passed.

- [ ] **Step 7: Wire config and state**

In `server/src/config.rs` add three fields to `Config` and set them in `from_env`:

```rust
    pub database_url: String,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
```

```rust
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                format!(
                    "sqlite://{}/type-n-stitch.db",
                    env("DATA_DIR", concat!(env!("CARGO_MANIFEST_DIR"), "/data"))
                )
            }),
            admin_email: std::env::var("ADMIN_EMAIL").ok(),
            admin_password: std::env::var("ADMIN_PASSWORD").ok(),
```

Note the `data_dir` field is built from the same `env("DATA_DIR", …)` expression; copy it exactly so the default DB lands next to the media directories.

In `server/src/main.rs` add `pub db: sqlx::SqlitePool,` to `AppState`, and in `main` after `create_dir_all`:

```rust
    let db = db::open(&config.database_url).await?;
```

and pass `db` when building `AppState`.

Add `server/data/*.db*` is already covered by `server/data/` in `.gitignore`; no change needed unless `DATA_DIR` is overridden. Leave `.gitignore` as is.

- [ ] **Step 8: Verify build and lint**

Run: `cargo build -p server && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add server/Cargo.toml Cargo.lock server/migrations server/src/db.rs server/src/config.rs server/src/main.rs
git commit -m "server: add SQLite pool, migrations and the foundation schema"
```

---

### Task 2: Engine operation type and fold

**Files:**
- Create: `engine/src/ops.rs`
- Modify: `engine/src/lib.rs`

**Interfaces:**
- Produces:
  - `enum Op { Cut{start,end}, Overdub{start,end,text,audio_url,audio_duration}, ApplyCuts{cuts: Vec<Range>}, RenameSpeaker{speaker: u32, name: String}, Undo{target_seq: i64}, Redo{target_seq: i64} }` — serde `tag = "kind"`, lowercase, camelCase fields.
  - `struct SeqOp { seq: i64, author_id: String, op: Op, undone: bool }`
  - `struct ProjectDoc { edits: Vec<Edit>, speaker_names: Vec<String> }`
  - `fn fold(ops: &[SeqOp]) -> ProjectDoc`

- [ ] **Step 1: Write the failing tests**

`engine/src/ops.rs`:

```rust
//! The operation log: what a collaborator did, in server order. A project's
//! edit list is the fold of its operations; undone operations are skipped.
//! Mirrors the rules in the client's `editor.ts` reducer.

use serde::{Deserialize, Serialize};

use crate::types::{Edit, Range};

/// One change submitted by a client. `Undo` and `Redo` never appear in a
/// fold's output; the server marks their targets so `fold` can skip them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Op {
    Cut {
        start: f64,
        end: f64,
    },
    #[serde(rename_all = "camelCase")]
    Overdub {
        start: f64,
        end: f64,
        text: String,
        audio_url: String,
        audio_duration: f64,
    },
    #[serde(rename_all = "camelCase")]
    ApplyCuts {
        cuts: Vec<Range>,
    },
    #[serde(rename_all = "camelCase")]
    RenameSpeaker {
        speaker: u32,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    Undo {
        target_seq: i64,
    },
    #[serde(rename_all = "camelCase")]
    Redo {
        target_seq: i64,
    },
}

/// An operation as stored: its position in the log, who sent it, and
/// whether a later undo removed it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeqOp {
    pub seq: i64,
    pub author_id: String,
    pub op: Op,
    pub undone: bool,
}

/// Everything the fold produces: the edit list the engine renders plus
/// project-level state that is not an edit.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDoc {
    pub edits: Vec<Edit>,
    /// Display names by speaker index; empty strings mean "unnamed".
    pub speaker_names: Vec<String>,
}

/// Replay the log in order, skipping undone operations.
pub fn fold(ops: &[SeqOp]) -> ProjectDoc {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(seq: i64, op: Op) -> SeqOp {
        SeqOp {
            seq,
            author_id: "u1".into(),
            op,
            undone: false,
        }
    }

    fn undone(seq: i64, op: Op) -> SeqOp {
        SeqOp {
            undone: true,
            ..self::op(seq, op)
        }
    }

    fn cut(start: f64, end: f64) -> Op {
        Op::Cut { start, end }
    }

    fn overdub(start: f64, end: f64) -> Op {
        Op::Overdub {
            start,
            end,
            text: "x".into(),
            audio_url: "/data/m/overdub-0.wav".into(),
            audio_duration: 1.0,
        }
    }

    #[test]
    fn empty_log_is_empty_doc() {
        assert_eq!(fold(&[]), ProjectDoc::default());
    }

    #[test]
    fn cuts_append_in_order() {
        let doc = fold(&[op(1, cut(1.0, 2.0)), op(2, cut(5.0, 6.0))]);
        assert_eq!(
            doc.edits,
            vec![
                Edit::Cut { start: 1.0, end: 2.0 },
                Edit::Cut { start: 5.0, end: 6.0 }
            ]
        );
    }

    #[test]
    fn undone_ops_are_skipped() {
        let doc = fold(&[undone(1, cut(1.0, 2.0)), op(2, cut(5.0, 6.0))]);
        assert_eq!(doc.edits, vec![Edit::Cut { start: 5.0, end: 6.0 }]);
    }

    #[test]
    fn undo_and_redo_rows_produce_no_edits() {
        let doc = fold(&[
            op(1, cut(1.0, 2.0)),
            op(2, Op::Undo { target_seq: 1 }),
            op(3, Op::Redo { target_seq: 2 }),
        ]);
        // The server flips `undone` on row 1; the fold only reads that flag.
        assert_eq!(doc.edits, vec![Edit::Cut { start: 1.0, end: 2.0 }]);
    }

    #[test]
    fn cut_removes_overdubs_inside_it() {
        let doc = fold(&[op(1, overdub(2.0, 3.0)), op(2, cut(1.0, 4.0))]);
        assert_eq!(doc.edits, vec![Edit::Cut { start: 1.0, end: 4.0 }]);
    }

    #[test]
    fn cut_keeps_overdubs_that_only_overlap() {
        let doc = fold(&[op(1, overdub(2.0, 5.0)), op(2, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 2);
        assert!(matches!(doc.edits[0], Edit::Overdub { start, .. } if start == 2.0));
    }

    #[test]
    fn overdub_replaces_overlapping_overdubs() {
        let doc = fold(&[op(1, overdub(2.0, 4.0)), op(2, overdub(3.0, 5.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Overdub { start, end, .. } if start == 3.0 && end == 5.0));
    }

    #[test]
    fn undoing_a_cut_restores_the_overdub_it_removed() {
        let doc = fold(&[op(1, overdub(2.0, 3.0)), undone(2, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Overdub { .. }));
    }

    #[test]
    fn apply_cuts_appends_every_cut() {
        let doc = fold(&[op(
            1,
            Op::ApplyCuts {
                cuts: vec![Range::new(1.0, 2.0), Range::new(3.0, 4.0)],
            },
        )]);
        assert_eq!(doc.edits.len(), 2);
    }

    #[test]
    fn rename_speaker_grows_the_name_list() {
        let doc = fold(&[
            op(
                1,
                Op::RenameSpeaker {
                    speaker: 2,
                    name: "Ada".into(),
                },
            ),
            op(
                2,
                Op::RenameSpeaker {
                    speaker: 0,
                    name: "Bob".into(),
                },
            ),
        ]);
        assert_eq!(doc.speaker_names, vec!["Bob", "", "Ada"]);
    }

    #[test]
    fn two_authors_interleave_by_seq() {
        let a = SeqOp {
            seq: 1,
            author_id: "a".into(),
            op: cut(1.0, 2.0),
            undone: false,
        };
        let b = SeqOp {
            seq: 2,
            author_id: "b".into(),
            op: cut(3.0, 4.0),
            undone: false,
        };
        let doc = fold(&[a, b]);
        assert_eq!(doc.edits.len(), 2);
    }

    #[test]
    fn op_json_round_trips_with_camel_case() {
        let json = serde_json::to_value(Op::Undo { target_seq: 7 }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "undo", "targetSeq": 7 }));
        let back: Op = serde_json::from_value(json).unwrap();
        assert_eq!(back, Op::Undo { target_seq: 7 });
        let cuts: Op = serde_json::from_str(r#"{"kind":"applycuts","cuts":[{"start":1,"end":2}]}"#).unwrap();
        assert!(matches!(cuts, Op::ApplyCuts { .. }));
    }
}
```

In `engine/src/lib.rs` add `pub mod ops;` and `pub use ops::*;` in the same alphabetical position as the others.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p engine ops::` — Expected: every test panics with `not yet implemented`, except `op_json_round_trips_with_camel_case` which passes (serde only).

- [ ] **Step 3: Implement `fold`**

Replace the `todo!()` body:

```rust
pub fn fold(ops: &[SeqOp]) -> ProjectDoc {
    let mut doc = ProjectDoc::default();
    for entry in ops.iter().filter(|o| !o.undone) {
        apply(&mut doc, &entry.op);
    }
    doc
}

fn inside(inner: Range, outer: Range) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

fn overlaps(a: Range, b: Range) -> bool {
    a.start < b.end && a.end > b.start
}

fn apply(doc: &mut ProjectDoc, op: &Op) {
    match op {
        Op::Cut { start, end } => {
            let cut = Range::new(*start, *end);
            // Deleting an overdubbed passage removes the overdub with it.
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if inside(e.range(), cut)));
            doc.edits.push(Edit::Cut {
                start: *start,
                end: *end,
            });
        }
        Op::Overdub {
            start,
            end,
            text,
            audio_url,
            audio_duration,
        } => {
            let span = Range::new(*start, *end);
            // A new overdub replaces any it overlaps.
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if overlaps(e.range(), span)));
            doc.edits.push(Edit::Overdub {
                start: *start,
                end: *end,
                text: text.clone(),
                audio_url: audio_url.clone(),
                audio_duration: *audio_duration,
            });
        }
        Op::ApplyCuts { cuts } => doc.edits.extend(cuts.iter().map(|r| Edit::Cut {
            start: r.start,
            end: r.end,
        })),
        Op::RenameSpeaker { speaker, name } => {
            let i = *speaker as usize;
            if doc.speaker_names.len() <= i {
                doc.speaker_names.resize(i + 1, String::new());
            }
            doc.speaker_names[i] = name.trim().to_owned();
        }
        // Undo and redo only flip `undone` flags; the server does that when
        // it appends them, so here they are no-ops.
        Op::Undo { .. } | Op::Redo { .. } => {}
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p engine ops::` — Expected: 12 passed.

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add engine/src/ops.rs engine/src/lib.rs
git commit -m "engine: add the operation log type and its fold"
```

---

### Task 3: Auth — passwords, sessions, `CurrentUser`, handlers

**Files:**
- Create: `server/src/auth.rs`
- Create: `server/src/test_util.rs`
- Create: `server/src/app.rs`
- Modify: `server/src/error.rs`
- Modify: `server/src/main.rs`

**Interfaces:**
- Consumes: `db::now()`, `AppState.db`.
- Produces:
  - `struct User { id, email, display_name, color }` (Serialize camelCase).
  - `struct CurrentUser(pub User)` — axum extractor; rejects with 401.
  - `auth::create_user(db, email, password, display_name) -> AppResult<User>`.
  - `auth::count_users(db) -> AppResult<i64>`.
  - `AppError::unauthorized()`, `AppError::forbidden(msg)`, `AppError::bad_request_at(index, msg)`.
  - `app::router(state: Arc<AppState>) -> Router`.
  - Test helpers: `test_util::state() -> (Arc<AppState>, TempDir)`, `test_util::register(app, email) -> String` (returns cookie), `test_util::call(app, Request) -> (StatusCode, Value, HeaderMap)`, `test_util::json_req(method, uri, cookie: Option<&str>, body: Option<Value>) -> Request<Body>`.
  - Routes: `POST /api/auth/register {email,password,displayName}`, `POST /api/auth/login {email,password}`, `POST /api/auth/logout`, `GET /api/me`, `GET /api/auth/setup → {needsSetup}`.

- [ ] **Step 1: Extend `AppError`**

In `server/src/error.rs` add an `index: Option<usize>` field and three constructors, and include `index` in the body when present:

```rust
#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
    /// Which item in a submitted batch was rejected, if any.
    index: Option<usize>,
}
```

Update the existing constructors to set `index: None`, then add:

```rust
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "sign in first".into(),
            index: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            index: None,
        }
    }

    /// A bad request that names which entry of a batch was rejected.
    pub fn bad_request_at(index: usize, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            index: Some(index),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
```

`From<E>` sets `index: None`. `into_response` becomes:

```rust
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let Some(index) = self.index {
            body["index"] = json!(index);
        }
        (self.status, Json(body)).into_response()
    }
}
```

- [ ] **Step 2: Move router construction into `app.rs`**

Create `server/src/app.rs` containing the `Router::new()…with_state(state.clone())` expression from `main.rs` as:

```rust
//! The axum router, built once for `main` and once per test.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::{auth, library, routes, AppState};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/auth/setup", get(auth::setup))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/library", get(library::list))
        .route("/api/library/{slug}", post(library::open))
        .route("/api/media", post(routes::upload))
        .route("/api/media/{id}/transcribe", post(routes::transcribe))
        .route("/api/media/{id}/suggest", post(routes::suggest))
        .route("/api/media/{id}/thumbnails", post(routes::thumbnails))
        .route("/api/media/{id}/speakers", post(routes::speakers))
        .route("/api/media/{id}/overdub", post(routes::overdub))
        .route("/api/media/{id}/export", post(routes::export))
        .route(
            "/api/media/{id}/export/{job}/progress",
            get(routes::export_progress),
        )
        .nest_service("/data", ServeDir::new(&state.config.data_dir))
        .nest_service(
            "/library",
            ServeDir::new(state.config.samples_dir.join("library")),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
```

In `main.rs`: add `mod app; mod auth; #[cfg(test)] mod test_util;`, delete the imports the router used (`DefaultBodyLimit`, `get`, `post`, `Router`, `CorsLayer`, `ServeDir`, `TraceLayer`), and replace the router block with `let app = app::router(state.clone());`.

- [ ] **Step 3: Write the test helpers**

`server/src/test_util.rs`:

```rust
//! Shared helpers for handler tests: a temp `AppState` and one-shot requests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::config::Config;
use crate::{app, db, AppState};

/// A state whose data dir and database live in a fresh temp dir.
pub async fn state() -> (Arc<AppState>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.data_dir = dir.path().join("data");
    config.database_url = format!("sqlite://{}/test.db", dir.path().display());
    config.admin_email = None;
    config.admin_password = None;
    tokio::fs::create_dir_all(&config.data_dir).await.unwrap();
    let db = db::open(&config.database_url).await.unwrap();
    let state = Arc::new(AppState {
        http: reqwest::Client::new(),
        config,
        jobs: Mutex::new(HashMap::new()),
        db,
        folds: Mutex::new(HashMap::new()),
    });
    (state, dir)
}

pub fn app(state: &Arc<AppState>) -> Router {
    app::router(state.clone())
}

pub fn json_req(method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

pub async fn call(app: Router, req: Request<Body>) -> (StatusCode, Value, HeaderMap) {
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, headers)
}

/// The `name=value` part of the session cookie a response set.
pub fn cookie_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::SET_COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .next()
        .map(str::to_owned)
}

/// Register `email` with password "pw-secret" and return its session cookie.
pub async fn register(state: &Arc<AppState>, email: &str) -> String {
    let (status, _, headers) = call(
        app(state),
        json_req(
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({ "email": email, "password": "pw-secret", "displayName": email })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie_of(&headers).expect("register sets a cookie")
}
```

`folds` is added to `AppState` in Task 5; add the field now as `pub folds: Mutex<HashMap<String, (i64, engine::ProjectDoc)>>` in `main.rs` and initialise it with `Mutex::new(HashMap::new())` so this helper compiles.

- [ ] **Step 4: Write the failing auth tests**

`server/src/auth.rs` (tests first; the implementation follows in Step 6):

```rust
//! Accounts and sessions: argon2id passwords, an opaque session token in an
//! HttpOnly cookie, and the `CurrentUser` extractor every private route uses.

use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::response::{AppendHeaders, IntoResponse};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::AppState;

pub const COOKIE_NAME: &str = "tns_session";
const SESSION_SECS: i64 = 30 * 24 * 60 * 60;

/// Avatar colours, assigned round-robin at registration.
const COLORS: &[&str] = &[
    "#e0575b", "#e08f3c", "#c9a227", "#4caf6e", "#3c8fd1", "#7c5cd6", "#d15ca7", "#3fb3b3",
];

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub color: String,
}

/// The signed-in user, or a 401.
pub struct CurrentUser(pub User);

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use crate::test_util::{app, call, cookie_of, json_req, register, state};

    #[tokio::test]
    async fn register_sets_cookie_and_me_returns_user() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (status, body, _) = call(app(&state), json_req(Method::GET, "/api/me", Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["email"], "ada@example.com");
        assert_eq!(body["displayName"], "ada@example.com");
        assert!(body["color"].as_str().unwrap().starts_with('#'));
        assert!(body.get("passwordHash").is_none());
    }

    #[tokio::test]
    async fn me_without_cookie_is_401() {
        let (state, _dir) = state().await;
        let (status, body, _) = call(app(&state), json_req(Method::GET, "/api/me", None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "sign in first");
    }

    #[tokio::test]
    async fn duplicate_email_is_400_case_insensitive() {
        let (state, _dir) = state().await;
        register(&state, "ada@example.com").await;
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/register",
                None,
                Some(json!({ "email": "ADA@example.com", "password": "pw-secret", "displayName": "Ada" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn short_password_is_400() {
        let (state, _dir) = state().await;
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/register",
                None,
                Some(json!({ "email": "a@example.com", "password": "short", "displayName": "A" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn login_with_right_and_wrong_password() {
        let (state, _dir) = state().await;
        register(&state, "ada@example.com").await;
        let (status, _, headers) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": "ada@example.com", "password": "pw-secret" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(cookie_of(&headers).is_some());

        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": "ada@example.com", "password": "nope" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_invalidates_the_session() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (status, _, _) = call(app(&state), json_req(Method::POST, "/api/auth/logout", Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(app(&state), json_req(Method::GET, "/api/me", Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn expired_session_is_401() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        sqlx::query("UPDATE sessions SET expires_at = 0")
            .execute(&state.db)
            .await
            .unwrap();
        let (status, _, _) = call(app(&state), json_req(Method::GET, "/api/me", Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn setup_reports_whether_any_user_exists() {
        let (state, _dir) = state().await;
        let (_, body, _) = call(app(&state), json_req(Method::GET, "/api/auth/setup", None, None)).await;
        assert_eq!(body["needsSetup"], true);
        register(&state, "ada@example.com").await;
        let (_, body, _) = call(app(&state), json_req(Method::GET, "/api/auth/setup", None, None)).await;
        assert_eq!(body["needsSetup"], false);
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test -p server auth::` — Expected: compile errors for the missing handlers (`auth::setup` etc. referenced from `app.rs`). That is the failing state; proceed.

- [ ] **Step 6: Implement auth**

Add below `CurrentUser` in `server/src/auth.rs`:

```rust
impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, AppError> {
        let token = session_token(parts).ok_or_else(AppError::unauthorized)?;
        let user: Option<User> = sqlx::query_as(
            "SELECT u.id, u.email, u.display_name, u.color
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.id = ? AND s.expires_at > ?",
        )
        .bind(&token)
        .bind(now())
        .fetch_optional(&state.db)
        .await?;
        user.map(CurrentUser).ok_or_else(AppError::unauthorized)
    }
}

/// The session token from the `Cookie` header, if present.
fn session_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(COOKIE_NAME).and_then(|rest| rest.strip_prefix('=')))
        .map(str::to_owned)
}

fn set_cookie(token: &str, max_age: i64) -> [(axum::http::HeaderName, String); 1] {
    [(
        SET_COOKIE,
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}"),
    )]
}

fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}").into())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

pub async fn count_users(db: &SqlitePool) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users").fetch_one(db).await?;
    Ok(n)
}

pub async fn create_user(db: &SqlitePool, email: &str, password: &str, display_name: &str) -> AppResult<User> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') {
        return Err(AppError::bad_request("enter a valid email address"));
    }
    if password.len() < 8 {
        return Err(AppError::bad_request("password must be at least 8 characters"));
    }
    let display_name = display_name.trim();
    let display_name = if display_name.is_empty() { email.as_str() } else { display_name };
    let n = count_users(db).await?;
    let color = COLORS[(n as usize) % COLORS.len()];
    let user = User {
        id: Uuid::new_v4().to_string(),
        email,
        display_name: display_name.to_owned(),
        color: color.to_owned(),
    };
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name, color, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.email)
    .bind(hash_password(password)?)
    .bind(&user.display_name)
    .bind(&user.color)
    .bind(now())
    .execute(db)
    .await;
    match inserted {
        Ok(_) => Ok(user),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(AppError::bad_request("that email is already registered"))
        }
        Err(e) => Err(e.into()),
    }
}

async fn create_session(db: &SqlitePool, user_id: &str) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(user_id)
        .bind(now() + SESSION_SECS)
        .bind(now())
        .execute(db)
        .await?;
    Ok(id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    email: String,
    password: String,
    #[serde(default)]
    display_name: String,
}

/// `POST /api/auth/register` — create an account and sign in.
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<impl IntoResponse> {
    let user = create_user(&state.db, &req.email, &req.password, &req.display_name).await?;
    if count_users(&state.db).await? == 1 {
        crate::projects::adopt_orphans(&state, &user.id).await?;
    }
    let token = create_session(&state.db, &user.id).await?;
    Ok((AppendHeaders(set_cookie(&token, SESSION_SECS)), Json(user)))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

/// `POST /api/auth/login`.
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let row: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, email, display_name, color, password_hash FROM users WHERE email = ?",
    )
    .bind(req.email.trim().to_ascii_lowercase())
    .fetch_optional(&state.db)
    .await?;
    let Some((id, email, display_name, color, hash)) = row else {
        return Err(AppError::unauthorized());
    };
    if !verify_password(&req.password, &hash) {
        return Err(AppError::unauthorized());
    }
    let token = create_session(&state.db, &id).await?;
    let user = User { id, email, display_name, color };
    Ok((AppendHeaders(set_cookie(&token, SESSION_SECS)), Json(user)))
}

/// `POST /api/auth/logout` — forget the session and clear the cookie.
pub async fn logout(State(state): State<Arc<AppState>>, parts: Parts) -> AppResult<impl IntoResponse> {
    if let Some(token) = session_token(&parts) {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(token)
            .execute(&state.db)
            .await?;
    }
    Ok((AppendHeaders(set_cookie("", 0)), Json(json!({ "ok": true }))))
}

/// `GET /api/me`.
pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

/// `GET /api/auth/setup` — whether the first account still needs creating.
pub async fn setup(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    Ok(Json(json!({ "needsSetup": count_users(&state.db).await? == 0 })))
}
```

`logout` takes `Parts` as an extractor: axum implements `FromRequestParts` for `Parts`, but it must be the last extractor without a body; keep the order `(State, Parts)`.

`crate::projects::adopt_orphans` does not exist until Task 4. For this task, create `server/src/projects.rs` with only:

```rust
//! Projects, membership and roles.

use std::sync::Arc;

use crate::error::AppResult;
use crate::AppState;

/// Give every media directory that has no project yet to `owner_id`.
pub async fn adopt_orphans(_state: &Arc<AppState>, _owner_id: &str) -> AppResult<()> {
    Ok(())
}
```

and add `mod projects;` to `main.rs`. Task 4 fills it in.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p server` — Expected: all `auth::` and `db::` tests pass.

- [ ] **Step 8: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add server/src
git commit -m "server: add accounts, sessions and the CurrentUser extractor"
```

---

### Task 4: Projects, roles and the `ProjectAccess` extractor

**Files:**
- Modify: `server/src/projects.rs`
- Modify: `server/src/app.rs`
- Modify: `server/src/routes.rs` (make `Meta` fields `pub`, already are; expose `item_dir` as `pub(crate)`)

**Interfaces:**
- Consumes: `CurrentUser`, `routes::read_meta`, `routes::Meta`.
- Produces:
  - `enum Role { Owner, Editor, Commenter, Viewer }` with `as_str()`, `parse(&str) -> Option<Role>`, `can_edit()`, `can_manage()`.
  - `struct Project { id, media_id, owner_id, title, created_at }` (FromRow).
  - `struct ProjectAccess { user: User, project: Project, role: Role }` — extractor reading the `{id}` path param; 401 / 404 / 403 in that order.
  - `ProjectAccess::require_edit(&self) -> AppResult<()>`, `require_manage(&self) -> AppResult<()>`.
  - `projects::create_project(db, owner: &User, media_id, title) -> AppResult<Project>`.
  - `projects::adopt_orphans(state, owner_id)`.
  - `struct ProjectSummary { id, title, role, media: Meta, created_at }` for list/create responses.
  - Routes: `GET /api/projects`, `POST /api/projects/{id}/members {email, role}`, `DELETE /api/projects/{id}/members/{user_id}`, `GET /api/projects/{id}/members`.
  - Test helper: `projects::test_support::seed_media(state, duration) -> String` writes a fake `meta.json` and returns the media id.

`POST /api/projects` (upload) and `GET /api/projects/{id}` arrive in Tasks 5 and 6.

- [ ] **Step 1: Write the failing tests**

Replace `server/src/projects.rs` with the tests-first skeleton:

```rust
//! Projects, membership and roles. A project is one media directory plus its
//! operation log; membership decides who may read, comment on or edit it.

use std::sync::Arc;

use axum::extract::{FromRequestParts, Path as UrlPath, RawPathParams, State};
use axum::http::request::Parts;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::{CurrentUser, User};
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::routes::{read_meta, Meta};
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Commenter,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Commenter => "commenter",
            Role::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "commenter" => Some(Role::Commenter),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    pub fn can_edit(self) -> bool {
        matches!(self, Role::Owner | Role::Editor)
    }

    pub fn can_manage(self) -> bool {
        self == Role::Owner
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub media_id: String,
    pub owner_id: String,
    pub title: String,
    pub created_at: i64,
}

/// A signed-in user's access to the project named by the `{id}` path
/// parameter. Rejections, in order: 401 (no session), 404 (no such project),
/// 403 (not a member).
pub struct ProjectAccess {
    pub user: User,
    pub project: Project,
    pub role: Role,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub title: String,
    pub role: Role,
    pub media: Meta,
    pub created_at: i64,
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::routes::Meta;
    use crate::AppState;

    /// A fake media item on disk: just `meta.json`, enough for project routes.
    pub async fn seed_media(state: &Arc<AppState>, duration: f64) -> String {
        let id = Uuid::new_v4().to_string();
        let dir = state.config.data_dir.join(&id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let meta = Meta {
            id: id.clone(),
            filename: "clip.mp4".into(),
            ext: "mp4".into(),
            duration,
            kind: engine::MediaKind::Video,
            url: format!("/data/{id}/source.mp4"),
        };
        tokio::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).unwrap())
            .await
            .unwrap();
        id
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::test_support::seed_media;
    use super::*;
    use crate::test_util::{app, call, json_req, register, state};

    async fn user_id(state: &Arc<AppState>, cookie: &str) -> String {
        let (_, me, _) = call(app(state), json_req(Method::GET, "/api/me", Some(cookie), None)).await;
        me["id"].as_str().unwrap().to_owned()
    }

    async fn owned_project(state: &Arc<AppState>, cookie: &str) -> Project {
        let media_id = seed_media(state, 10.0).await;
        let (_, me, _) = call(app(state), json_req(Method::GET, "/api/me", Some(cookie), None)).await;
        let owner: User = serde_json::from_value(me).unwrap();
        create_project(&state.db, &owner, &media_id, "Clip").await.unwrap()
    }

    #[tokio::test]
    async fn list_shows_only_projects_the_user_belongs_to() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let (_, list, _) = call(app(&state), json_req(Method::GET, "/api/projects", Some(&ada), None)).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], project.id);
        assert_eq!(list[0]["role"], "owner");
        assert_eq!(list[0]["media"]["duration"], 10.0);
        let (_, list, _) = call(app(&state), json_req(Method::GET, "/api/projects", Some(&bob), None)).await;
        assert_eq!(list.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn members_route_distinguishes_401_404_403() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);

        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects/does-not-exist/members", Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, Some(&bob), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body, _) = call(app(&state), json_req(Method::GET, &uri, Some(&ada), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn owner_adds_and_removes_a_member_by_email() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);

        let (status, body, _) = call(
            app(&state),
            json_req(Method::POST, &uri, Some(&ada), Some(json!({ "email": "bob@example.com", "role": "viewer" }))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, list, _) = call(app(&state), json_req(Method::GET, "/api/projects", Some(&bob), None)).await;
        assert_eq!(list[0]["role"], "viewer");

        // A viewer cannot manage members.
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, &uri, Some(&bob), Some(json!({ "email": "ada@example.com", "role": "editor" }))),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let bob_id = user_id(&state, &bob).await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::DELETE, &format!("{uri}/{bob_id}"), Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, Some(&bob), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unknown_email_and_bad_role_are_400_and_owner_cannot_be_removed() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, &uri, Some(&ada), Some(json!({ "email": "zed@example.com", "role": "viewer" }))),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, &uri, Some(&ada), Some(json!({ "email": "ada@example.com", "role": "boss" }))),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let ada_id = user_id(&state, &ada).await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::DELETE, &format!("{uri}/{ada_id}"), Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn adopt_orphans_creates_a_project_per_media_dir_once() {
        let (state, _dir) = state().await;
        seed_media(&state, 4.0).await;
        seed_media(&state, 5.0).await;
        let ada = register(&state, "ada@example.com").await; // first user adopts
        let (_, list, _) = call(app(&state), json_req(Method::GET, "/api/projects", Some(&ada), None)).await;
        assert_eq!(list.as_array().unwrap().len(), 2);
        let (_, me, _) = call(app(&state), json_req(Method::GET, "/api/me", Some(&ada), None)).await;
        adopt_orphans(&state, me["id"].as_str().unwrap()).await.unwrap();
        let (_, list, _) = call(app(&state), json_req(Method::GET, "/api/projects", Some(&ada), None)).await;
        assert_eq!(list.as_array().unwrap().len(), 2);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p server projects::` — Expected: compile errors (`create_project`, `adopt_orphans`, routes missing).

- [ ] **Step 3: Implement**

Insert between `ProjectSummary` and `#[cfg(test)] pub mod test_support`:

```rust
impl ProjectAccess {
    pub fn require_edit(&self) -> AppResult<()> {
        if self.role.can_edit() {
            Ok(())
        } else {
            Err(AppError::forbidden("you can view this project but not edit it"))
        }
    }

    pub fn require_manage(&self) -> AppResult<()> {
        if self.role.can_manage() {
            Ok(())
        } else {
            Err(AppError::forbidden("only the owner can manage members"))
        }
    }
}

impl FromRequestParts<Arc<AppState>> for ProjectAccess {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, AppError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        let params = RawPathParams::from_request_parts(parts, state)
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?;
        let id = params
            .iter()
            .find(|(k, _)| *k == "id")
            .map(|(_, v)| v.to_owned())
            .ok_or_else(|| AppError::bad_request("missing project id"))?;
        let project = find_project(&state.db, &id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("no project with id {id}")))?;
        let role = member_role(&state.db, &project.id, &user.id)
            .await?
            .ok_or_else(|| AppError::forbidden("you are not a member of this project"))?;
        Ok(ProjectAccess { user, project, role })
    }
}

pub async fn find_project(db: &SqlitePool, id: &str) -> AppResult<Option<Project>> {
    Ok(sqlx::query_as("SELECT id, media_id, owner_id, title, created_at FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?)
}

async fn member_role(db: &SqlitePool, project_id: &str, user_id: &str) -> AppResult<Option<Role>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT role FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(project_id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.and_then(|(r,)| Role::parse(&r)))
}

pub async fn create_project(db: &SqlitePool, owner: &User, media_id: &str, title: &str) -> AppResult<Project> {
    let project = Project {
        id: Uuid::new_v4().to_string(),
        media_id: media_id.to_owned(),
        owner_id: owner.id.clone(),
        title: title.to_owned(),
        created_at: now(),
    };
    let mut tx = db.begin().await?;
    sqlx::query("INSERT INTO projects (id, media_id, owner_id, title, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&project.id)
        .bind(&project.media_id)
        .bind(&project.owner_id)
        .bind(&project.title)
        .bind(project.created_at)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES (?, ?, 'owner')")
        .bind(&project.id)
        .bind(&owner.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(project)
}

pub async fn summary(state: &AppState, project: &Project, role: Role) -> AppResult<ProjectSummary> {
    let media = read_meta(&state.config.data_dir.join(&project.media_id)).await?;
    Ok(ProjectSummary {
        id: project.id.clone(),
        title: project.title.clone(),
        role,
        media,
        created_at: project.created_at,
    })
}

/// `GET /api/projects` — every project the user is a member of, newest first.
pub async fn list(State(state): State<Arc<AppState>>, CurrentUser(user): CurrentUser) -> AppResult<Json<Vec<ProjectSummary>>> {
    let rows: Vec<(String, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT p.id, p.media_id, p.owner_id, p.title, p.created_at, m.role
         FROM projects p JOIN project_members m ON m.project_id = p.id
         WHERE m.user_id = ? ORDER BY p.created_at DESC",
    )
    .bind(&user.id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, media_id, owner_id, title, created_at, role) in rows {
        let project = Project { id, media_id, owner_id, title, created_at };
        let role = Role::parse(&role).unwrap_or(Role::Viewer);
        // A project whose media directory vanished is skipped, not fatal.
        if let Ok(s) = summary(&state, &project, role).await {
            out.push(s);
        }
    }
    Ok(Json(out))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    #[serde(flatten)]
    user: User,
    role: Role,
}

/// `GET /api/projects/:id/members`.
pub async fn members(State(state): State<Arc<AppState>>, access: ProjectAccess) -> AppResult<Json<Vec<Member>>> {
    Ok(Json(list_members(&state.db, &access.project.id).await?))
}

async fn list_members(db: &SqlitePool, project_id: &str) -> AppResult<Vec<Member>> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, u.color, m.role
         FROM project_members m JOIN users u ON u.id = m.user_id
         WHERE m.project_id = ? ORDER BY m.role, u.display_name",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, display_name, color, role)| Member {
            user: User { id, email, display_name, color },
            role: Role::parse(&role).unwrap_or(Role::Viewer),
        })
        .collect())
}

#[derive(Deserialize)]
pub struct AddMember {
    email: String,
    role: String,
}

/// `POST /api/projects/:id/members` — add or re-role a member by email.
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    Json(req): Json<AddMember>,
) -> AppResult<Json<Vec<Member>>> {
    access.require_manage()?;
    let role = Role::parse(&req.role)
        .filter(|r| *r != Role::Owner)
        .ok_or_else(|| AppError::bad_request("role must be editor, commenter or viewer"))?;
    let user: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(req.email.trim().to_ascii_lowercase())
        .fetch_optional(&state.db)
        .await?;
    let Some((user_id,)) = user else {
        return Err(AppError::bad_request("no account with that email"));
    };
    if user_id == access.project.owner_id {
        return Err(AppError::bad_request("the owner's role cannot change"));
    }
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role) VALUES (?, ?, ?)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(&access.project.id)
    .bind(&user_id)
    .bind(role.as_str())
    .execute(&state.db)
    .await?;
    Ok(Json(list_members(&state.db, &access.project.id).await?))
}

/// `DELETE /api/projects/:id/members/:user_id`.
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    UrlPath((_, user_id)): UrlPath<(String, String)>,
) -> AppResult<Json<Value>> {
    access.require_manage()?;
    if user_id == access.project.owner_id {
        return Err(AppError::bad_request("the owner cannot be removed"));
    }
    sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
        .bind(&access.project.id)
        .bind(&user_id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// Give every media directory that has no project yet to `owner_id`. Runs at
/// startup for the admin user and after the first registration.
pub async fn adopt_orphans(state: &Arc<AppState>, owner_id: &str) -> AppResult<()> {
    let owner: Option<User> = sqlx::query_as("SELECT id, email, display_name, color FROM users WHERE id = ?")
        .bind(owner_id)
        .fetch_optional(&state.db)
        .await?;
    let Some(owner) = owner else { return Ok(()) };
    let mut entries = tokio::fs::read_dir(&state.config.data_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let dir = entry.path();
        if !dir.join("meta.json").is_file() {
            continue;
        }
        let Ok(meta) = read_meta(&dir).await else { continue };
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects WHERE media_id = ?")
            .bind(&meta.id)
            .fetch_one(&state.db)
            .await?;
        if n == 0 {
            create_project(&state.db, &owner, &meta.id, &meta.filename).await?;
            tracing::info!(media = meta.id, "adopted orphan media into a project");
        }
    }
    Ok(())
}
```

Register routes in `server/src/app.rs` after the auth routes:

```rust
        .route("/api/projects", get(projects::list))
        .route(
            "/api/projects/{id}/members",
            get(projects::members).post(projects::add_member),
        )
        .route(
            "/api/projects/{id}/members/{user_id}",
            axum::routing::delete(projects::remove_member),
        )
```

and add `projects` to the `use crate::{…}` line.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p server` — Expected: all pass, including the five new `projects::` tests.

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add server/src
git commit -m "server: add projects, roles, membership and the ProjectAccess extractor"
```

---

### Task 5: Operation log — `apply_ops`, fold cache, `POST /ops`, `GET /projects/:id`

**Files:**
- Create: `server/src/ops.rs`
- Modify: `server/src/main.rs` (`AppState.folds` already added in Task 3)
- Modify: `server/src/app.rs`

**Interfaces:**
- Consumes: `engine::{Op, SeqOp, ProjectDoc, fold}`, `ProjectAccess`, `read_meta`.
- Produces:
  - `struct ClientOp { op_id: String, #[serde(flatten)] op: Op }` (Deserialize).
  - `struct DocState { head_seq: i64, edits: Vec<Edit>, speaker_names: Vec<String>, undoable: Option<i64>, redoable: Option<i64> }` (Serialize camelCase).
  - `ops::apply_ops(state, project: &Project, user: &User, ops: Vec<ClientOp>) -> AppResult<DocState>` — one transaction; validates; idempotent by `op_id`.
  - `ops::doc_state(state, project, user) -> AppResult<DocState>` — cached fold plus the user's undo/redo targets.
  - `ops::load_doc(state, project_id) -> AppResult<(i64, ProjectDoc)>` — cached fold used by export in Task 6.
  - Routes: `POST /api/projects/{id}/ops { ops: [ClientOp] }` → `DocState`; `GET /api/projects/{id}` → `{ project: ProjectSummary, doc: DocState }`.

- [ ] **Step 1: Write the failing tests**

`server/src/ops.rs`:

```rust
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

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::*;
    use crate::projects::test_support::seed_media;
    use crate::projects::create_project;
    use crate::test_util::{app, call, json_req, register, state};

    async fn me(state: &Arc<AppState>, cookie: &str) -> User {
        let (_, me, _) = call(app(state), json_req(Method::GET, "/api/me", Some(cookie), None)).await;
        serde_json::from_value(me).unwrap()
    }

    /// A 10 s project owned by ada, with bob as `role` (or not a member).
    async fn setup(role: Option<&str>) -> (Arc<AppState>, tempfile::TempDir, String, String, String) {
        let (state, dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let media = seed_media(&state, 10.0).await;
        let owner = me(&state, &ada).await;
        let project = create_project(&state.db, &owner, &media, "Clip").await.unwrap();
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

    async fn post_ops(state: &Arc<AppState>, cookie: &str, project: &str, ops: Vec<Value>) -> (StatusCode, Value) {
        let (status, body, _) = call(
            app(state),
            json_req(Method::POST, &format!("/api/projects/{project}/ops"), Some(cookie), Some(json!({ "ops": ops }))),
        )
        .await;
        (status, body)
    }

    #[tokio::test]
    async fn get_project_returns_summary_and_empty_doc() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body, _) = call(app(&state), json_req(Method::GET, &format!("/api/projects/{project}"), Some(&ada), None)).await;
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
        let (status, body) = post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0), cut("b", 3.0, 4.0)]).await;
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
        let (status, _, _) = call(app(&state), json_req(Method::GET, &format!("/api/projects/{project}"), Some(&bob), None)).await;
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
        let (status, body) = post_ops(&state, &ada, &project, vec![cut("a", 1.0, 2.0), cut("b", 5.0, 50.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["index"], 1);
        let (_, body, _) = call(app(&state), json_req(Method::GET, &format!("/api/projects/{project}"), Some(&ada), None)).await;
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
        let (status, body) = post_ops(&state, &bob, &project, vec![json!({ "opId": "u1", "kind": "undo", "targetSeq": 1 })]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        // Ada undoes her own; the fold drops it, bob's stays.
        let (status, body) = post_ops(&state, &ada, &project, vec![json!({ "opId": "u2", "kind": "undo", "targetSeq": 1 })]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["headSeq"], 3);
        assert_eq!(body["edits"].as_array().unwrap().len(), 1);
        assert_eq!(body["edits"][0]["start"], 3.0);
        assert_eq!(body["undoable"], Value::Null);
        assert_eq!(body["redoable"], 3);

        // Undoing an already-undone op is rejected.
        let (status, _) = post_ops(&state, &ada, &project, vec![json!({ "opId": "u3", "kind": "undo", "targetSeq": 1 })]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Redo brings it back.
        let (status, body) = post_ops(&state, &ada, &project, vec![json!({ "opId": "r1", "kind": "redo", "targetSeq": 3 })]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["edits"].as_array().unwrap().len(), 2);
        assert_eq!(body["undoable"], 1);
        assert_eq!(body["redoable"], Value::Null);
    }

    #[tokio::test]
    async fn rename_speaker_is_project_state() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (_, body) = post_ops(&state, &ada, &project, vec![json!({ "opId": "s", "kind": "renamespeaker", "speaker": 1, "name": "Ada" })]).await;
        assert_eq!(body["speakerNames"], json!(["", "Ada"]));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p server ops::` — Expected: compile errors for `apply_ops`, routes.

- [ ] **Step 3: Implement**

Insert after `DocState`:

```rust
/// Every stored operation for a project, in order.
async fn read_log(db: &sqlx::SqlitePool, project_id: &str) -> AppResult<Vec<SeqOp>> {
    let rows: Vec<(i64, String, String, Option<i64>)> =
        sqlx::query_as("SELECT seq, author_id, op, undone_by FROM edit_ops WHERE project_id = ? ORDER BY seq")
            .bind(project_id)
            .fetch_all(db)
            .await?;
    let mut ops = Vec::with_capacity(rows.len());
    for (seq, author_id, op, undone_by) in rows {
        let op: Op = serde_json::from_str(&op).map_err(|e| anyhow::anyhow!("corrupt op {seq}: {e}"))?;
        ops.push(SeqOp { seq, author_id, op, undone: undone_by.is_some() });
    }
    Ok(ops)
}

/// The folded document and head seq, from the cache when it is current.
pub async fn load_doc(state: &AppState, project_id: &str) -> AppResult<(i64, ProjectDoc)> {
    let (head,): (Option<i64>,) = sqlx::query_as("SELECT MAX(seq) FROM edit_ops WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(&state.db)
        .await?;
    let head = head.unwrap_or(0);
    if let Some((seq, doc)) = state.folds.lock().unwrap_or_else(|e| e.into_inner()).get(project_id) {
        if *seq == head {
            return Ok((head, doc.clone()));
        }
    }
    let doc = fold(&read_log(&state.db, project_id).await?);
    state
        .folds
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(project_id.to_owned(), (head, doc.clone()));
    Ok((head, doc))
}

fn invalidate(state: &AppState, project_id: &str) {
    state.folds.lock().unwrap_or_else(|e| e.into_inner()).remove(project_id);
}

/// What `user` may undo or redo next: their newest live op, and their newest
/// undo that has not itself been undone by a redo.
async fn undo_targets(db: &sqlx::SqlitePool, project_id: &str, user_id: &str) -> AppResult<(Option<i64>, Option<i64>)> {
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
    Ok(DocState { head_seq, edits: doc.edits, speaker_names: doc.speaker_names, undoable, redoable })
}

/// Reject operations that cannot apply to this media: ranges outside the
/// duration, overdub audio that is not this media's, undo of someone else's.
async fn validate(
    tx: &mut Transaction<'_, Sqlite>,
    state: &AppState,
    project: &Project,
    user: &User,
    index: usize,
    op: &Op,
) -> AppResult<()> {
    let dir = state.config.data_dir.join(&project.media_id);
    let duration = read_meta(&dir).await?.duration;
    let check_range = |start: f64, end: f64| -> AppResult<()> {
        if !(0.0..=duration).contains(&start) || !(0.0..=duration).contains(&end) || end < start {
            return Err(AppError::bad_request_at(index, format!("range {start}-{end} is outside the media")));
        }
        Ok(())
    };
    match op {
        Op::Cut { start, end } => check_range(*start, *end),
        Op::ApplyCuts { cuts } => cuts.iter().try_for_each(|r| check_range(r.start, r.end)),
        Op::Overdub { start, end, audio_url, .. } => {
            check_range(*start, *end)?;
            let prefix = format!("/data/{}/", project.media_id);
            let ok = audio_url
                .strip_prefix(&prefix)
                .filter(|n| n.starts_with("overdub-") && n.ends_with(".wav") && !n.contains('/'))
                .is_some_and(|n| dir.join(n).is_file());
            if ok { Ok(()) } else { Err(AppError::bad_request_at(index, "overdub audio does not belong to this media")) }
        }
        Op::RenameSpeaker { name, .. } => {
            if name.chars().count() > 80 {
                return Err(AppError::bad_request_at(index, "speaker name is too long"));
            }
            Ok(())
        }
        Op::Undo { target_seq } | Op::Redo { target_seq } => {
            let want_undo_row = matches!(op, Op::Redo { .. });
            let row: Option<(String, Option<i64>, String)> =
                sqlx::query_as("SELECT author_id, undone_by, json_extract(op, '$.kind') FROM edit_ops WHERE project_id = ? AND seq = ?")
                    .bind(&project.id)
                    .bind(target_seq)
                    .fetch_optional(&mut **tx)
                    .await?;
            let Some((author, undone_by, kind)) = row else {
                return Err(AppError::bad_request_at(index, format!("no operation {target_seq}")));
            };
            if author != user.id {
                return Err(AppError::bad_request_at(index, "you can only undo your own changes"));
            }
            if undone_by.is_some() {
                return Err(AppError::bad_request_at(index, "that change is already undone"));
            }
            let is_undo_row = kind == "undo";
            if kind == "redo" || is_undo_row != want_undo_row {
                return Err(AppError::bad_request_at(index, "that operation cannot be targeted"));
            }
            Ok(())
        }
    }
}

/// Append `ops` for `user`, in one transaction. Replays (same `op_id`) are
/// skipped so a client may resend after a lost response.
pub async fn apply_ops(state: &AppState, project: &Project, user: &User, ops: Vec<ClientOp>) -> AppResult<DocState> {
    let mut tx = state.db.begin().await?;
    for (index, client_op) in ops.iter().enumerate() {
        let exists: Option<(i64,)> = sqlx::query_as("SELECT seq FROM edit_ops WHERE project_id = ? AND op_id = ?")
            .bind(&project.id)
            .bind(&client_op.op_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_some() {
            continue;
        }
        validate(&mut tx, state, project, user, index, &client_op.op).await?;
        let (head,): (Option<i64>,) = sqlx::query_as("SELECT MAX(seq) FROM edit_ops WHERE project_id = ?")
            .bind(&project.id)
            .fetch_one(&mut *tx)
            .await?;
        let seq = head.unwrap_or(0) + 1;
        sqlx::query("INSERT INTO edit_ops (project_id, seq, op_id, author_id, op, undone_by, created_at) VALUES (?, ?, ?, ?, ?, NULL, ?)")
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
                    .bind(seq).bind(&project.id).bind(target_seq).execute(&mut *tx).await?;
            }
            Op::Redo { target_seq } => {
                // Retire the undo row and revive the op it had undone.
                sqlx::query("UPDATE edit_ops SET undone_by = ? WHERE project_id = ? AND seq = ?")
                    .bind(seq).bind(&project.id).bind(target_seq).execute(&mut *tx).await?;
                sqlx::query("UPDATE edit_ops SET undone_by = NULL WHERE project_id = ? AND undone_by = ?")
                    .bind(&project.id).bind(target_seq).execute(&mut *tx).await?;
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
    Ok(Json(apply_ops(&state, &access.project, &access.user, req.ops).await?))
}

/// `GET /api/projects/:id` — the project summary plus its folded document.
pub async fn get_project(State(state): State<Arc<AppState>>, access: ProjectAccess) -> AppResult<Json<Value>> {
    let project = summary(&state, &access.project, access.role).await?;
    let doc = doc_state(&state, &access.project, &access.user).await?;
    Ok(Json(json!({ "project": project, "doc": doc })))
}
```

Note on `Redo` bookkeeping: an undo row U (seq u) sets `undone_by = u` on its target T. A redo row R (seq r) targeting U sets `undone_by = r` on U and clears `undone_by` on every row whose `undone_by = u` — that is T. `undo_targets` then sees T live again (undoable) and U retired (not redoable). A second undo of T creates a new undo row; `fold` never reads undo/redo rows for edits, only the `undone` flag.

Register routes in `app.rs` (add `ops` to the `use crate::{…}` list):

```rust
        .route("/api/projects/{id}", get(ops::get_project))
        .route("/api/projects/{id}/ops", post(ops::submit))
```

Add `mod ops;` to `main.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p server` — Expected: all pass (9 new `ops::` tests).

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add server/src
git commit -m "server: append, validate and fold the operation log"
```

---

### Task 6: Move media routes under projects; server-side export

**Files:**
- Modify: `server/src/routes.rs`
- Modify: `server/src/library.rs`
- Modify: `server/src/app.rs`
- Modify: `server/src/main.rs`

**Interfaces:**
- Consumes: `ProjectAccess`, `projects::create_project`, `projects::summary`, `ops::load_doc`.
- Produces routes (all replace the `/api/media/…` ones, which are removed):
  - `POST /api/projects` (multipart `file`) → `ProjectSummary`
  - `POST /api/library/{slug}` → `ProjectSummary` (a project per user per clip)
  - `POST /api/projects/{id}/transcribe`, `/speakers`, `/thumbnails`, `/suggest`, `/overdub` — same bodies and responses as before.
  - `POST /api/projects/{id}/export { format? }` — edits come from the fold.
  - `GET /api/projects/{id}/export/{job}/progress`.
  - Startup: `adopt_orphans` for the admin user when `ADMIN_EMAIL`/`ADMIN_PASSWORD` are set.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `server/src/ops.rs` (it already has the fixtures):

```rust
    #[tokio::test]
    async fn media_routes_require_membership_and_upload_creates_a_project() {
        let (state, _d, ada, bob, project) = setup(None).await;
        // suggest needs a transcript; without one the route answers 404 for a
        // member, but 403 for a non-member — the extractor runs first.
        let (status, _, _) = call(app(&state), json_req(Method::POST, &format!("/api/projects/{project}/suggest"), Some(&bob), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _, _) = call(app(&state), json_req(Method::POST, &format!("/api/projects/{project}/suggest"), Some(&ada), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // The old media routes are gone.
        let (status, _, _) = call(app(&state), json_req(Method::POST, "/api/media/x/suggest", Some(&ada), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn export_uses_the_fold_and_viewers_may_not_export() {
        let (state, _d, ada, bob, project) = setup(Some("viewer")).await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, &format!("/api/projects/{project}/export"), Some(&bob), Some(json!({}))),
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
            json_req(Method::POST, &format!("/api/projects/{project}/export"), Some(&ada), Some(json!({}))),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["planned"], 6.0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p server ops::media_routes` — Expected: FAIL (old routes still answer; new ones 404).

- [ ] **Step 3: Rewrite the handlers in `routes.rs`**

Apply these changes to `server/src/routes.rs`:

1. Replace the imports of `Path as UrlPath` usage: keep `UrlPath` only for `export_progress`. Add `use crate::projects::{create_project, summary, ProjectAccess, ProjectSummary};` and `use crate::auth::CurrentUser;` and `use crate::ops::load_doc;`.

2. Change `item_dir` to `pub(crate) fn media_dir(state: &AppState, media_id: &str) -> AppResult<PathBuf>` with the same body (the `Uuid::parse_str` check stays).

3. `upload` becomes `POST /api/projects`:

```rust
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
        .map_err(|e| AppError::bad_request(e.to_string()))?
    {
        if field.name() == Some("file") {
            let meta = store_upload(&state, field).await?;
            let project = create_project(&state.db, &user, &meta.id, &meta.filename).await?;
            return Ok(Json(summary(&state, &project, crate::projects::Role::Owner).await?));
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}
```

4. Every handler that took `UrlPath(id): UrlPath<String>` now takes `access: ProjectAccess` and uses `let id = access.project.media_id.clone();` in place of `id`. `transcribe_item` and `speakers_item` keep their `(state, media_id)` signatures (library warm-up uses them) but call `media_dir`. Handlers: `transcribe`, `speakers`, `thumbnails`, `suggest`, `overdub`, `export`, `export_progress`.

   `overdub` additionally calls `access.require_edit()?` first. Synthesizing audio is an edit-side action.

5. `export`:

```rust
#[derive(Default, Deserialize)]
pub struct ExportRequest {
    /// `mp4` (video sources only), `mp3` or `wav`. Defaults by source kind.
    format: Option<String>,
}

/// `POST /api/projects/:id/export` — fold the log, plan the render, start
/// ffmpeg in the background and return a job id to poll.
pub async fn export(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<ExportRequest>>,
) -> AppResult<Json<ExportStarted>> {
    access.require_edit()?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let id = access.project.media_id.clone();
    let dir = media_dir(&state, &id)?;
    let meta = read_meta(&dir).await?;
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let edits = doc.edits;
    // …the existing body follows, with `req.edits` replaced by `edits`.
```

   Everything after (`format` match, `overdub_files`, `build_ffmpeg_args`, job spawn) is unchanged except `&req.edits` → `&edits` and `req.edits.len()` → `edits.len()`.

6. `export_progress`:

```rust
pub async fn export_progress(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    UrlPath((_, job_id)): UrlPath<(String, String)>,
) -> AppResult<Json<ExportJob>> {
    let id = access.project.media_id;
    // …unchanged lookup.
```

7. Update the module doc comment's first line to `//! HTTP handlers for a project's media. Each media item lives in `data/<id>/`:`.

- [ ] **Step 4: Library opens into a project**

In `server/src/library.rs`, `open` becomes:

```rust
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
    let role = if project.owner_id == user.id { Role::Owner } else { Role::Viewer };
    Ok(Json(summary(&state, &project, role).await?))
}
```

with imports `use crate::auth::CurrentUser; use crate::projects::{create_project, summary, Project, ProjectSummary, Role};`. (`role` for a non-owner is approximated as viewer here only because a member row must exist for the join to match; look it up properly: replace the `let role = …` line with a query on `project_members` for `(project.id, user.id)` parsed via `Role::parse`, defaulting to `Role::Viewer`.)

- [ ] **Step 5: Routes and startup**

In `app.rs` remove all `/api/media/…` routes and add:

```rust
        .route("/api/projects", get(projects::list).post(routes::upload))
        .route("/api/projects/{id}/transcribe", post(routes::transcribe))
        .route("/api/projects/{id}/suggest", post(routes::suggest))
        .route("/api/projects/{id}/thumbnails", post(routes::thumbnails))
        .route("/api/projects/{id}/speakers", post(routes::speakers))
        .route("/api/projects/{id}/overdub", post(routes::overdub))
        .route("/api/projects/{id}/export", post(routes::export))
        .route(
            "/api/projects/{id}/export/{job}/progress",
            get(routes::export_progress),
        )
```

(The existing `.route("/api/projects", get(projects::list))` line from Task 4 is replaced by the combined one.)

In `main.rs`, after building `state` and before `tokio::spawn(library::warm…)`:

```rust
    if let (Some(email), Some(password)) = (&state.config.admin_email, &state.config.admin_password) {
        let admin = match auth::create_user(&state.db, email, password, "admin").await {
            Ok(user) => Some(user.id),
            Err(e) if e.status() == axum::http::StatusCode::BAD_REQUEST => {
                let row: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
                    .bind(email.trim().to_ascii_lowercase())
                    .fetch_optional(&state.db)
                    .await?;
                row.map(|r| r.0)
            }
            Err(e) => anyhow::bail!("creating admin user: {e:?}"),
        };
        if let Some(id) = admin {
            projects::adopt_orphans(&state, &id).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
    }
```

`AppError` needs `impl std::fmt::Display` for `{e:?}`… it derives `Debug` already; `{e:?}` is enough.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p server` — Expected: all pass.

- [ ] **Step 7: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add server/src
git commit -m "server: move media routes under projects and export from the fold"
```

---

### Task 7: Client — session and login screen

**Files:**
- Modify: `client/src/types.ts`
- Modify: `client/src/api.ts`
- Create: `client/src/session.ts`
- Create: `client/src/components/Login.tsx`
- Modify: `client/src/styles.css`

**Interfaces:**
- Produces:
  - types: `User { id, email, displayName, color }`, `Role`, `ProjectSummary { id, title, role, media: Media, createdAt }`.
  - api: `fetchMe(): Promise<User>`, `fetchSetup(): Promise<{needsSetup: boolean}>`, `register(email, password, displayName)`, `login(email, password)`, `logout()`.
  - `useSession(): { user: User | null | undefined; setUser; signOut }` — `undefined` while loading.
  - `<Login needsSetup onSignedIn(user) />`.

- [ ] **Step 1: Types and API**

Append to `client/src/types.ts`:

```ts
export interface User {
  id: string;
  email: string;
  displayName: string;
  color: string;
}

export type Role = 'owner' | 'editor' | 'commenter' | 'viewer';

export interface ProjectSummary {
  id: string;
  title: string;
  role: Role;
  media: Media;
  createdAt: number;
}
```

Append to `client/src/api.ts`:

```ts
export function fetchMe(): Promise<User> {
  return request<User>('/api/me');
}

export function fetchSetup(): Promise<{ needsSetup: boolean }> {
  return request('/api/auth/setup');
}

export function register(email: string, password: string, displayName: string): Promise<User> {
  return postJson('/api/auth/register', { email, password, displayName });
}

export function login(email: string, password: string): Promise<User> {
  return postJson('/api/auth/login', { email, password });
}

export function logout(): Promise<void> {
  return request('/api/auth/logout', { method: 'POST' });
}
```

and extend the type import to include `ProjectSummary, User`.

- [ ] **Step 2: Session hook**

`client/src/session.ts`:

```ts
// Who is signed in. `undefined` until the first /api/me answers.

import { useCallback, useEffect, useState } from 'react';

import { ApiError, fetchMe, logout } from './api';
import type { User } from './types';

export function useSession() {
  const [user, setUser] = useState<User | null | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    fetchMe()
      .then((u) => {
        if (!cancelled) setUser(u);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        // 401 means "nobody"; anything else is still "nobody" but worth a log.
        if (!(err instanceof ApiError && err.status === 401)) console.error(err);
        setUser(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const signOut = useCallback(async () => {
    await logout().catch(() => undefined);
    setUser(null);
  }, []);

  return { user, setUser, signOut };
}
```

- [ ] **Step 3: Login component**

`client/src/components/Login.tsx`:

```tsx
import { useState, type FormEvent } from 'react';

import { login, register } from '../api';
import type { User } from '../types';

interface Props {
  /** No accounts exist yet: show "create the first account" instead of sign in. */
  needsSetup: boolean;
  onSignedIn: (user: User) => void;
}

export function Login({ needsSetup, onSignedIn }: Props) {
  const [mode, setMode] = useState<'login' | 'register'>(needsSetup ? 'register' : 'login');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const user =
        mode === 'login'
          ? await login(email, password)
          : await register(email, password, displayName);
      onSignedIn(user);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="login" onSubmit={onSubmit}>
      <h2>{mode === 'login' ? 'Sign in' : needsSetup ? 'Create the first account' : 'Create an account'}</h2>
      {mode === 'register' && (
        <label>
          Name
          <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} autoFocus />
        </label>
      )}
      <label>
        Email
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          autoFocus={mode === 'login'}
          required
        />
      </label>
      <label>
        Password
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          minLength={8}
          required
        />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>
        {mode === 'login' ? 'Sign in' : 'Create account'}
      </button>
      {!needsSetup && (
        <button
          type="button"
          className="ghost"
          onClick={() => setMode(mode === 'login' ? 'register' : 'login')}
        >
          {mode === 'login' ? 'Need an account?' : 'Have an account? Sign in'}
        </button>
      )}
    </form>
  );
}
```

Append to `client/src/styles.css` (match the existing variables and spacing used by `.dropzone-wrap`; read that block first):

```css
.login {
  max-width: 22rem;
  margin: 4rem auto;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}
.login label {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  font-size: 0.9rem;
}
.login input {
  font: inherit;
  padding: 0.5rem 0.6rem;
}
.login .error {
  color: var(--danger, #e0575b);
  margin: 0;
}
```

- [ ] **Step 4: Type-check and lint**

Run: `npm run build -w client && npx eslint . && npx prettier --check .` — Expected: clean (the new files are not yet used; unused-export warnings are not errors in this config).

- [ ] **Step 5: Commit**

```bash
git add client/src
git commit -m "client: add session hook, auth API and login screen"
```

---

### Task 8: Client — operations module and reducer sync

**Files:**
- Create: `client/src/ops.ts`
- Create: `client/src/ops.test.ts`
- Modify: `client/src/editor.ts`
- Modify: `client/src/editor.test.ts`
- Modify: `client/src/api.ts`

**Interfaces:**
- Produces:
  - types in `ops.ts`: `Op` (union mirroring `engine::Op`, camelCase, `kind` lowercase: `'cut' | 'overdub' | 'applycuts' | 'renamespeaker' | 'undo' | 'redo'`), `ClientOp = Op & { opId: string }`, `DocState { headSeq, edits, speakerNames, undoable, redoable }`.
  - `opForAction(state: EditorState, action: EditorAction): Op | null` — the operation an editing action corresponds to, or null for selection-only actions.
  - `newOpId(): string` — UUID v7 via `crypto.randomUUID()` fallback (v4 is fine for the unique index; v7 is a nicety and `crypto.randomUUID` produces v4 — document this).
  - reducer: `EditorState` loses `past`, gains `headSeq: number`, `speakerNames: string[]`, `undoable: number | null`, `redoable: number | null`; new action `{ type: 'sync'; doc: DocState }`; `'undo'` action removed from the reducer (it is server-only); `'renameSpeaker'` action added.
  - api: `fetchProject(id): Promise<{ project: ProjectSummary; doc: DocState }>`, `submitOps(id, ops: ClientOp[]): Promise<DocState>`, `listProjects()`, `uploadMedia` → returns `ProjectSummary`, `openLibraryClip` → `ProjectSummary`, and every `/api/media/${id}/…` helper becomes `/api/projects/${id}/…`; `exportMedia(id)` takes no edits.

- [ ] **Step 1: Write the failing tests**

`client/src/ops.test.ts`:

```ts
import { describe, expect, it } from 'vitest';

import { editorReducer, initialEditor, type EditorState } from './editor';
import { opForAction } from './ops';
import type { Word } from './types';

const words: Word[] = [
  { id: 'w0', text: 'thankful', start: 0, end: 0.91 },
  { id: 'w1', text: 'for', start: 0.91, end: 1.25 },
  { id: 'w2', text: 'you', start: 1.25, end: 1.59 },
  { id: 'w3', text: 'as', start: 2.0, end: 2.3 },
];

const loaded = editorReducer(initialEditor, { type: 'load', words, duration: 20 });

function select(state: EditorState, from: number, to = from): EditorState {
  const first = editorReducer(state, { type: 'select', index: from, extend: false });
  return to === from ? first : editorReducer(first, { type: 'select', index: to, extend: true });
}

describe('opForAction', () => {
  it('maps deleteSelection to a cut owning the trailing gap', () => {
    expect(opForAction(select(loaded, 1, 2), { type: 'deleteSelection' })).toEqual({
      kind: 'cut',
      start: 0.91,
      end: 2.0,
    });
  });

  it('returns null with no selection or for selection-only actions', () => {
    expect(opForAction(loaded, { type: 'deleteSelection' })).toBeNull();
    expect(opForAction(loaded, { type: 'select', index: 0, extend: false })).toBeNull();
    expect(opForAction(loaded, { type: 'clearSelection' })).toBeNull();
  });

  it('maps overdub to an overdub op over the selection', () => {
    const op = opForAction(select(loaded, 3), {
      type: 'overdub',
      text: 'hi',
      audioUrl: '/data/m/overdub-0.wav',
      audioDuration: 0.4,
    });
    expect(op).toEqual({
      kind: 'overdub',
      start: 2.0,
      end: 20,
      text: 'hi',
      audioUrl: '/data/m/overdub-0.wav',
      audioDuration: 0.4,
    });
  });

  it('maps applyCuts to applycuts with bare ranges', () => {
    expect(
      opForAction(loaded, {
        type: 'applyCuts',
        cuts: [
          { kind: 'cut', start: 1, end: 2 },
          { kind: 'cut', start: 3, end: 4 },
        ],
      }),
    ).toEqual({
      kind: 'applycuts',
      cuts: [
        { start: 1, end: 2 },
        { start: 3, end: 4 },
      ],
    });
    expect(opForAction(loaded, { type: 'applyCuts', cuts: [] })).toBeNull();
  });

  it('maps renameSpeaker', () => {
    expect(opForAction(loaded, { type: 'renameSpeaker', speaker: 1, name: ' Ada ' })).toEqual({
      kind: 'renamespeaker',
      speaker: 1,
      name: 'Ada',
    });
  });
});
```

In `client/src/editor.test.ts`, replace the `describe('undo', …)` block (whatever tests reference `past` or `{ type: 'undo' }`) with:

```ts
describe('sync', () => {
  it('replaces edits and metadata with the server document and clears the selection', () => {
    const state = editorReducer(select(loaded, 1), {
      type: 'sync',
      doc: {
        headSeq: 4,
        edits: [{ kind: 'cut', start: 1, end: 2 }],
        speakerNames: ['Ada'],
        undoable: 4,
        redoable: null,
      },
    });
    expect(state.edits).toEqual([{ kind: 'cut', start: 1, end: 2 }]);
    expect(state.headSeq).toBe(4);
    expect(state.speakerNames).toEqual(['Ada']);
    expect(state.undoable).toBe(4);
    expect(state.selection).toBeNull();
  });

  it('renameSpeaker updates names optimistically', () => {
    const state = editorReducer(loaded, { type: 'renameSpeaker', speaker: 2, name: 'Bob' });
    expect(state.speakerNames).toEqual(['', '', 'Bob']);
  });
});
```

Also update any existing test asserting `state.past` (search: `grep -n past client/src/editor.test.ts`) to assert on `edits` only.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run` — Expected: `ops.test.ts` fails to import `./ops`; `editor.test.ts` fails on the `sync` action type.

- [ ] **Step 3: Update the reducer**

In `client/src/editor.ts`:

```ts
export interface EditorState {
  words: Word[];
  duration: number;
  edits: Edit[];
  /** Display names by speaker index; '' means unnamed. */
  speakerNames: string[];
  /** Sequence number of the last server operation folded into `edits`. */
  headSeq: number;
  /** This user's next undo / redo target on the server, if any. */
  undoable: number | null;
  redoable: number | null;
  selection: Selection | null;
}

export type EditorAction =
  | { type: 'load'; words: Word[]; duration: number }
  /** The server's authoritative document replaces local edits. */
  | { type: 'sync'; doc: DocState }
  | { type: 'select'; index: number; extend: boolean }
  | { type: 'move'; delta: -1 | 1; extend: boolean; skipCut?: boolean }
  | { type: 'clearSelection' }
  | { type: 'deleteSelection' }
  | { type: 'overdub'; text: string; audioUrl: string; audioDuration: number }
  | { type: 'applyCuts'; cuts: CutEdit[] }
  | { type: 'renameSpeaker'; speaker: number; name: string };

export const initialEditor: EditorState = {
  words: [],
  duration: 0,
  edits: [],
  speakerNames: [],
  headSeq: 0,
  undoable: null,
  redoable: null,
  selection: null,
};
```

Import `type DocState` from `./ops`. `withEdits` becomes `{ ...state, edits, selection: null }` (no `past`). Replace the `'undo'` case with:

```ts
    case 'sync':
      return {
        ...state,
        edits: action.doc.edits,
        speakerNames: action.doc.speakerNames,
        headSeq: action.doc.headSeq,
        undoable: action.doc.undoable,
        redoable: action.doc.redoable,
        selection: null,
      };

    case 'renameSpeaker': {
      const speakerNames = [...state.speakerNames];
      while (speakerNames.length <= action.speaker) speakerNames.push('');
      speakerNames[action.speaker] = action.name.trim();
      return { ...state, speakerNames };
    }
```

- [ ] **Step 4: Write `ops.ts`**

```ts
// Operations: what the client tells the server it did. Mirrors
// engine/src/ops.rs. The reducer applies the same change optimistically;
// the server's fold (a `sync` action) is authoritative.

import { rangeForWords } from './editlist';
import type { EditorAction, EditorState } from './editor';
import { selectedRange } from './editor';
import type { Edit, Range } from './types';

export type Op =
  | { kind: 'cut'; start: number; end: number }
  | {
      kind: 'overdub';
      start: number;
      end: number;
      text: string;
      audioUrl: string;
      audioDuration: number;
    }
  | { kind: 'applycuts'; cuts: Range[] }
  | { kind: 'renamespeaker'; speaker: number; name: string }
  | { kind: 'undo'; targetSeq: number }
  | { kind: 'redo'; targetSeq: number };

export type ClientOp = Op & { opId: string };

export interface DocState {
  headSeq: number;
  edits: Edit[];
  speakerNames: string[];
  undoable: number | null;
  redoable: number | null;
}

/** A fresh operation id. The server dedupes replays by it. */
export function newOpId(): string {
  return crypto.randomUUID();
}

/** The operation an editing action sends, or null if it changes nothing shared. */
export function opForAction(state: EditorState, action: EditorAction): Op | null {
  switch (action.type) {
    case 'deleteSelection': {
      const range = selectedRange(state.selection);
      if (!range) return null;
      return { kind: 'cut', ...rangeForWords(state.words, range[0], range[1], state.duration) };
    }
    case 'overdub': {
      const range = selectedRange(state.selection);
      if (!range) return null;
      return {
        kind: 'overdub',
        ...rangeForWords(state.words, range[0], range[1], state.duration),
        text: action.text,
        audioUrl: action.audioUrl,
        audioDuration: action.audioDuration,
      };
    }
    case 'applyCuts':
      if (action.cuts.length === 0) return null;
      return { kind: 'applycuts', cuts: action.cuts.map(({ start, end }) => ({ start, end })) };
    case 'renameSpeaker':
      return { kind: 'renamespeaker', speaker: action.speaker, name: action.name.trim() };
    default:
      return null;
  }
}
```

- [ ] **Step 5: Update `api.ts`**

Replace every `` `/api/media/${id}/…` `` with `` `/api/projects/${id}/…` ``. Change:

```ts
export function uploadMedia(file: File): Promise<ProjectSummary> {
  const form = new FormData();
  form.append('file', file, file.name);
  return request<ProjectSummary>('/api/projects', { method: 'POST', body: form });
}

export function openLibraryClip(slug: string): Promise<ProjectSummary> {
  return request<ProjectSummary>(`/api/library/${encodeURIComponent(slug)}`, { method: 'POST' });
}

export function listProjects(): Promise<ProjectSummary[]> {
  return request<ProjectSummary[]>('/api/projects');
}

export function fetchProject(id: string): Promise<{ project: ProjectSummary; doc: DocState }> {
  return request(`/api/projects/${id}`);
}

export function submitOps(id: string, ops: ClientOp[]): Promise<DocState> {
  return postJson(`/api/projects/${id}/ops`, { ops });
}

export function exportMedia(id: string): Promise<ExportStarted> {
  return postJson(`/api/projects/${id}/export`, {});
}
```

with `import type { ClientOp, DocState } from './ops';`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `npx vitest run` — Expected: all pass. `npm run build -w client` will fail on `App.tsx` until Task 9; that is expected here.

- [ ] **Step 7: Commit**

```bash
git add client/src
git commit -m "client: add the operations module and sync the reducer with the server"
```

---

### Task 9: Client — project list, editor wiring, optimistic ops

**Files:**
- Create: `client/src/components/Projects.tsx`
- Modify: `client/src/App.tsx`
- Modify: `client/src/components/Toolbar.tsx`
- Modify: `client/src/components/Dropzone.tsx` (only if its props need the busy label; otherwise unchanged)
- Modify: `client/src/styles.css`

**Interfaces:**
- Consumes: `useSession`, `Login`, `fetchProject`, `submitOps`, `listProjects`, `opForAction`, `newOpId`.
- Produces: `<Projects items onOpen />`; `App` screens: loading → `Login` → home (`Dropzone` + `Projects`) → editor.
- Toolbar gains `canRedo: boolean` and `onRedo: () => void`; `canUndo` now means `editor.undoable !== null`.

- [ ] **Step 1: Projects list component**

`client/src/components/Projects.tsx`:

```tsx
import type { ProjectSummary } from '../types';
import { formatTime } from '../editlist';

interface Props {
  items: ProjectSummary[];
  onOpen: (project: ProjectSummary) => void;
}

export function Projects({ items, onOpen }: Props) {
  if (items.length === 0) return null;
  return (
    <section className="projects">
      <h2>Your projects</h2>
      <ul>
        {items.map((p) => (
          <li key={p.id}>
            <button type="button" onClick={() => onOpen(p)}>
              <span className="title">{p.title}</span>
              <span className="muted">
                {p.media.kind} · {formatTime(p.media.duration)} · {p.role}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
```

Append styles:

```css
.projects {
  max-width: 40rem;
  margin: 2rem auto 0;
}
.projects ul {
  list-style: none;
  padding: 0;
  margin: 0;
  display: grid;
  gap: 0.5rem;
}
.projects li button {
  width: 100%;
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  text-align: left;
  font: inherit;
  padding: 0.6rem 0.8rem;
}
```

- [ ] **Step 2: Toolbar redo**

In `client/src/components/Toolbar.tsx` add to `Props`: `canRedo: boolean; onRedo: () => void;` and next to the existing undo button:

```tsx
      <button type="button" onClick={onRedo} disabled={!canRedo} title="Redo (⇧⌘Z)">
        Redo
      </button>
```

Read the file first and match the existing button markup exactly (class names, `title` style).

- [ ] **Step 3: Rewrite `App.tsx` state flow**

Apply these edits to `client/src/App.tsx`:

1. Imports: add `fetchProject, listProjects, submitOps` from `./api`; `Login` and `Projects` components; `useSession` from `./session`; `newOpId, opForAction, type ClientOp` from `./ops`; type `ProjectSummary` (replace `Media` where it was the loaded item — keep `Media` for `project.media`).

2. Replace `const [media, setMedia] = useState<Media | null>(null);` with:

```tsx
  const { user, setUser, signOut } = useSession();
  const [needsSetup, setNeedsSetup] = useState(false);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const media = project?.media ?? null;
  const canEdit = project?.role === 'owner' || project?.role === 'editor';
```

3. Load the setup flag once and the project list whenever the user changes:

```tsx
  useEffect(() => {
    fetchSetup().then((s) => setNeedsSetup(s.needsSetup)).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!user) {
      setProjects([]);
      return;
    }
    let cancelled = false;
    listProjects()
      .then((list) => {
        if (!cancelled) setProjects(list);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [user, project]);
```

(`fetchSetup` import from `./api`.)

4. Replace `load`:

```tsx
  const load = useCallback(async (label: string, fetchSummary: () => Promise<ProjectSummary>) => {
    setLoadError(null);
    setExportState({ status: 'idle' });
    try {
      setBusy(label);
      const summary = await fetchSummary();
      setBusy('Transcribing');
      const [words, { doc }] = await Promise.all([
        transcribeMedia(summary.id),
        fetchProject(summary.id),
      ]);
      dispatch({ type: 'load', words, duration: summary.media.duration });
      dispatch({ type: 'sync', doc });
      setProject(summary);
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  }, []);

  const onOpenProject = useCallback(
    (p: ProjectSummary) => load(`Opening ${p.title}`, () => Promise.resolve(p)),
    [load],
  );
```

`onFile` and `onLibraryClip` are unchanged apart from the type (both helpers now return `ProjectSummary`). Wherever `media.id` was passed to an API helper (`suggestEdits`, `fetchSpeakers`, `fetchThumbnails` in `Player`, `synthesizeOverdub`, `exportMedia`, `exportProgress`), pass `project.id` instead. `Player` receives `media={media}` still (it reads `url`/`kind`); check whether it calls `fetchThumbnails(media.id)` — if so, add a `projectId` prop and pass `project.id`.

5. `goHome` sets `setProject(null)` instead of `setMedia(null)`; the history effect keys on `project.id`.

6. Optimistic edit dispatch — add after `const [editor, dispatch] = useReducer(...)`:

```tsx
  // The server's last confirmed document, for rolling back a rejected op.
  const confirmed = useRef<DocState | null>(null);

  /**
   * Apply an editing action locally, send its operation, and settle on the
   * server's fold. A rejected operation rolls back to the last confirmed doc.
   */
  const edit = useCallback(
    (action: EditorAction) => {
      if (!project) return;
      const op = opForAction(editor, action);
      dispatch(action);
      if (!op) return;
      if (!canEdit) return;
      const clientOp: ClientOp = { ...op, opId: newOpId() };
      submitOps(project.id, [clientOp])
        .then((doc) => {
          confirmed.current = doc;
          dispatch({ type: 'sync', doc });
        })
        .catch((err: unknown) => {
          setLoadError(err instanceof Error ? err.message : String(err));
          if (confirmed.current) dispatch({ type: 'sync', doc: confirmed.current });
        });
    },
    [project, editor, canEdit],
  );

  const undoRedo = useCallback(
    (kind: 'undo' | 'redo') => {
      if (!project) return;
      const targetSeq = kind === 'undo' ? editor.undoable : editor.redoable;
      if (targetSeq === null) return;
      submitOps(project.id, [{ kind, targetSeq, opId: newOpId() }])
        .then((doc) => {
          confirmed.current = doc;
          dispatch({ type: 'sync', doc });
        })
        .catch((err: unknown) => setLoadError(err instanceof Error ? err.message : String(err)));
    },
    [project, editor.undoable, editor.redoable],
  );
```

with `import type { DocState } from './ops'` and `type EditorAction` from `./editor`. In `load`, after the `sync` dispatch, set `confirmed.current = doc;`.

7. Route every editing dispatch through `edit`: the keyboard handler's `deleteSelection` → `edit({ type: 'deleteSelection' })`; ⌘Z → `undoRedo('undo')`; add ⇧⌘Z → `undoRedo('redo')`; Toolbar `onDelete`, `onRemoveFillers`, `onTightenPauses` → `edit(...)`; `onOverdubSubmit` → `edit({ type: 'overdub', … })`; `canUndo={editor.undoable !== null}`, `canRedo={editor.redoable !== null}`, `onUndo={() => undoRedo('undo')}`, `onRedo={() => undoRedo('redo')}`. Selection actions (`select`, `move`, `clearSelection`) keep calling `dispatch` directly.

8. Speaker names: delete `speakerNames` state, `readSpeakerNames`, `writeSpeakerNames`. `onRenameSpeaker` becomes `(speaker, name) => edit({ type: 'renameSpeaker', speaker, name })` and `Transcript` gets `speakerNames={editor.speakerNames}`.

9. Non-editors: pass `readOnly={!canEdit}` to `Toolbar` and disable its editing buttons when set (add the prop; `Export` stays enabled only for editors since the route requires edit — disable it too). Keep the change minimal: one `disabled={readOnly || …}` per editing button.

10. Render:

```tsx
  if (user === undefined) return <div className="app"><p className="muted">Loading…</p></div>;
  if (!user) return <div className="app"><Login needsSetup={needsSetup} onSignedIn={setUser} /></div>;
```

before the existing `return`, and in the header add after the tagline:

```tsx
        <span className="spacer" />
        <span className="muted">{user.displayName}</span>
        <button type="button" className="ghost" onClick={signOut}>Sign out</button>
```

(dropping the existing `<span className="spacer" />` so there is one). On the home screen render `<Projects items={projects} onOpen={onOpenProject} />` under the `Dropzone`.

- [ ] **Step 4: Type-check, lint, test**

Run: `npm run build -w client && npm run lint && npx vitest run` — Expected: clean. Fix any `Player`/`Transcript` prop mismatches the compiler reports by passing `project.id` where a project-scoped API call needs it.

- [ ] **Step 5: Manual smoke test**

Run `npm run dev`, open http://localhost:5174:

1. First visit shows "Create the first account"; register.
2. Open a library clip; delete a word; reload the page — the cut is still there.
3. Open a second browser (or private window), register `bob`, add bob as `viewer` via `curl -b … -X POST /api/projects/<id>/members` (the UI for members arrives with the realtime plan). Bob sees the project read-only.
4. ⌘Z / ⇧⌘Z undo and redo only your own change.
5. Export renders the server's fold.

- [ ] **Step 6: Commit**

```bash
git add client/src
git commit -m "client: sign in, list projects and send every edit as an operation"
```

---

### Task 10: README and configuration notes

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the README**

In the "How it works" list, change step 2's last sentence to: "Every change is an operation appended to the project's log in SQLite; the edit list is the fold of that log, and undo appends an `undo` targeting your own operation." Update the ASCII diagram's server column to list:

```
 │  POST /api/auth/{register,login,logout}, GET /api/me   │
 │  GET/POST /api/projects           (list, upload)       │
 │  GET  /api/projects/:id           (media + fold)       │
 │  POST /api/projects/:id/ops       (append to log)      │
 │  POST /api/projects/:id/{transcribe,overdub,export…}   │
```

and add `SQLite (sqlx)` beneath `engine/`. In "Setup", add a subsection:

```markdown
### Accounts

The first visit asks you to create an account. To create an admin non-interactively and
adopt any media already under `DATA_DIR`, set `ADMIN_EMAIL` and `ADMIN_PASSWORD` before
starting the server. The database lives at `$DATA_DIR/type-n-stitch.db`; override with
`DATABASE_URL`. Registration is open to anyone who can reach the server — put it behind
your own network or proxy.
```

- [ ] **Step 2: Full verification**

Run: `npm test && npm run lint` — Expected: both clean.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "README: document accounts, projects and the operation log"
```

---

## Self-review

**Spec coverage (Foundation scope):**
- Accounts, sessions, roles → Tasks 3, 4. ✔
- SQLite persistence for users, projects, membership, op log → Tasks 1, 4, 5. ✔
- Server-authoritative edit list folded from the log; cache; invalidation → Tasks 2, 5. ✔
- `op_id` idempotence → Tasks 1, 5, 8. ✔
- Undo/redo own ops only; `undone_by` bookkeeping → Tasks 2, 5. ✔
- Extractors authorize in signature; 401/403/404 distinct → Tasks 3, 4, 6. ✔
- Malformed op → 400 with index; batch is one transaction → Task 5. ✔
- Media routes under projects; export folds server-side → Task 6. ✔
- Orphan adoption at startup / first registration → Tasks 4, 6. ✔
- Client: login, project list, optimistic apply + reconcile, rollback on rejection → Tasks 7–9. ✔
- Speaker names move from localStorage to `RenameSpeaker` ops → Tasks 2, 8, 9. ✔
- Not in this plan (by spec): websocket, presence, comments, versions. Member-management UI is also deferred to the realtime plan; the API exists.

**Deviation noted:** spec sketches `RenameSpeaker { id, name }`; the client only has speaker indices, so the op uses `speaker: u32`.

**Type consistency:** `DocState` fields (`headSeq`, `edits`, `speakerNames`, `undoable`, `redoable`) match between `server/src/ops.rs` (camelCase serde) and `client/src/ops.ts`; `ClientOp` flattens `Op` with `opId` on both sides; `Role` strings match the SQL `CHECK`; `ProjectSummary` fields match `client/src/types.ts`.
