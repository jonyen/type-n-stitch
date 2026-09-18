# Titles, captions and transitions: follow-ups

Deferred findings from the per-task and whole-branch reviews of the
`titles` branch.

- Task 1: minor (deferred): editlist::EPS is `pub` (could be pub(crate))
- Task 2: minor (deferred): `cut_points` naming; caption_windows overdub overlap test lacks EPS; joins override lookup ambiguous for two cuts sharing a start; title at 0 maps source 0 to output 0
- Task 5: minor (deferred): outputDuration duplicates pieces() arithmetic (derive from pieces later); tokenEnd for a title token returns `before` unclamped; caption_windows has no client mirror (captionsAt suffices for preview)
- Task 6: minor (deferred): titles()/captionsAt() recomputed per render; duplicate unmount cleanups; onPause never resets wantPlaying on a browser-initiated pause (pre-existing)
- Task 3/3b: minor (deferred): caption box not clamped to frame width for an unbreakable word; whitespace-only caption yields a padding-only box (Task 4 skips blank captions); no combined title+overdub+caption ordering test; fonts re-parsed per call (OnceLock); layout_lines/Rgba leak into the crate root; full TTFs served to the browser (subset/woff2 later)
- Task 6: minor (deferred): shown-title identity breaks if a sync replaces edit object identities mid-card (same assumption as the existing overdub undo check); momentary titling=null between chained cards relies on React batching
- Task 7: minor (deferred): Enter on a title card both selects and opens; style-chip radios show native inputs; pause-suggested cuts never get a gap token so no per-cut override UI for them (pre-existing tokenize boundary)
- Task 4: minor (deferred): caption cap over-rejects when a replacement would not grow the fold; Remove* earlier in a batch don't decrement; superseded PNGs never reaped; DefaultHasher not stable across Rust versions (comment); malformed op JSON is rejected by the extractor without an index (pre-existing)
- Task 4: minor (deferred): AppError::internal doesn't log; write_atomic leaves the temp file on failure; write path now folds the log per batch under the lock (no cache)
- Final: minor (deferred): re-probe retries forever when video is None; nextTitleAt exact compare; text::* glob export; TTFs unsubsetted; superseded PNGs unreaped; EditCaption missing so caption cap blocks replacements
- Final: re-review — all 7 addressed, no new breakage. minor (deferred): overdub entry rewinds to od.start after a mid-overdub title (pre-existing); captionRange only cleared via goHome/submit/cancel
