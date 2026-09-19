# MCP server: an agent that edits alongside you

Date: 2026-09-19
Status: approved design, ready for an implementation plan

## Problem

Every edit in type-n-stitch is an operation over HTTP, every project fans
its state out live, and every collaborator is a peer with a cursor. Nothing
lets an AI agent take part. The goal is a demo where Claude Code opens a
project, reads the transcript, cuts the fillers, drops in a title, and
exports — while the people with the project open watch its cursor move and
its edits land.

## Approach

An MCP endpoint inside the existing axum server, using the `rmcp` crate's
streamable-HTTP transport at `/mcp`. Each MCP session authenticates with a
per-user API token, acts as a bot user owned by that person, and joins a
project's hub as a peer exactly like a browser tab. Tools call the same
`apply_ops`, suggestion and export code the HTTP routes use, so validation,
fan-out and history are unchanged.

Rejected: a separate stdio server over the HTTP API (two processes, no way to
be a live peer; a stdio shim can proxy to `/mcp` later if Claude Desktop
matters). Rejected: acting directly as the token's owner (edits and undo
targets would be indistinguishable from the person's own).

## API tokens

`api_tokens(id, user_id, token_hash, label, created_at, last_used_at)`.

- `POST /api/tokens { label }` (cookie session) mints 32 random bytes,
  stores their SHA-256, and returns the plaintext token once as
  `tns_<base64url>`.
- `GET /api/tokens` lists `{ id, label, createdAt, lastUsedAt }`;
  `DELETE /api/tokens/:id` revokes.
- `CurrentUser` accepts `Authorization: Bearer <token>` as well as the
  session cookie; a Bearer hit updates `last_used_at`.

Client: the avatar menu gains "Connect an agent", a dialog that mints a token
with a label, shows the plaintext once with the copy-pasteable
`claude mcp add --transport http type-n-stitch <origin>/mcp --header
"Authorization: Bearer <token>"` line, and lists existing tokens with a
revoke button.

## The bot user

The first time a token is used, the server creates (if missing) a bot user
owned by the token's owner: `users` gains `owner_id: Option<String>` (null
for people); the bot's `display_name` is "Claude", its email
`agent+<owner-id>@local`, its colour distinct from the owner's. Bots cannot
log in (no password; `login` rejects rows with an owner).

When the agent opens a project, the server adds the bot as a member with the
owner's role capped at `editor` (`owner`/`editor` → `editor`, `commenter` →
`commenter`, `viewer` → `viewer`). Edits are attributed to the bot, so its
undo/redo targets are its own and history shows who did what. The project
list and member list show bots with a small "agent" badge.

## MCP surface

Tools only; no resources or prompts in this work.

| Tool                                                   | Effect                                                                                                                                                                                                                         |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `list_projects()`                                      | Projects the owner can open, with role and duration.                                                                                                                                                                           |
| `open_project(project_id)`                             | Joins the hub as the bot (presence), ensures membership, returns title, duration, output duration, cut/overdub counts, speaker names and the transcript as `[{ i, text, start, end, speaker, status }]` with `status` in `kept | cut | overdub`. |
| `get_transcript()`                                     | The same transcript for the open project, after edits.                                                                                                                                                                         |
| `find(text)`                                           | Whole-word, case- and punctuation-insensitive matches as `[{ from, to }]` word ranges.                                                                                                                                         |
| `look_at(from, to)`                                    | Presence only: the agent's selection moves to that range.                                                                                                                                                                      |
| `cut(from, to)`                                        | A cut over the words' owned range (same rule as the transcript's Delete).                                                                                                                                                      |
| `remove_fillers()`, `tighten_pauses()`                 | The engine's suggestions applied as one `applycuts`.                                                                                                                                                                           |
| `overdub(from, to, text)`                              | Synthesises with VoiceStudio and appends the overdub.                                                                                                                                                                          |
| `add_title(after, text, subtitle?, style?, duration?)` | `after` is a word index (`-1` = before the first word); `at` = that word's end (or 0).                                                                                                                                         |
| `add_caption(from, to, text, position?)`               | Over the words' range.                                                                                                                                                                                                         |
| `set_transition(kind)`                                 | `none` or `dip`.                                                                                                                                                                                                               |
| `undo()`, `redo()`                                     | The bot's own targets.                                                                                                                                                                                                         |
| `export(format?)`                                      | Starts the job, polls until done (10 min cap), returns `{ url, duration, bytes }`.                                                                                                                                             |

Every editing tool first moves the bot's presence to the affected range,
waits 400 ms so the cursor is visible to people watching, then applies the
operation and returns the new stats plus the words it touched. `find`,
`look_at` and `get_transcript` do not edit. Tool errors return the server's
error text verbatim (the same 400 messages the browser sees).

## Transport and sessions

`rmcp`'s `StreamableHttpService` is nested at `/mcp`. A tower layer runs
before it: it reads `Authorization: Bearer`, resolves the token to its owner
(401 otherwise), ensures the bot user, and stores both in request extensions.
The service factory builds one `McpSession` per rmcp session holding the
owner, the bot, the open project (if any), the hub `Subscription`, and a
connection id. Closing the session, or 10 minutes idle, drops the
subscription so the hub announces `left`.

Presence goes through `bus.update_presence` and `publish`, the same calls
`ws.rs` makes. Edits go through `ops::apply_ops(state, project, bot, ops)`,
which already publishes the fold to every subscriber; the session ignores its
own subscription's frames (it only holds the subscription to exist as a
peer).

## Error handling

- Missing or unknown token → 401 before any MCP traffic; revoked token → 401
  on the next request.
- A project the bot cannot edit → the tool returns the same 403 text the HTTP
  route would.
- Any rejected operation → the 400 message with the operation index, as the
  browser gets.
- `export` failure → the job's error message; polling past 10 minutes returns
  the job id so a later call can resume.

## Testing

- Server: mint/list/revoke tokens; Bearer on `/api/me` succeeds, revoked
  token fails; bot user created once per owner, cannot log in; membership
  capped at editor; each tool called directly against `test_util::state()`
  asserting the op landed (fold), presence moved (`bus.peers`), and the
  returned transcript status; `find` normalisation; `export` polling.
- Integration: an rmcp client over streamable HTTP with a Bearer header calls
  `open_project` then `cut`; a WebSocket peer sees the bot's `presence` and
  the `doc` broadcast.
- Client: vitest for the `claude mcp add` command string helper; the dialog
  itself is verified by build + a browser check.

## Documentation

README gains an "Agents" section: mint a token, the one-line `claude mcp
add`, the tool list, and the note that the agent appears as "Claude" with
its own cursor and that its edits are undoable by it, not by you.
