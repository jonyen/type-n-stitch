# Collaborative editing and video review

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

The work is four projects, each with its own implementation plan, built in
order because each depends on the one before it:

1. **Foundation.** Accounts, sessions, and role-based access. SQLite
   persistence for users, projects, membership, and an append-only log of edit
   operations. A server-authoritative edit list folded from that log.
2. **Realtime.** A WebSocket transport carrying operations and presence, with
   catch-up, reconnect, idempotent replay, and per-user undo and redo.
3. **Review.** Comment threads anchored to source time and word ranges,
   @mentions, resolution, and frame annotations drawn over a paused frame.
4. **Workflow.** Review status with sign-offs, named versions, compare, and
   restore.

Out of scope: follow mode and presenter mode (shared playback), simultaneous
editing of comment text, and running more than one server instance. Each is
discussed where the design leaves room for it.

One principle runs through every project: **everything anchors to source
time, which is immutable.** Cuts and overdubs change the output timeline, never
the source timeline, so comments, annotations, and versions never need
re-anchoring when the edit list changes.

## Approach

A project is already `source + Vec<Edit>`, and an `Edit` (`engine/src/types.rs`)
is a range operation over immutable source media. That makes an append-only
operation log the natural representation:

```
edit_ops(project_id, seq, op_id, author_id, op json, undone_by, created_at)
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
- `edit_ops(project_id, seq, op_id, author_id, op json, undone_by, created_at)`
  with `primary key (project_id, seq)` and `unique (project_id, op_id)`

Roles are `owner`, `editor`, `commenter`, and `viewer`. The foundation enforces
all four even though commenting arrives in a later project.

`op_id` is a client-generated UUID v7. If a client replays an operation after a
reconnect, the unique index lets the server return the existing sequence number
instead of applying it twice. This is the same reason Figma generates object ids
on the client: identity must not depend on a round trip.

Tables added by later projects are listed in their own sections.

## Operations

A new `engine/src/ops.rs` defines the operation type and the fold:

```rust
pub enum Op {
    Cut { start: f64, end: f64 },
    Overdub { start: f64, end: f64, text: String, audio_url: String, audio_duration: f64 },
    ApplyCuts { cuts: Vec<Range> },
    RenameSpeaker { id: String, name: String },
    Undo { target_seq: i64 },
    Redo { target_seq: i64 },
}

pub fn fold(ops: &[SeqOp]) -> Vec<Edit>;
```

`fold` applies the same normalization rules as `engine/src/editlist.rs`, and an
operation marked undone is treated as absent. `client/src/editlist.ts` remains a
preview mirror; the engine is the source of truth for export.

`Undo { target_seq }` targets the submitting user's most recent operation that
has not already been undone. `Redo { target_seq }` targets the submitting
user's most recent `Undo` and re-enables the operation it removed. The server
validates that the target belongs to that user, so one person's undo cannot
remove another person's cut. The client keeps per-user undo and redo stacks.
Because both only ever touch the user's own operations, Figma's invariant
holds: undoing many steps and redoing back to the present leaves the document
unchanged, however much collaborators did in between.

Every operation carries the client-generated `op_id` described above.

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

Fan-out sits behind a `Bus` trait:

```rust
trait Bus {
    fn publish(&self, project: ProjectId, msg: ServerMsg);
    fn subscribe(&self, project: ProjectId) -> impl Stream<Item = ServerMsg>;
}
```

The only implementation in this work is in-process: `AppState` gains
`hubs: Mutex<HashMap<ProjectId, Weak<Hub>>>`, a `Hub` is created on first
subscriber and dropped when the last one leaves, and it holds a
`tokio::sync::broadcast::Sender<ServerMsg>` for fan-out and a
`Mutex<HashMap<ConnId, PresenceState>>` that is never written to SQLite.

The trait exists so that running more than one server instance later is a
second `Bus` implementation, for example Postgres `LISTEN/NOTIFY`, with no
call-site changes. Multiple instances also require moving off SQLite, which is
single-writer, so that work is one change rather than two. This is the
boundary Figma draws with a separate process per document; here it is a trait.

Comment, review, and version events from later projects are published on the
same bus. They are not operations and are not in the log; they are ordinary
rows, stored first and then broadcast, which is also how Figma keeps comments
out of its multiplayer layer.

Each connection splits into a reader task, which turns client frames into
operations, and a writer task, which forwards broadcast messages to the socket.
A client that lags the broadcast channel receives `RecvError::Lagged`; the
server responds with a full resync instead of closing the connection. A ping
frame every 30 seconds detects dead sockets.

### Protocol

Client to server:

```
{t:"ops", ops:[...], baseSeq}
{t:"presence", playhead, selection, caret, playing}
{t:"ping"}
```

Server to client:

```
{t:"hello", headSeq, edits, peers:[...], you:{id, name, color}}
{t:"ops", seq, authorId, ops:[...]}
{t:"presence", connId, state}
{t:"left", connId}
{t:"resync", headSeq, edits}
{t:"comment", event, comment}
{t:"review", event, review}
{t:"version", event, version}
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
word range, a caret word index for peers with no selection, and whether that
peer is playing. The client throttles presence
frames to roughly ten per second. Nothing about presence is persisted.

The UI renders peer playheads as colored scrubber markers, peer selections as a
tinted underline in the transcript, peer carets as a thin colored bar between
words, and connected peers as an avatar row.

Playback stays local to each person. Follow mode and presenter mode are out of
scope; follow mode can later be built entirely on the client from presence
frames already in the protocol, with no server change.

## Comments and annotations

Schema:

- `comments(id, project_id, author_id, start, end, word_start_id, word_end_id,
body, at_seq, resolved_by, resolved_at, created_at)`
- `comment_replies(id, comment_id, author_id, body, created_at)`
- `annotations(id, comment_id, time, shapes json, created_at)`
- `mentions(target_kind, target_id, user_id)`

A comment anchors to a source-time range and, when it was made from the
transcript, to a pair of word ids. The word ids tell the transcript where to
draw a marker; the time range tells the scrubber. If the range is later cut,
the comment remains and renders with an "on cut content" badge, because
reviewers often comment on exactly the thing they want removed. `at_seq`
records the head sequence at creation so the workflow project can show what the
edit looked like when the comment was written.

A comment may carry one annotation: a list of vector shapes (`pen` polyline,
`arrow`, `rect`, `text`) in normalized 0 to 1 coordinates so they survive any
player size, tied to one source `time`. The client draws them on a canvas
overlay when the player is paused within one frame of that time and hides them
while playing. Annotations are never rendered into the export.

Mutations are REST, under `/api/projects/:id/comments`, and are stored as rows
before being published on the bus as `{t:"comment", event, comment}`. Roles
`commenter` and above may create; the author or an `owner` may resolve or
delete. Comment bodies are last-writer-wins, the same limitation Figma accepts.

UI: a comment rail beside the transcript, markers on the scrubber, click to
seek and open the thread, `@` autocompletes project members, an unresolved
count in the header, and an annotation toolbar that appears when the player is
paused on video.

## Approvals and versions

Schema:

- `versions(id, project_id, name, seq, created_by, created_at)`
- `reviews(project_id, status, requested_by, requested_at)` with status one of
  `draft`, `in_review`, `changes_requested`, `approved`
- `review_signoffs(id, project_id, user_id, decision, note, at_seq, created_at)`

A version is a name attached to a sequence number. Nothing is copied; the log
already holds the state. Restore appends the operations that bring the current
fold back to the named version's fold, computed as a diff, so a restore is
itself undoable and appears in history like any other change.

Compare folds the log at two sequence numbers, diffs the two edit lists, and
renders the transcript with added cuts, removed cuts, and changed overdubs
distinguished by color. It is a pure client function over two folds and is
unit-tested as such.

A sign-off records the head sequence it approved, so "approved" means "approved
this fold". Any operation appended after an approval moves the status back to
`in_review` automatically, and the UI names the version that was approved.
Status and sign-off changes are published on the bus as `{t:"review", …}`;
version creation as `{t:"version", …}`.

## Client changes

- A login screen and a project list; `App.tsx` loads a project rather than a
  media id.
- `editorReducer` still applies edits locally, now marked optimistic, and sends
  the corresponding operation through a socket client module.
- A reconcile step replaces optimistic state with the authoritative fold when a
  broadcast or resync arrives.
- `usePlayback.ts` is unchanged apart from reporting the playhead for presence.
- A comment rail, annotation canvas overlay, review status header, and a
  version list with compare and restore, each arriving with its project.

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
  rolling back exactly one step, the reconnect queue flushing in order and
  replayed operations deduplicating by `op_id`, annotation coordinates
  round-tripping through a resize, and the version compare diff.
- `server`, later projects: role checks on comment, review, and version routes;
  a sign-off followed by an operation flipping status to `in_review`; restore
  producing a fold equal to the named version's fold.
- `npm test` (`cargo test && vitest run`) remains the whole suite.

## Relationship to Figma's multiplayer design

The architecture follows the same reasoning as Figma's: a central server orders
every change, so neither operational transformation nor a full CRDT is needed.
The differences are deliberate. Figma stores a mutable property map and
discards history because its documents are large; this design stores an
append-only operation log because an edit list is small and history buys
versions, comments anchored to a sequence number, and per-user undo for free.
Figma needs fractional indexing and cycle rejection for its object tree; this
design has no tree, because every operation is a range on a fixed source
timeline. Both keep comments and membership out of the multiplayer channel and
in ordinary tables.

## Risks

- The fold cost grows with log length. Mitigated by the cached fold, and by
  compaction into a snapshot row if a log ever grows large enough to matter.
- Optimistic local state can diverge from the server fold under unusual
  interleavings. Mitigated by making the broadcast authoritative and by the
  resync path, which is the same code as first connect.
- SQLite write contention under many simultaneous editors. Acceptable for
  self-hosted use; WAL mode is enabled, and moving to Postgres is a connection
  string change plus dialect fixes.
