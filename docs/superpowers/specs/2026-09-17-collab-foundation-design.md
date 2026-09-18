# Collaborative foundation and realtime co-editing

Date: 2026-09-17
Status: approved design, ready for an implementation plan

## Problem

type-n-stitch is a single-user local tool. A project's edit list lives only in
browser state (`client/src/editor.ts`); the server stores media and words on
disk under `DATA_DIR` and exposes stateless REST routes keyed by a media id.
There are no accounts, no persistence of edits, and no way for a second person
to see or change what the first person is doing.

The goal is a self-hosted collaborative editor and video review tool: real
accounts, projects that are shared, an edit list that the server owns, and live
co-editing with presence.

## Scope

In scope:

- Accounts, sessions, and role-based access to projects.
- SQLite persistence for users, projects, membership, and an append-only log of
  edit operations.
- A server-authoritative edit list, folded from that log.
- A WebSocket transport carrying operations and presence, with catch-up and
  reconnect.
- Per-user undo.

Out of scope, deliberately, and planned as later projects:

- Timestamped comments and threads.
- Frame annotations.
- Approvals and review status.
- Named versions, compare, and restore.

The schema and the operation log are designed so each of those attaches without
a redesign: comments anchor to a sequence number, versions name a sequence
number, and annotations are rows that reference a project and a time.

## Approach

A project is already `source + Vec<Edit>`, and an `Edit` (`engine/src/types.rs`)
is a range operation over immutable source media. That makes an append-only
operation log the natural representation:

```
edit_ops(project_id, seq, author_id, op json, undone_by, created_at)
```

Project state is the fold of its operations. The `seq` column is the version
number every later subsystem needs. The server keeps a cached fold per project
for speed; the cache is derived state and can always be rebuilt from the log.

A CRDT was rejected. Edits are ranges over immutable source, not free text, and
server sequencing already converges. It would be cost without benefit.

A single mutable snapshot with optimistic concurrency was rejected. It is less
code today, but realtime, history, and per-user undo would each have to rebuild
what the log provides.

## Data model

SQLite via `sqlx`, one file next to `DATA_DIR`. Media files, synthesized WAVs,
and exports stay on disk; the database holds metadata and operations only.

- `users(id, email unique, password_hash, display_name, color, created_at)`
- `sessions(id, user_id, expires_at, created_at)`
- `projects(id, media_id, owner_id, title, created_at)`
- `project_members(project_id, user_id, role)`
- `edit_ops(project_id, seq, author_id, op json, undone_by, created_at)`
  with `primary key (project_id, seq)`

Roles are `owner`, `editor`, `commenter`, and `viewer`. The foundation enforces
all four even though commenting arrives in a later project.

## Operations

A new `engine/src/ops.rs` defines the operation type and the fold:

```rust
pub enum Op {
    Cut { start: f64, end: f64 },
    Overdub { start: f64, end: f64, text: String, audio_url: String, audio_duration: f64 },
    ApplyCuts { cuts: Vec<Range> },
    RenameSpeaker { id: String, name: String },
    Undo { target_seq: i64 },
}

pub fn fold(ops: &[SeqOp]) -> Vec<Edit>;
```

`fold` applies the same normalization rules as `engine/src/editlist.rs`, and an
operation marked undone is treated as absent. `client/src/editlist.ts` remains a
preview mirror; the engine is the source of truth for export.

`Undo { target_seq }` targets the submitting user's most recent operation that
has not already been undone. The server validates that the target belongs to
that user, so one person's undo cannot remove another person's cut.

## Authentication and authorization

Email and password, hashed with argon2id. Login creates a `sessions` row and
sets an opaque token in an HttpOnly, SameSite=Lax cookie. An axum middleware
resolves the cookie to a `CurrentUser` extractor; a second extractor resolves a
project id and the current user to a `ProjectRole`. Every project route
declares its requirement in its signature rather than checking in its body.

A WebSocket upgrade is an ordinary HTTP request, so both extractors run before
the socket exists. Authentication never happens as a first message over an
already-open socket.

On first boot the server creates a `projects` row for each existing media
directory under `DATA_DIR`, owned by a bootstrap user taken from `ADMIN_EMAIL`
and `ADMIN_PASSWORD`, or created by a first-run registration flow when those are
unset. Operation logs start empty, because today's edits never left the browser.
No files move and nothing is re-transcribed.

## HTTP API

- `POST /api/auth/register`, `POST /api/auth/login`, `POST /api/auth/logout`
- `GET /api/me`
- `GET /api/projects`, `POST /api/projects`
- `GET /api/projects/:id` — media, words, folded edits, head sequence
- `POST /api/projects/:id/ops` — submit operations, returns the new head
- `POST /api/projects/:id/members`, `DELETE /api/projects/:id/members/:user_id`

The existing transcribe, speakers, thumbnails, suggest, overdub, and export
routes move under a project and gain the role extractor. Export folds the
operation log server-side and feeds `build_ffmpeg_args`, so the client stops
being the source of truth for what gets rendered.

## Realtime transport

`GET /api/projects/:id/ws` upgrades to a WebSocket. `viewer` and `commenter`
connections are read-only; the server rejects their operation frames rather than
relying on the client to hide the control.

`AppState` gains `hubs: Mutex<HashMap<ProjectId, Weak<Hub>>>`. A `Hub` is
created on first subscriber and dropped when the last one leaves. It holds a
`tokio::sync::broadcast::Sender<ServerMsg>` for fan-out and a
`Mutex<HashMap<ConnId, PresenceState>>` that is never written to SQLite.

Each connection splits into a reader task, which turns client frames into
operations, and a writer task, which forwards broadcast messages to the socket.
A client that lags the broadcast channel receives `RecvError::Lagged`; the
server responds with a full resync instead of closing the connection. A ping
frame every 30 seconds detects dead sockets.

### Protocol

Client to server:

```
{t:"ops", ops:[...], baseSeq}
{t:"presence", playhead, selection, playing}
{t:"ping"}
```

Server to client:

```
{t:"hello", headSeq, edits, peers:[...], you:{id, name, color}}
{t:"ops", seq, authorId, ops:[...]}
{t:"presence", connId, state}
{t:"left", connId}
{t:"resync", headSeq, edits}
{t:"error", code, detail}
```

Operation broadcasts go to every subscriber including the sender, so the sender
learns the authoritative sequence number its optimistic state should settle on.

### Catch-up and reconnect

The client remembers `headSeq`. On connect it sends its last known sequence; the
server replays `edit_ops WHERE seq > lastSeq` when the gap is small and sends a
full `resync` when it is not. Reconnect uses exponential backoff with jitter.
While disconnected the client keeps editing optimistically and flushes queued
operations on reconnect; the server sequences them at their arrival position,
not their origin time.

### One operation path

The socket `ops` frame and `POST /api/projects/:id/ops` both call a single
`apply_ops(state, project, user, ops)`. The REST route remains for tests,
scripts, and clients without a socket. One code path, two doors.

## Presence

Presence carries a user id, display name, color, playhead position, selected
word range, and whether that peer is playing. The client throttles presence
frames to roughly ten per second. Nothing about presence is persisted.

The UI renders peer playheads as colored scrubber markers, peer selections as a
tinted underline in the transcript, and connected peers as an avatar row.

Playback stays local to each person. Follow mode and presenter mode are out of
scope; follow mode can later be built entirely on the client from presence
frames already in the protocol, with no server change.

## Client changes

- A login screen and a project list; `App.tsx` loads a project rather than a
  media id.
- `editorReducer` still applies edits locally, now marked optimistic, and sends
  the corresponding operation through a socket client module.
- A reconcile step replaces optimistic state with the authoritative fold when a
  broadcast or resync arrives.
- `usePlayback.ts` is unchanged apart from reporting the playhead for presence.

## Data flow for one edit

1. The user deletes a run of words; the reducer applies it optimistically.
2. The client sends `{t:"ops", ops:[{kind:"cut", start, end}], baseSeq}`.
3. The server resolves the session and role, appends each operation with the
   next sequence number in one transaction, and invalidates the cached fold.
4. The server broadcasts `{t:"ops", seq, authorId, ops}` to every subscriber.
5. Each client, sender included, applies the broadcast and settles on `headSeq`.

## Error handling

- Unauthenticated requests return 401 and the client routes to login.
- Authenticated requests with an insufficient role return 403, kept distinct
  from 404 so a mistyped link and a permissions problem read differently.
- A malformed operation, such as a range outside the media duration or an
  overdub whose audio is missing, returns 400 naming the operation's index; the
  client rolls back that optimistic step alone.
- Appending a batch of operations is one SQLite transaction, so a crash leaves
  no partial batch.
- The fold cache is derived state; losing or corrupting it costs a re-fold, not
  data.
- `AppError` in `server/src/error.rs` gains `Unauthorized`, `Forbidden`, and
  `Conflict`.

## Testing

- `engine`: unit tests for `fold` covering overlapping cuts, an overdub inside a
  later cut, undo, undo of an undo, two authors interleaved, and an operation
  referencing an already-removed range.
- `server`: `sqlx` tests against a temporary database for register, login,
  logout, session expiry, and each role against each project route, asserting
  401, 403, and 404 are distinguished; a hub test asserting that an operation
  from one connection reaches another, that a read-only role's operation frame
  is rejected, and that a lagged subscriber receives a resync.
- `client`: vitest for optimistic apply and reconcile, a rejected operation
  rolling back exactly one step, and the reconnect queue flushing in order.
- `npm test` (`cargo test && vitest run`) remains the whole suite.

## Risks

- The fold cost grows with log length. Mitigated by the cached fold, and by
  compaction into a snapshot row if a log ever grows large enough to matter.
- Optimistic local state can diverge from the server fold under unusual
  interleavings. Mitigated by making the broadcast authoritative and by the
  resync path, which is the same code as first connect.
- SQLite write contention under many simultaneous editors. Acceptable for
  self-hosted use; WAL mode is enabled, and moving to Postgres is a connection
  string change plus dialect fixes.
