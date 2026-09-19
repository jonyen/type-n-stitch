# MCP agent: follow-ups

Deferred findings from the per-task and final reviews of the `mcp` branch
(plan `2026-09-19-mcp-agent.md`). None block the demo; each is small.

## Behaviour

- `export` holds one MCP request for up to ten minutes. The `{ jobId, pending: true }` escape hatch is inert: nothing consumes a job id. Add an `export_status(job_id)` tool.
- A bot's role is read once at `open_project` and cached until the agent is evicted, so removing or downgrading a member is invisible to the agent for up to ten minutes. This mirrors the WebSocket path. `remove_member` and `add_member` could evict the owner's agent the way `tokens::delete` does.
- `undoable`/`redoable` in tool results are booleans; the browser's `doc` frame carries sequence numbers. Documented in code, but diverges.
- `look_at` also moves the playhead, and `open_project`/`look_at` return extra `role`/`text` keys beyond the spec.
- `find_ranges` normalises transcript tokens too, so a punctuation-only token (a bare dash) collapses to an empty string and breaks phrase adjacency.
- `Open.speakers` is cached at open; diarisation finishing mid-session is not seen until the project is reopened.
- `require_open` says "open a project first" when the bot id changes mid-session (token swap); a clearer message would help.
- `get_info` says every editing tool returns touched words; the no-op branch of `remove_fillers`/`tighten_pauses` returns `{ applied: 0, message }` only.

## Code health

- `server/src/mcp.rs` is about 2,000 lines. The pure helpers (`report`, `already_cut`, `touched_by`, the style/position/transition parsers, `range_of`, `words_in`) belong in `mcp_tools.rs`.
- `touched_by` (cursor) and `word_range` (cut) use two different word-ownership rules on purpose; cross-reference them.
- Bearer parsing is duplicated between `mcp.rs` and `auth.rs`.
- `report()` duplicates `open_project`'s cut/overdub/output-duration block.
- `set_transition` parses `kind` twice.
- `tokens::verify` hashes the plaintext twice.
- `Role::rank` is a hand-rolled table where deriving `Ord` would do.
- `ToolRouter` is rebuilt per request under the stateless lifecycle; a `OnceLock` would remove that.
- `start_export` takes `&Arc<AppState>` while its siblings take `&AppState`.

## Tests

- No test forces the bot colour fallback (`#3fb3b3`).
- The live-token HTTP test asserts only "not 401" (no `Accept` header, so rmcp answers 406).
- The export test's `unwrap_err` panics opaquely if a render ever survives the test cap.
- `settle()` in the mcp tests is a 64-iteration `yield_now` barrier; fine on the current-thread runtime, fragile elsewhere.
- Cookie plus Bearer on one request (Bearer wins, even when invalid) is only proven by reading the code.

## Client

- `CopyBox` swallows clipboard failures silently.
- No modal in the app has `role="dialog"`; `FormEvent` is deprecated in React 19 types across all dialogs.
- Each live agent holds an undrained broadcast receiver, pinning up to 256 `ServerMsg` clones per open project. Bounded, not a leak.
