# Realtime: follow-ups

Deferred findings from the per-task and whole-branch reviews of the
`realtime` branch.

- Task 1: minor (deferred): repeated lock().unwrap_or_else idiom (codebase convention); hub_or_create retain is O(hubs); conn_id reuse race is moot with per-socket UUIDs (T2)
- Task 2: minor (deferred): concurrent appends can publish Doc out of order (client `remote` rule headSeq>local makes it harmless — now a hard requirement on Task 4); Doc.seq == head_seq (redundant field); doc frame may reach author before its POST reply (same rule covers it); hello/resync frame construction duplicated; serve() leaks a task per test; no presence rate limit server-side
- Task 2: minor (deferred): SEND_TIMEOUT differs test(250ms)/prod(10s) — the timeout path is tested, not the constant
- Task 3: minor (deferred): wsUrl non-browser fallback yields empty host (unreachable in practice); three hand-rolled timer handles across the two modules
- Task 4: minor (deferred): silent drop if queue.current null at edit time (unreachable today); `error` frames only reach console (surface in Task 5's status pill or later)
- Task 5: minor (deferred): peerFor/caretsAt are O(peers×tokens) per render (fine at demo scale); status label duplicated as title (fixed in round)
- Final: minor (deferred): infinite reconnect after auth loss (probe /api/me later); dead client ping/pong (keep for a future liveness probe); Doc.seq redundant; SEND_TIMEOUT cfg swap; O(peers×tokens) lookups; no hook/presence-UI automated tests
- Final: re-review — all 6 addressed. parked — origin_authority lowercases host only in the no-port branch — Ruling: real but fail-closed and browsers lowercase Origin; one-liner for the follow-ups list, no second fix wave — costs a false 403 for a mixed-case Origin with explicit port if wrong.
- Final: minor (deferred): same_origin ignores scheme (intentional behind a TLS-terminating proxy; documented Host requirement)
