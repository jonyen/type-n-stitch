# Tracks: split, reorder, B-roll and music

Date: 2026-09-20
Status: approved design, ready for an implementation plan

## Problem

Every edit today keeps the source order: cuts remove, overdubs replace,
titles insert, captions overlay. A demo-quality cut also needs to move the
best line to the front, cover a talking head with a shot of the product,
and put music under the intro. All three must stay in the operation log so
they are collaborative, undoable, visible to peers and drivable by the
agent.

## Approach

Stay source-anchored and add order on top. Two new operations, `Split` and
`Move`, give the project an explicit output order of pieces; B-roll and
audio are edits anchored to a range of the main source, like captions, so a
reorder carries them with their words. The timeline lays pieces out in that
order and maps every overlay through it; ffmpeg already concatenates pieces
in timeline order, so reordering costs nothing in the planner.

Rejected: a real multi-track model with output-time anchoring (replaces the
fold, breaks "edit the words", rewrites every edit kind); output-anchored
overlays on source-anchored cuts (a cut before a shot shifts it off its
words).

## Data model

`engine::Edit` gains:

```rust
Broll { start: f64, end: f64, media: String, offset: f64 }
Audio { start: f64, end: f64, media: String, offset: f64, gain: f64, duck: bool }
```

`Broll` shows asset `media` from `offset` over main-source `[start, end)`;
the main audio continues. `Audio` mixes asset `media` from `offset` over
`[start, end)` at `gain` dB (default 0), ducked under speech when `duck`
(default true). Serde defaults keep old logs and payloads parsing.

`ProjectDoc` gains `splits: Vec<f64>` (sorted, deduplicated) and
`order: Vec<f64>` (piece starts in output order).

A **piece** is a kept range after cuts, further divided at every split that
falls inside it. A piece's identity is its source start. `order` may name
starts that no longer exist (a cut swallowed them): the fold ignores those.
Pieces missing from `order` append in source order. Old logs, new cuts and
undo therefore stay consistent without rewriting `order`.

Operations:

- `Split { at }`, `Unsplit { at }`
- `Move { piece: f64, before: Option<f64> }` — `None` means the end
- `AddBroll { start, end, media, offset }`, `RemoveBroll { start }`
- `AddAudio { start, end, media, offset, gain, duck }`,
  `EditAudio { start, gain, duck }`, `RemoveAudio { start }`

`fold` applies them in order; `Undo`/`Redo` flip the flag as today. The
client reducer mirrors each for optimistic display.

Validation in `ops::validate`: `Split.at` within `[0, duration]`, not inside
a cut, not an existing split; `Move.piece` and `before` must be current
piece starts; B-roll and audio ranges within `[0, duration]` with
`start < end`; `offset >= 0` and, for B-roll, `offset + (end - start) <=`
the asset's duration (audio shorter than its window plays out and stops);
the asset must belong to the project; `gain` within `[-30, 12]`; at most 64
splits, 32 B-roll and 16 audio edits per project.

## Timeline

`timeline()` builds pieces, sorts them by `order`, then inserts titles and
overdub freezes as today. A title whose `at` falls inside a piece splits that
piece in output; a title at a boundary lands before the next piece in output
order.

`caption_windows` generalises to `overlay_windows(segments, ranges)`: each
overlay's source range is intersected with every segment and emitted as
segment-relative windows. Captions, B-roll and audio all use it, so a range
that spans a reordered boundary renders as two windows in the right places.

`source_to_output_time` and `output_to_source_time` become piece-aware. The
output-to-source direction stays single-valued; the source-to-output
direction returns the output time of the piece containing the source time.

## Assets

Table `project_assets(id, project_id, kind, name, duration, width, height,
created_at)` with `kind` in `video | audio`. Files live at
`<media_dir>/assets/<id>.<ext>` and are served under `/data/` like the main
media. Routes:

- `POST /api/projects/:id/assets` (multipart, editor or above) probes the
  file with ffprobe, extracts a poster frame for video, stores the row.
- `GET /api/projects/:id/assets` (any member).
- `DELETE /api/projects/:id/assets/:aid` (editor or above), refused with 409
  naming the edits while any live edit references it.

## ffmpeg

- Pieces already concatenate in timeline order, so `Move` needs no planner
  change.
- B-roll: one extra input per asset. For each window, `trim` from `offset`,
  `scale` to the frame, `setpts`, then `overlay=enable='between(t,a,b)'` on
  the containing segment, the same shape as caption overlays.
- Audio: one extra input per asset. Per window: `atrim`, `adelay` to the
  window start, `volume=<gain>dB`. Ducking multiplies by a speech envelope
  built from the segment's word times: 12 dB down under words with 120 ms
  ramps, expressed as a `volume` filter with a time expression. Then
  `amix=inputs=N:normalize=0` with the main track.
- Audio-only exports mix music and ignore B-roll. Rotation via `oriented()`
  as today.

## Browser preview

`Player` adds a second, muted `<video>` for B-roll positioned over the
frame, shown only inside a window and seeked to `offset + (t - start)`, and
one `<audio>` per active audio edit with `volume = gain × 0.25` while a word
is under the playhead and `duck` is set. `usePlayback` exposes
`activeOverlays(t)` from the same window list; seeking across a reorder
boundary jumps the main video as cuts do today. The scrubber shows B-roll
(teal) and audio (amber) marks.

## UI

- **Clip strip** above the transcript: one card per piece in output order
  with a poster frame from the existing thumbnails, its duration and first
  words. Drag to reorder with a drop indicator (pointer events); click
  selects the piece and scrolls the transcript to it.
- **Split here** in the toolbar splits at the caret, or at the playhead with
  no caret. A split renders as a thin divider in the transcript; Delete on a
  selected divider unsplits.
- The **transcript** renders words in output order (pieces concatenated).
  Selection, cut, caption and title keep working on source ranges; word to
  output mapping is piece-aware.
- **Add B-roll** (needs a selection) opens a dialog: drop a file or pick a
  project asset, an offset slider with a poster preview.
- **Add music** (selection, or "whole edit") opens a dialog: file or asset,
  gain, duck toggle.
- B-roll and audio render as tags under their words like captions; click to
  edit or remove.
- Presence and realtime need nothing new.

## MCP

Tools `list_clips()`, `split(after)` (after a word index; `-1` before the
first word), `move_clip(clip, before)` (clip = index in the current output
order; `before` = index or `null` for the end), `list_assets()`,
`add_broll(from, to, asset, offset?)`, `add_audio(from, to, asset, gain?,
duck?)`. Uploading assets stays in the browser. `open_project` returns the
clips.

## Error handling

Rejected operations return 400 with the operation index as today and the
client rolls back that step. Deleting a referenced asset returns 409 naming
the edits. A missing asset file shows a placeholder in the preview and fails
the export with the asset's name.

## Testing

- Engine: pieces and order (stale entries, missing pieces, splits inside
  cuts); timeline reorder with titles and overdubs; `overlay_windows` across a
  reordered boundary; remap both ways through a reorder; ffmpeg argument
  snapshots for B-roll, audio with ducking, audio-only with music.
- Server: validation of every new operation; fold of add/edit/remove; asset
  routes including the referenced-delete 409; the MCP tools.
- Client: reducer and `opForAction` mirrors; output-order token layout; the
  strip's drag helper; the overlay window mirror against the engine's cases;
  dialog helpers. Browser check of the strip drag and both overlays.
