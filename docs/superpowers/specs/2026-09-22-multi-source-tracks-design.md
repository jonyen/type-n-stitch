# Multiple videos per project and stacked video tracks

Date: 2026-09-22
Status: approved design, ready for an implementation plan

## Problem

A project is one media file. `projects.media_id` names it, and every time in
the engine — cuts, splits, clip order, words, overlays — is seconds into that
file. There is no way to edit two recordings together, and the only way to
put a second picture over the first is the B-roll lane: one track, always full
frame, always silent.

This design lets a project hold several source videos, laid end to end on the
main track and transcribed as one script, and adds stacked video tracks (V2,
V3) whose clips cover the tracks below, full frame or as a corner
picture-in-picture, muted by default with an optional level.

## Decisions

- **Tracks are stacked layers**, as in Premiere's V1/V2/V3: an upper track
  covers the ones below for its span. The main track carries the transcript
  and the programme audio.
- **Imported videos go on the main track in sequence.** Each is transcribed;
  the transcript reads straight through all of them; the joins are clip
  boundaries that split, cut and reorder like any other.
- **Layer audio is muted by default**, with a per-clip switch that mixes it in
  at a set level.
- **Layer framing is full frame or a preset picture-in-picture corner.** No
  free transform.
- **Time model: one stitched source timeline.** Each source is placed at a
  fixed offset; every existing operation keeps working on plain seconds.

Rejected:

- **A source id on every range.** It rewrites the operation format, the fold,
  the MCP tools and every client mirror, and needs a log migration, for no
  visible gain.
- **Concatenating the files at import.** Every import re-encodes, adding a
  video later re-renders everything, and the project loses which file is which.
- **Multicam** (synced angles) and **sequential-only** readings of "tracks":
  not what the user wants.

## Data model and engine

### Sources

A new table:

```sql
CREATE TABLE project_sources (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  position   INTEGER NOT NULL,
  media_id   TEXT NOT NULL,
  start_at   REAL NOT NULL, -- stitched offset (OFFSET is an SQL keyword)
  duration   REAL NOT NULL,
  PRIMARY KEY (project_id, position)
);
```

Media stays content-hashed and shared, as today. The migration writes one row
per existing project: `(media_id, 0, 0.0, duration)`.

### Stitched time

Source _k_ occupies `[offset_k, offset_k + duration_k)`. The offsets are fixed
when a source is added and never change, so every time already in an
operation log stays valid.

### Operations

- `AddSource { media, offset, duration }` appends a source and adds a
  permanent split at `offset`, so each video starts as its own clip. Undo
  removes both. `offset` must equal the current stitched end; the server
  rejects anything else.
- Removing a video is an ordinary cut over its range. The log stays
  append-only and the removal is undoable.
- `AddLayer { track, start, end, media, offset, frame, audio }`:
  - `track`: `2` or `3`.
  - `frame`: `full`, `pipTopLeft`, `pipTopRight`, `pipBottomLeft`,
    `pipBottomRight`.
  - `audio`: `null` (muted) or a level in dB, as `AddAudio` uses.
- `SetLayer { track, start, to_track, frame, audio }` moves the layer on
  `track` starting at `start` to `to_track` and sets its frame and audio. `RemoveLayer { track, start }` removes it.
- `AddBroll` and `RemoveBroll` stay readable. The fold turns them into a
  track-2, full-frame, muted layer, so no existing log is rewritten.

Layers stay anchored to main-track (stitched source) time, exactly as B-roll
is today, so they follow their words through cuts and reorders.

### Fold and timeline

Pieces, splits, order, cuts, overdubs, titles and holds are unchanged: they
are numbers in stitched time. The one new function is

```rust
pub fn locate(sources: &[Source], t: f64) -> Option<(MediaId, f64)>;
```

mapping a stitched instant to a file and a time in it. An instant exactly on
a join belongs to the later source. The export planner and the player use it.

### Words and speakers

Each media's transcript stays cached per `media_id`. The project's word list
is each source's words shifted by its offset, concatenated in source order.
Speaker indices are namespaced per source (the fold assigns each source a
base index), so speaker 0 in video 1 and speaker 0 in video 2 are different
people; names are editable as today.

## Import and transcription

- **Home.** The dropzone accepts several files. They are listed sorted by
  name and can be reordered by dragging before import. The first creates the
  project (`POST /api/projects`, as today); the rest are appended one at a
  time.
- **In a project.** Insert ▾ → **Add video…** appends files at the end.
- **Route.** `POST /api/projects/{id}/sources` takes a multipart upload,
  stores the media under its content hash, probes its duration, appends
  `AddSource` at the current stitched end, and returns the source. Editors and
  owners only.
- **Transcription.** The existing pipeline (whisper wav, transcribe, diarize,
  silences) runs once per source and is cached per media. `/transcribe`
  reports progress per source. A source still transcribing shows in the
  transcript as a greyed "Transcribing video 2…" block; its clip on the
  timeline is usable. Filler and pause suggestions cover every source once it
  is ready.
- **Mixed formats.** The project canvas is the first source's resolution and
  frame rate. At export each source is scaled to fit and letterboxed onto the
  canvas, resampled to the canvas frame rate, and its audio resampled to
  48 kHz stereo, in the same ffmpeg filter graph that applies cuts. The
  preview shows each file with `object-fit: contain` in a frame of the canvas
  aspect ratio, so preview and export agree.
- **Audio-only sources** are allowed and show black under their words.
- **Limits.** At most 20 sources per project; the existing per-file size
  limit applies. An oversized or unreadable file is rejected with a clear
  error and nothing is appended.

## Tracks in the UI

### Timeline

Lanes, top to bottom: **V3, V2, Clips (V1), Music**. The B-roll lane becomes
V2. V3 appears once it has a clip; until then a thin "+ V3" row stands in for
it. Below 600 px only the Clips lane shows, as today.

The Clips lane gains a small source badge on each clip ("1", "2", …) and a
thin divider where one source ends and the next begins. Razor, Range,
reorder and delete are unchanged.

### Adding and editing a layer

- Select words, then **Insert ▾ → Layer…** (replacing B-roll). The dialog is
  the asset picker plus **Track** (V2/V3), **Frame** (full or one of four
  picture-in-picture corners) and **Sound** (off, or on with a level). The
  picker lists project assets and the project's own sources, so a stretch of
  video 2 can go over video 1.
- The floating selection toolbar's B-roll button becomes **Layer**.
- Click a layer bar to select it; Delete removes it. Double-click opens the
  dialog to change track, frame or sound (`SetLayer`).

### Preview

The Player stacks one `<video>` per visible layer above the main video, upper
tracks on top. A picture-in-picture layer is 30 % of the canvas width, 4 %
in from its corner. Layers with sound play at their level; the rest are
muted. The timeline's hover preview stays the main track's frame.

### Transcript

B-roll tags become layer tags — "V2 · clip.mp4 · PiP ↗", with a speaker icon
when sound is on. Clicking one selects the layer.

### Export

The planner overlays V2, then V3, onto the main track with ffmpeg `overlay`
enabled over each layer's output span. Picture-in-picture layers pass through
`scale` first and overlay at the corner offset. Layers with sound are mixed
with `amix` at their level, under the main audio and ducked like music.

## Agents (MCP)

New tools:

- `add_source(media)` appends a project asset or uploaded media as a source.
- `add_layer(from, to, media, track, frame, audio)` over word indices.
- `set_layer(track, start, to_track, frame, audio)`.

`add_broll` stays as an alias for track 2, full frame, muted. `list_clips`
reports each clip's source.

## Migration and compatibility

- The SQL migration above, plus one source row per existing project.
- A log with no `AddSource` has one source by definition; `AddBroll` folds to
  a V2 layer.
- `GET /api/projects/{id}` gains `sources`. `media` stays for one release so
  open tabs on an older client keep working.

## Testing

- **Engine:**
  - `AddSource` with its split and its undo.
  - `locate` at every boundary, including exact source ends and joins.
  - `AddBroll` folding to a layer.
  - An export plan for two sources of different resolution and frame rate,
    checked against the expected filter graph.
  - An export plan with V2 full frame, V3 picture-in-picture and one layer
    with sound.
- **Client mirror:** stitched words with offsets and namespaced speakers,
  source badges, `timelineSegments` across sources, layer spans per track.
- **Server:** `/sources` append and its errors, per-source transcription, the
  20-source cap, the MCP tools and the `add_broll` alias.
- **Live:**
  1. Import three clips at once, one of a different resolution.
  2. Reorder clips across sources.
  3. Make a Range cut across a source join.
  4. Put a stretch of video 3 as a picture-in-picture layer with sound on V3
     over video 1.
  5. Export and check it with ffprobe: duration, canvas resolution, one audio
     stream.

## Order of work

Each step leaves the app usable.

1. Engine: stitched sources, `locate`, `AddSource`, the `Layer` edit, and
   `AddBroll` mapped onto it. No UI change.
2. Server: migration, `/sources`, per-source transcription, `sources` in the
   project response.
3. Export: per-source normalisation to the canvas; multi-source cuts.
4. Client: multi-file import on home and in Insert, stitched words, source
   badges and dividers, playback across files.
5. Layers: V2/V3 lanes, the Layer dialog, the preview stack, layer export
   (overlay, picture-in-picture, layer audio).
6. MCP tools and the README.

## Out of scope

- Dragging or resizing layer bars.
- Free transform, keyframes, opacity.
- More than three video tracks.
- Multicam sync.
- Removing a source's files from disk when its range is cut.
