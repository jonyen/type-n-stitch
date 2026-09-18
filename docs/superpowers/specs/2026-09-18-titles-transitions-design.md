# Titles, captions and transitions

Date: 2026-09-18
Status: approved design, ready for an implementation plan

## Problem

An edit today is a cut or an overdub. A demo-quality cut needs a title card
between sections, a lower-third under a speaker, and something softer than a
jump cut where two pieces meet. All three must live in the operation log so
they are collaborative, undoable and exported by the engine like every other
edit.

## Approach

New edit kinds inside the existing model. Titles and captions are edits
anchored to source time, transitions are a project setting with a per-cut
override, and the timeline, ffmpeg planner and browser preview each learn the
new kinds. Rejected: a separate "layers" table (two sources of truth for the
timeline) and server-side pre-rendered title clips (a render step per edit, no
instant preview; revisit when inserting arbitrary B-roll matters).

## Data model

`engine::Edit` gains two variants and one field:

```rust
Title   { at: f64, duration: f64, text: String, subtitle: Option<String>, style: TitleStyle }
Caption { start: f64, end: f64, text: String, position: CaptionPos }
Cut     { start: f64, end: f64, #[serde(default)] transition: Option<Transition> }

enum TitleStyle { Dark, Light, Accent }
enum CaptionPos { BottomLeft, BottomCenter, TopLeft }
enum Transition { None, Dip, Crossfade }
```

`ProjectDoc` gains `transition: Transition` (default `None`), the
project-wide setting. Serde defaults keep every existing log and client
payload parsing.

A title is an insertion at source time `at`, between two words; the output
grows by `duration`. A caption is an overlay over source range `[start, end)`;
the output length is unchanged. Two titles at the same `at` render in
sequence order.

Operations mirror the edits:

- `AddTitle { at, duration, text, subtitle, style }`
- `EditTitle { at, duration, text, subtitle, style }` — replaces the title at `at`
- `RemoveTitle { at }`
- `AddCaption { start, end, text, position }`
- `RemoveCaption { start }`
- `SetTransition { transition }`
- `SetCutTransition { start, transition: Option<Transition> }` — the cut whose range starts at `start`

`fold` applies them in order like today's operations; `Undo`/`Redo` work
unchanged because they only flip the `undone` flag. The client reducer mirrors
each for optimistic display.

Validation in `ops::validate`: `at`, `start`, `end` within `[0, duration]`;
title `duration` within `[0.5, 30]`; `text` and `subtitle` at most 200
characters; at most 32 titles and 32 captions per project; `Crossfade` is
rejected with a 400 saying it is not yet supported (the value exists so the
client can show it disabled).

## Timeline

`timeline()` inserts a `Segment { kind: Title { index }, source: [at, at),
output: [t, t + duration) }` at the point where `at` falls between kept
pieces, splitting a kept piece if `at` is inside it. A title whose `at` sits
inside a cut range still renders: it is an insertion, not content, and the
editor placed it there.

Captions create no segments. `caption_windows(segments, edits)` returns, per
segment, the caption text and its `[a, b)` in segment-relative time for every
caption whose source range overlaps that segment.

Transitions create no segments. `joins(segments, edits, project_transition)`
returns each boundary between consecutive output pieces with its effective
transition: the cut's override when the boundary is a cut, else the project
setting; both sides of a title are always `Dip`.

`source_to_output_time` and `output_to_source_time` treat a title like an
overdub freeze: a source time at `at` maps to the title's output start, and
output times inside the title map back to `at`.

## Rendering text

ffmpeg's `drawtext` filter needs libfreetype, and the Homebrew ffmpeg builds
(7 and 8) ship without it, so the engine rasterises text itself. A pure-Rust
`engine::text` module (`ab_glyph` + `png`) draws a title card at the source
frame size and a caption box as an RGBA image, using Inter (SIL OFL, bundled
under `engine/assets/inter/`). The server writes those PNGs into the media
directory before an export and the same font files are served to the browser
at `/fonts/`, so the preview and the render use one typeface.

## ffmpeg

- Title piece: the card PNG as an extra input, `-loop 1 -framerate <fps>
-t <dur> -i card.png`, then `format=yuv420p,setsar=1,trim=end=<dur>,
setpts=PTS-STARTPTS`; audio is `anullsrc=r=48000:cl=stereo,atrim=end=<dur>`
  normalised like every other piece. Background and text colours come from
  `TitleStyle`.
- `W`, `H` and `fps` come from ffprobe. `Probe` and `Meta` gain `video:
{width, height, fps}` (video only); media probed before this change is
  re-probed the first time an export needs it and its `meta.json` rewritten.
- Caption: the box PNG as an extra input, composited onto the containing
  segment with `overlay=x=<x>:y=<y>:enable='between(t,a,b)'`; position from
  `CaptionPos`.
- Dip: for every join with `Dip`, `fade=t=out:st=<len-0.25>:d=0.25` on the
  piece before and `fade=t=in:d=0.25` on the piece after, with `afade`
  equivalents. Crossfade is out of scope for this work.
- Audio-only exports keep a title's silence so durations match the video
  export, and ignore captions.
- No font configuration: the bundled font is the only font. Media without
  dimensions (audio, or video whose probe failed) renders titles at 1280x720
  at 30 fps rather than failing.

## Browser preview

`usePlayback` gains a third pause state beside overdub: reaching `at` pauses
the video and starts a `duration` timer, after which playback resumes at `at`.
`playback.titling` exposes the active title. `Player` renders a `TitleCard`
over the frame while titling, in Inter served from `/fonts/`, so the preview
matches the export closely (not pixel-identically).

`Player` renders `Caption` overlays for every caption whose source range
contains `currentTime`. For `Dip` joins it toggles a `fading` class on the
frame for 250 ms on either side of the boundary, from a `joins()` mirror in
`editlist.ts`. The scrubber shows titles as a third mark colour.
`outputDuration` in `editlist.ts` adds title durations; the stats line stays
correct.

## UI

- Transcript: a title renders as a full-width card token between words (text
  and duration); click selects it, Enter or double-click opens the editor,
  Delete removes it. A caption renders as a small tag under its words with the
  same interactions.
- Toolbar: "Add title" inserts after the current selection, or at the playhead
  with no selection; "Add caption" covers the selection and is disabled without
  one; a "Transitions" select offers None / Dip / Crossfade (disabled, "soon").
  A cut's gap token gets a small menu to override its transition.
- `TitleDialog` (text, subtitle, three style swatches, duration) and
  `CaptionDialog` (text, position) reuse the existing modal shell.
- Realtime and presence need nothing new: titles and captions ride the fold,
  and a peer's title appears as a card in everyone's transcript.

## Error handling

A rejected operation returns 400 with the operation index as today and the
client rolls back that step. Media without dimensions (audio, or video whose probe failed)
exports titles as 1280×720 at 30 fps rather than failing.

## Testing

- Engine: timeline with a title between pieces, inside a piece and inside a
  cut; two titles at one `at`; `caption_windows` across a segment boundary;
  `joins` with project default, per-cut override and title sides; remap
  functions through a title; ffmpeg argument snapshots for a title, a caption,
  a dip join and an audio-only export with a title; `drawtext` escaping.
- Server: validation of every new operation (ranges, duration bounds, text
  length, count caps, crossfade rejection); fold of add/edit/remove for titles
  and captions; `SetTransition` round trip through `GET /projects/:id`.
- Client: reducer and `opForAction` mirrors for every new action;
  `editlist.ts` `outputDuration` and `joins` mirrors against the engine's
  cases; `tokens.ts` placing a title card between words; `usePlayback`'s
  title timer via a pure helper; dialog helpers.
