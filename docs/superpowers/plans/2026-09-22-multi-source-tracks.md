# Multiple Videos and Stacked Tracks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a project hold up to 20 videos laid end to end on its main track and transcribed as one script, and add stacked video tracks V2 and V3 whose clips cover the tracks below, full frame or as a corner picture-in-picture, muted or at a set level, in the editor, the preview, the export and the MCP tools.

**Architecture:** One stitched source timeline: each source sits at a fixed offset, so every existing operation keeps working on plain seconds and no log is rewritten. The engine gains `Source`, `locate`, stitched words and speakers, an `AddSource` op whose fold adds a permanent split at the join, and an `Edit::Layer` (with `AddLayer`/`SetLayer`/`RemoveLayer`) that B-roll folds onto. The server keeps a `project_sources` registry, appends uploads through `POST /api/projects/{id}/sources`, transcribes each source in the background and serves stitched words. The export planner opens one input per source, fits each onto the canvas (the first source with a picture) and overlays V2 then V3 per piece. The client mirrors the fold, plays every file through one `<video>` behind a stitched clock (`StitchedMedia`), and draws V3/V2/Clips/Music lanes, a Layer dialog and a stacked layer preview.

**Tech Stack:** Rust (engine crate; axum, sqlx/SQLite, tokio and rmcp in the server), ffmpeg/ffprobe, React 19 + TypeScript on Vite 7 with CSS Modules and `radix-ui`, vitest 3 with jsdom and Testing Library.

**Spec:** docs/superpowers/specs/2026-09-22-multi-source-tracks-design.md

## Global Constraints

- Sources: at most 20 per project (`MAX_SOURCES = 20`), counted on the timeline (`all_sources(first, &doc.sources).len()`); the 21st is a 400 naming 20.
- Layer tracks are 2 and 3 only; the main track (1) never holds a layer.
- Frame names, exactly: `full`, `pipTopLeft`, `pipTopRight`, `pipBottomLeft`, `pipBottomRight`.
- Picture-in-picture geometry, in the preview and the export alike: w = 0.3W, x = 0.04W or W − w − 0.04W, y = 0.04H or H − h − 0.04H (W, H the canvas; h from the layer's own aspect).
- Layer audio is `null` (muted) or a level from −30 to +12 dB, and it is always ducked under speech, like music.
- An instant on a join (within `EPS`) belongs to the later source; the stitched end maps to the last source at its duration.
- The canvas is the first source with a picture, otherwise the default (`DEFAULT_VIDEO` in the export, 16:9 in the preview); audio is always resampled to 48 kHz stereo.
- `AddBroll` folds to a track-2, full-frame, muted layer (`RemoveBroll` to `RemoveLayer { track: 2 }`).
- Logs are never rewritten: old ops stay readable and fold to the new shapes.
- `media` stays in the `GET /api/projects/{id}` project response for one release, beside the new `sources`.
- Op tags are `addsource`, `addlayer`, `setlayer` and `removelayer`, with the field `toTrack`; the edit tag is `layer`.
- Word ids of source 0 are unchanged; word ids of source k > 0 are `"k:id"`.
- Joins stay in `splits` and cannot be unsplit (the fold ignores an `Unsplit` at a join).
- Undoing an `AddSource` is refused while a later source exists; redoing one is refused unless it would land at the stitched end.
- The checks: `export PATH="$HOME/.cargo/bin:$PATH"` first, then `npm test` (`cargo test && vitest run`), `npm run lint` (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && eslint . && prettier --check .`) and `npm run build -w client`. Run `npx prettier --write` on changed client files before linting.
- Every commit message ends with exactly this trailer:
  ```
  Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2
  ```

## Decisions made while planning

Parts A–E are the five planners: A engine (Task 1), B server (Tasks 3, 4 and 11), C export (Tasks 5 and 10), D client sources (Tasks 2, 6 and 7), E layers UI (Tasks 8 and 9). R1–R8 are the merge rulings.

- **R1:** `doc.sources` holds the appended sources only, everywhere: the engine's `ProjectDoc.sources`, the server's `DocState`, the WebSocket `hello`/`doc` frames, and the client's `DocState`/`RemoteDoc`/`resync`; the client prepends source 0 itself, and only `project.sources: SourceView[]` on `GET /api/projects/{id}` lists the first source too.
- **R2:** the engine's `stitch_speakers` is the only implementation of the speaker base rule; the server's `sources::stitch_speaker_labels` calls it and only pads labels to each source's word count and shifts turns by the same base.
- **R2:** speaker labels stop at the first source that is not transcribed yet: the ready prefix is passed with its diarization, every later source with no labels and no speakers, so a base never moves under a typed name.
- **R3:** the preview canvas follows the export's: the first playable source whose video has `videoWidth > 0` (audio-kind sources skipped), else 16:9; a project with no video source keeps today's audio frame, as its export renders no picture.
- **R4:** the client polls `GET /api/projects/{id}` every 2 s while any source is `pending` or `running`, posts `/transcribe` (`{ words, sources }`, which waits for source 0 only) when the ready count grows, and also when a source reads `pending`, since only `/sources` and `/transcribe` start jobs.
- **R5:** Tasks 8 and 9 build on Task 7's shapes: `Player` takes `sources` and `stitched` (no `media`/`mediaRef`), `Timeline` and `Transcript` take `sources`, App passes `playable`, and Task 8 renames every B-roll UI name Task 2 kept (`OverlayRef` `'broll'`, the lane and label, `BrollDialog`/`brollRange`, `onBrollClick`, `onBroll`, `onAddBroll` → `onAddLayer`, the menu item, `.broll`/`.brollTag`).
- **R6:** after Task 1 the server rejects `AddSource`, `AddLayer`, `SetLayer` and `RemoveLayer` with "not supported yet"; Task 3 deletes that arm and validates them for real.
- **R7:** media ids are random UUIDs (`Uuid::new_v4`), not content hashes, so there is no dedup of uploads; the only derived id is MCP `add_source` promoting an asset (`Uuid::new_v5` of the asset id), so the same asset added twice shares one directory and transcript.
- **R8:** layer media resolves in one place, `sources::resolve_layer_media` (a project asset first, else any media in the project's registry), called by `AddLayer` validation (`sources::layer_media`) and by `assets::asset_files` for export; music stays asset-only.
- Engine: `stitch_speakers(parts: &[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>)` lives in `speakers.rs`, and each source raises the base by `max(count, highest label + 1)`, so a too-small cached count never merges two people.
- Engine: `Source` and `Frame` are in `types.rs`; `all_sources`, `stitched_duration`, `locate` and `stitch_words` in `editlist.rs`; `lib.rs` already glob re-exports them as `engine::<name>`.
- Engine: `locate` returns `(usize, f64)`, an index into `all_sources(..)`, not the spec's `(MediaId, f64)`; `None` before 0 or more than `EPS` past the end.
- Engine: a join is permanent: `Unsplit` at a source offset is a no-op in the fold, so no piece ever spans two files; the server does not also reject it, because the client never sends one (the fold is the guarantee).
- Engine: `AddLayer` replaces the layers it overlaps on its own track only; `SetLayer` edits in place (keeping its index) and drops other layers it now overlaps on `to_track`; layers are addressed by `(track, start)`.
- Engine: a layer's `audio` serialises as `null` when muted (never omitted), `offset` defaults to 0 when missing, and `frame` is required.
- Engine: until Task 10, `broll_windows` selects every `Edit::Layer` and the planner draws each full frame and muted, pinned by `a_layer_on_any_track_exports_full_frame_and_muted_for_now`, which Task 10 rewrites.
- Engine: no new cap constant; the layer cap reuses `MAX_BROLL` (32), counting `Edit::Layer` on both tracks together.
- Engine: joins stay in `doc.splits` and count toward `MAX_SPLITS` (64); at most 19 joins leaves 45 splits for the user, so they are not exempted.
- Engine: Task 1 touches the server only to keep it compiling (`Edit::Broll` → `Edit::Layer` in `ops.rs`, `assets.rs`, `mcp.rs`, plus the R6 arm).
- Server: `POST /transcribe` returns `{ words, sources }`: `words` is `stitch_words` over every ready source (a source not ready adds no words), `sources` is `SourceView[]`; source 0 is transcribed before answering, the rest in the background.
- Server: transcript status is not persisted: `ready` = the media's `words-v2.json` exists, `running`/`error` = the in-memory `AppState::transcripts` map, `pending` = neither; an `error` is sticky until restart, so polling never loops whisper.
- Server: `POST /sources` starts the source's transcription when the file lands; `/transcribe` starts any source it finds `pending`.
- Server: speaker names stay in `doc.speaker_names`, keyed by the stitched index, through the unchanged `RenameSpeaker` (still capped at `MAX_SPEAKERS`); a source whose diarization fails counts as 0 speakers.
- Server: the 20-source cap is checked by the route before reading a byte of the body and again by `AddSource` validation inside the write transaction; an undone source frees its slot.
- Server: registry rows record zeros for position 0's timing (SQL cannot read `meta.json`), in the backfill and in `create_project`; nothing reads registry timing back, because the fold's `doc.sources` is authoritative.
- Server: `project_sources` is a registry of every media uploaded into a project (listing, membership checks, `/data` lookups); it never shrinks, not even on undo.
- Server: `/data` stays an unauthenticated `ServeDir` of unguessable ids; `/thumbnails` accepts an optional `{ "media": id }` for any registry media and 400s for others; `adopt_orphans` skips media that is a source.
- Server: `AddSource` must sit at the stitched end, name registry media, and carry the media's real duration; ranges and `Split` edges are checked against the stitched duration.
- Server: MCP `add_source(media)` takes an asset id or a registry media id; `add_layer` takes an optional `offset`, defaults `track` 2 and `frame` `full`, and an omitted `audio` means muted; `set_layer` keeps the track and framing when omitted, and an omitted `audio` mutes.
- Export: `build_ffmpeg_args(&[SourceInput], edits, opts)` replaces the single input; input k is `sources[k]`, every source is opened, and overdubs, titles, captions, layers and music are numbered from `sources.len()`.
- Export: `ExportOptions` loses `duration` and `video`; `kind` means "the project has a picture" and the server fills it with `sources_kind(sources)`.
- Export: `canvas(sources)` is the first source with a picture, an unprobed video counts as `DEFAULT_VIDEO`, and titles and captions are rasterised at the canvas.
- Export: a one-source graph is byte-for-byte unchanged: a source that already matches the canvas gets no normalisation filters.
- Export: normalisation runs per piece after the trim (`trim → setpts → scale(fit) → pad → fps → setsar=1`), because an output label can be consumed only once; the same fit follows an overdub's frozen frame.
- Export: audio needs nothing new: every chain already ends in `AUDIO_NORMALIZE` (48 kHz stereo), and `media::probe` already rejects files without audio.
- Export: `split_at_joins` splits any source piece that crosses a join inside the planner too, as a guard against removed splits or callers passing none.
- Export: an audio-only source shows black on a video export (`color=c=black:s=WxH:r=R:d=hold,format=yuv420p,setsar=1`); NTSC rates are written as fractions (`30000/1001`).
- Export: layers are overlaid per piece, as B-roll was, with piece-relative `enable` windows, so dips fade layers with the picture; V2 windows come before V3 in every piece.
- Export: picture-in-picture is `scale=<even(0.3·W)>:-2`, inset 4 % of the width and 4 % of the height, with right/bottom corners as `main_w-overlay_w-mx` / `main_h-overlay_h-my`; full frame keeps the fit-and-pad chain at `x=0:y=0`.
- Export: a layer with sound goes through the same `bed()` chain as music (`atrim`, `AUDIO_NORMALIZE`, `adelay`, `volume`) as `[a{i}l{n}]`, always ducked, and only where its picture would be (not over title cards).
- Export: one `-i` per layer window; a muted layer opens no input in an audio-only export, a layer with sound always does; the labels `[v{i}b{n}]`/`[v{i}bo{n}]` stay.
- Export: the export stitches words itself from each source's `words-v2.json`; a source not transcribed yet only means no ducking there.
- Client: `StitchedMedia` drives one `<video>` behind a stitched clock that swaps files when time crosses a join and holds the element's events back while the next file loads, so `usePlayback` only gains a structural `PlaybackMedia` type.
- Client: a `play()`/`pause()` during a swap is reported at once and honoured when the file lands; a swap started while playing keeps playing and fires no `pause`.
- Client: one detached, muted `<video preload="auto">` warms the next file; a short gap at a join is accepted (no gapless double buffer).
- Client: `EditorState.sources` is `[first, ...doc.sources]` with `first` from `load`; `duration` is `stitchedDuration(sources)` on every sync, so undoing an `AddSource` shrinks the edit.
- Client: `playableSources` joins the fold's sources to the listed `SourceView`s by media id; a source the list does not know yet triggers one refetch.
- Client: `addSource` is an editor action but never an op: the upload route appends `AddSource`, the reducer's copy is the optimistic echo, and an `addSource` at an existing offset is ignored.
- Client: a join is not a user split: the reducer and `opForAction` ignore `unsplit` at a join, clicking a join never selects it, and the transcript labels it as where a video starts.
- Client: Task 2 removes `BrollEdit` and the `addBroll`/`removeBroll` actions and ops, and routes every B-roll entry point through `addLayer { track: 2, frame: 'full', audio: null }`/`removeLayer { track: 2 }`, keeping the B-roll UI names for Task 8.
- Client: uploads with progress use an XHR `upload()` with `request()`'s error handling; `uploadMedia` and `addSource` use it.
- Client: one file dropped with nothing staged imports at once; two or more are staged, sorted by name with natural numbering, reorderable by drag or ↑/↓, removable, and imported with **Import N files**.
- Client: `runImport` uploads one file at a time: the first successful upload creates the project, the rest append, a failed file is marked and skipped, and failures show in the editor's banner.
- Client: Insert → **Add video…** runs the same queue on the open project, with rows above the viewer; failed rows stay until dismissed and the item is disabled while an add runs.
- Client: the timeline's hover preview stays source 0's sprite and shows no frame past the first file rather than a wrong one.
- Layers UI: `LayerDialog` replaces `BrollDialog` (deleted), follows `AudioDialog`'s add/edit shape, and when editing hides the picker and offset, since `SetLayer` changes only track, frame and sound.
- Layers UI: `OverlayRef` moves to `selection.ts` as `{ kind: 'layer'; track; start } | { kind: 'audio'; start }`, with `overlayKey` (`v2:12.5`, `audio:0`) shared by bars and the toolbar anchor, and `sameOverlay` comparing the track too.
- Layers UI: the asset picker lists "This project's videos" above the uploads through `sourceAsset(s)` (id = `mediaId`), so a layer from a source stores `media = SourceView.mediaId` with `offset` in that file's seconds.
- Layers UI: Sound is a "Play its sound" checkbox plus a −30..+12 dB slider in 1 dB steps, off by default, starting at `DEFAULT_LEVEL = -6` dB when switched on, with the "applied on export" note above 0 dB.
- Layers UI: the "+ V3" row is a 10 px dashed hint, not a control; lane rows are ruler 18, V3 10 (22 with a clip), V2 22, Clips 30, Music 22, and below 600 px only the Clips lane shows.
- Layers UI: transcript layer tags are `<button>`s with `data-layer-tag` (not `data-overlay`, which would steal the toolbar anchor); clicking one selects the layer, for viewers too, and tags read `V2 · name` plus `· PiP ↗` and a speaker icon when sound is on.
- Layers UI: double-clicking a bar opens its dialog only for editors and only with the Select tool, music bars included.
- Layers UI: preview stacking uses DOM order, not `z-index`, so captions and title cards stay on top; layer videos are keyed `${track}:${start}`; a Save that moves a layer to the other track keeps it selected.
- Layers UI: preview layer sound is `min(1, 10^(dB/20))`, ducked by `DUCK` (0.25) while a word is under the playhead, matching the export; no layer (picture or sound) plays over a title card, matching the export.

- **Known leftovers from the merge.** Task 5's `export_sources` duplicates `sources::timeline` and `sources::source_file` from Task 3, so Task 5 reuses those two and doesn't add its own copy. `GET` lists sources that are on the timeline, not the registry. That means a layer placed over an undone source shows "missing file" in the preview but still exports. This is accepted and is not fixed in this plan.

## File Structure

| File                                                                            | Responsibility                                                                                                                                                                                           | Tasks      |
| ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `engine/src/types.rs`                                                           | `Source`, `Frame`, `Edit::Layer` (replacing `Edit::Broll`), `range()`                                                                                                                                    | 1          |
| `engine/src/ops.rs`                                                             | `AddSource`/`AddLayer`/`SetLayer`/`RemoveLayer`, B-roll fold onto layers, `ProjectDoc.sources`, permanent joins                                                                                          | 1          |
| `engine/src/editlist.rs`                                                        | `all_sources`, `stitched_duration`, `locate`, `stitch_words`; `broll_windows` then `layer_windows`                                                                                                       | 1, 10      |
| `engine/src/speakers.rs`                                                        | `stitch_speakers`, the one speaker base rule                                                                                                                                                             | 1          |
| `engine/src/ffmpeg.rs`                                                          | the planner: layers read as `Edit::Layer`; `SourceInput`, `canvas`, `sources_kind`, per-source fit and joins; layer stacking, PiP and layer sound                                                        | 1, 5, 10   |
| `server/migrations/0005_sources.sql`                                            | `project_sources` registry table and its backfill                                                                                                                                                        | 3          |
| `server/src/sources.rs`                                                         | registry, `SourceView`, timeline, `add_source`, the `/sources` route, `resolve_layer_media`/`layer_media`, transcription jobs, stitched words/speakers/suggestions, `project_transcript`, `source_media` | 3, 4, 11   |
| `server/src/main.rs`                                                            | `mod sources`, `AppState::transcripts`                                                                                                                                                                   | 3, 4       |
| `server/src/app.rs`                                                             | `POST /api/projects/{id}/sources` route                                                                                                                                                                  | 3          |
| `server/src/db.rs`                                                              | schema and backfill tests                                                                                                                                                                                | 3          |
| `server/src/projects.rs`                                                        | registry row in `create_project`; `adopt_orphans` skips sources                                                                                                                                          | 3          |
| `server/src/ops.rs`                                                             | compile shim; `DocState.sources`, stitched validation of every op, undo/redo rules for `AddSource`, `project.sources` in `get_project`; export test                                                      | 1, 3, 5    |
| `server/src/bus.rs`                                                             | `Hello`/`Doc` gain `sources`                                                                                                                                                                             | 3          |
| `server/src/ws.rs`                                                              | hello carries `sources`                                                                                                                                                                                  | 3          |
| `server/src/assets.rs`                                                          | compile shim; `references` and `asset_files` over layers through `resolve_layer_media`; export resolution test                                                                                           | 1, 3, 10   |
| `server/src/routes.rs`                                                          | `store_upload` cleanup, `/thumbnails` per media; stitched `transcribe`/`speakers`/`suggest`; `export_sources`, `source_meta`, `start_export` over every source                                           | 3, 4, 5    |
| `server/src/test_util.rs`                                                       | `seed_words`, the `transcripts` field, no whisper in tests                                                                                                                                               | 3, 4       |
| `server/src/mcp.rs`                                                             | compile shim; agent transcript across sources; `add_source`, `add_layer`, `set_layer`, `add_broll` alias, `list_clips` sources                                                                           | 1, 4, 11   |
| `client/src/types.ts`                                                           | `Source`, `Frame`, `LayerTrack`, `LayerEdit`, `SourceView`, `TranscriptStatus`                                                                                                                           | 2          |
| `client/src/editlist.ts` (+ test)                                               | `stitchedDuration`, `locate`, `sourceJoins`, `isJoin` mirror                                                                                                                                             | 2          |
| `client/src/editor.ts` (+ test)                                                 | `sources`, stitched `duration`, `setWords`, `addSource`, `addLayer`, `setLayer`, `removeLayer`                                                                                                           | 2          |
| `client/src/ops.ts` (+ test)                                                    | the four new op shapes, `DocState.sources`, no `unsplit` at a join                                                                                                                                       | 2          |
| `client/src/overlays.ts` (+ test)                                               | `layers`, `layerAt`; then `layersOn`, `layersAt`, `FRAMES`, `frameLabel`, `layerTag`, `layerVolume`, `pipPlacement`, `mediaName`, `mediaUrl`                                                             | 2, 8       |
| `client/src/selection.ts` (+ test)                                              | Delete removes a layer; then `OverlayRef`, `overlayKey`, `sameOverlay`                                                                                                                                   | 2, 8       |
| `client/src/api.ts` (+ test)                                                    | XHR `upload()` with progress, `uploadMedia`, `addSource`, `transcribeProject`                                                                                                                            | 2          |
| `client/src/sources.ts` (+ test)                                                | `sourceViewsOf`, `playableSources`, `unlistedSources`, `isTranscribing`                                                                                                                                  | 2          |
| `client/src/realtime.ts`, `client/src/useRealtime.ts`                           | `sources` on `hello`/`doc`/`resync` and `RemoteDoc`                                                                                                                                                      | 2          |
| `client/src/importQueue.ts` (+ test)                                            | multi-file import queue: sort, move, `runImport`, `importFailures`                                                                                                                                       | 6          |
| `client/src/stitchedMedia.ts` (+ test)                                          | the stitched clock over one `<video>`, canvas aspect, preloading                                                                                                                                         | 7          |
| `client/src/test/fakeVideo.ts`                                                  | a fake `<video>` for the clock tests                                                                                                                                                                     | 7          |
| `client/src/usePlayback.ts` (+ test)                                            | `PlaybackMedia` in place of `HTMLMediaElement`; playback across files                                                                                                                                    | 7          |
| `client/src/App.tsx`                                                            | layer entry points, source views, imports, Add video, the clock, polling, fresh words, the Layer dialog and selection                                                                                    | 2, 6, 7, 8 |
| `client/src/App.module.css`                                                     | `.uploads` rows above the viewer                                                                                                                                                                         | 6          |
| `client/src/components/Dropzone.tsx` (+ `Dropzone.test.tsx`), `Home.module.css` | multi-file staging and import                                                                                                                                                                            | 6          |
| `client/src/components/ImportList.tsx` + `.module.css`                          | the import rows: reorder, progress, errors                                                                                                                                                               | 6          |
| `client/src/components/TopBar.tsx` (+ test)                                     | Insert → Add video…; Insert → Layer… (`onAddLayer`)                                                                                                                                                      | 6, 8       |
| `client/src/components/Player.tsx` + `.module.css`                              | plays through `StitchedMedia` in a canvas-shaped frame; passes `sources` to the layer stack                                                                                                              | 7, 9       |
| `client/src/components/Timeline.tsx` + `.module.css` (+ test)                   | track-2 layers; source badges and joins; V3/V2/Clips/Music lanes and layer bars; hover guard                                                                                                             | 2, 7, 8, 9 |
| `client/src/components/Transcript.tsx` + `.module.css` (+ test)                 | track-2 layers; transcribing blocks and join labels; layer tags                                                                                                                                          | 2, 7, 8    |
| `client/src/components/Overlays.tsx` + `.module.css` (+ `Overlays.test.tsx`)    | track-2 layer; then the stacked layer preview with PiP and sound                                                                                                                                         | 2, 9       |
| `client/src/components/AssetPicker.tsx` + `.module.css`                         | "This project's videos" group, `sourceAsset`                                                                                                                                                             | 8          |
| `client/src/components/LayerDialog.tsx` + `.module.css` (+ test)                | add or change a layer: track, frame, sound                                                                                                                                                               | 8          |
| `client/src/components/SpeakerIcon.tsx`                                         | the "sound on" icon for bars and tags                                                                                                                                                                    | 8          |
| `client/src/components/SelectionToolbar.tsx` (+ test)                           | Layer button; anchors on `overlayKey`                                                                                                                                                                    | 8          |
| `client/src/components/BrollDialog.tsx` + `.module.css`                         | deleted                                                                                                                                                                                                  | 8          |
| `README.md`                                                                     | Agents table and multi-source paragraph; screenshot alt text                                                                                                                                             | 11, 12     |
| `docs/screenshot.png`                                                           | README screenshot of the new lanes                                                                                                                                                                       | 12         |

---

### Task 1: Engine — sources, locate, stitched words, Layer edit and ops

This task changes only the engine's model and fold. Nothing on screen changes, and export output is byte-for-byte the same for every existing log. `Edit::Broll` becomes `Edit::Layer` on track 2, the project learns about appended sources and their joins, and the engine gains the stitched-time helpers that the server, the export planner and the client mirror all build on.

**Files:**

- Modify: `engine/src/types.rs` (add `Source` and `Frame`, add `Edit::Layer`, remove `Edit::Broll`, update `range()`, add a test module)
- Modify: `engine/src/ops.rs` (add the ops `AddSource`, `AddLayer`, `SetLayer` and `RemoveLayer`, fold `AddBroll`/`RemoveBroll` onto layers, add `ProjectDoc.sources`, make joins permanent, add tests)
- Modify: `engine/src/editlist.rs` (`broll_windows` selects layers; add `all_sources`, `stitched_duration`, `locate` and `stitch_words`; add tests)
- Modify: `engine/src/ffmpeg.rs` (the planner reads `Edit::Layer`; update tests and add one)
- Modify: `engine/src/speakers.rs` (add `stitch_speakers` and tests)
- Modify, compile shims only: `server/src/ops.rs`, `server/src/assets.rs`, `server/src/mcp.rs`

**Interfaces:**

- Consumes (existing): `editlist::EPS`, `editlist::piece_starts`, `editlist::overlay_windows` (through `ranges_of`), `types::{Word, Range, Edit}`, `ops::{fold, apply_op, SeqOp}`.
- Produces:

  ```rust
  // engine/src/types.rs
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct Source { pub media: String, pub offset: f64, pub duration: f64 }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub enum Frame { Full, PipTopLeft, PipTopRight, PipBottomLeft, PipBottomRight }

  // Edit (tag "layer"); Edit::Broll is removed.
  Edit::Layer { track: u8, start: f64, end: f64, media: String, offset: f64,
                frame: Frame, audio: Option<f64> }

  // engine/src/ops.rs
  Op::AddSource { media: String, offset: f64, duration: f64 }                // "addsource"
  Op::AddLayer { track: u8, start: f64, end: f64, media: String, offset: f64,
                 frame: Frame, audio: Option<f64> }                           // "addlayer"
  Op::SetLayer { track: u8, start: f64, to_track: u8, frame: Frame,
                 audio: Option<f64> }                                         // "setlayer", field "toTrack"
  Op::RemoveLayer { track: u8, start: f64 }                                   // "removelayer"
  pub struct ProjectDoc { pub edits: Vec<Edit>, pub speaker_names: Vec<String>, pub transition: Transition,
                         pub splits: Vec<f64>, pub order: Vec<f64>,
                         pub sources: Vec<Source> }                        // new: JSON "sources", default []

  // engine/src/editlist.rs
  pub fn all_sources(first: Source, doc_sources: &[Source]) -> Vec<Source>;
  pub fn stitched_duration(sources: &[Source]) -> f64;
  pub fn locate(sources: &[Source], t: f64) -> Option<(usize, f64)>;
  pub fn stitch_words(parts: &[(&Source, &[Word])]) -> Vec<Word>;
  pub fn broll_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<Window>>; // now: every Edit::Layer

  // engine/src/speakers.rs
  pub fn stitch_speakers(parts: &[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>);
  ```

  The exact JSON for each new shape, which the client mirrors in Task 2:

  ```json
  {"kind":"layer","track":3,"start":1.0,"end":2.5,"media":"asset-1","offset":0.5,"frame":"pipTopRight","audio":-6.0}
  {"kind":"layer","track":2,"start":0.0,"end":1.0,"media":"asset-1","offset":0.0,"frame":"full","audio":null}
  {"kind":"addsource","media":"m2","offset":60.0,"duration":30.5}
  {"kind":"addlayer","track":3,"start":1.0,"end":4.0,"media":"asset-1","offset":2.0,"frame":"pipBottomRight","audio":-6.0}
  {"kind":"setlayer","track":2,"start":1.0,"toTrack":3,"frame":"full","audio":null}
  {"kind":"removelayer","track":2,"start":1.0}
  {"media":"m2","offset":10.0,"duration":5.0}                       // Source, inside ProjectDoc.sources
  ```

  Frame values: `"full"`, `"pipTopLeft"`, `"pipTopRight"`, `"pipBottomLeft"`, `"pipBottomRight"`.

Every command below assumes cargo is on `PATH`: `export PATH="$HOME/.cargo/bin:$PATH"`.

- [ ] **Step 1: Write failing tests for `Source`, `Frame` and `Edit::Layer`**

Append to the end of `engine/src/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layer() -> Edit {
        Edit::Layer {
            track: 3,
            start: 1.0,
            end: 2.5,
            media: "asset-1".into(),
            offset: 0.5,
            frame: Frame::PipTopRight,
            audio: Some(-6.0),
        }
    }

    #[test]
    fn layer_edit_serialises_with_the_layer_tag() {
        let json = serde_json::to_value(layer()).unwrap();
        assert_eq!(
            json,
            json!({
                "kind": "layer", "track": 3, "start": 1.0, "end": 2.5,
                "media": "asset-1", "offset": 0.5, "frame": "pipTopRight", "audio": -6.0
            })
        );
        let back: Edit = serde_json::from_value(json).unwrap();
        assert_eq!(back, layer());
    }

    #[test]
    fn a_muted_layer_writes_audio_null() {
        let muted = Edit::Layer {
            track: 2,
            start: 0.0,
            end: 1.0,
            media: "asset-1".into(),
            offset: 0.0,
            frame: Frame::Full,
            audio: None,
        };
        assert_eq!(
            serde_json::to_value(&muted).unwrap(),
            json!({
                "kind": "layer", "track": 2, "start": 0.0, "end": 1.0,
                "media": "asset-1", "offset": 0.0, "frame": "full", "audio": null
            })
        );
    }

    #[test]
    fn layer_offset_and_audio_default_when_missing() {
        let e: Edit = serde_json::from_str(
            r#"{"kind":"layer","track":2,"start":0,"end":1,"media":"m","frame":"full"}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 1.0,
                media: "m".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            }
        );
    }

    #[test]
    fn frame_names_are_camel_case() {
        for (frame, name) in [
            (Frame::Full, "full"),
            (Frame::PipTopLeft, "pipTopLeft"),
            (Frame::PipTopRight, "pipTopRight"),
            (Frame::PipBottomLeft, "pipBottomLeft"),
            (Frame::PipBottomRight, "pipBottomRight"),
        ] {
            assert_eq!(serde_json::to_value(frame).unwrap(), json!(name));
            let back: Frame = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(back, frame);
        }
    }

    #[test]
    fn source_serialises_flat() {
        let s = Source {
            media: "m2".into(),
            offset: 10.0,
            duration: 5.0,
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            json!({ "media": "m2", "offset": 10.0, "duration": 5.0 })
        );
        assert_eq!(serde_json::from_value::<Source>(json).unwrap(), s);
    }

    #[test]
    fn layer_range_is_its_span_and_broll_edits_are_gone() {
        assert_eq!(layer().range(), Range::new(1.0, 2.5));
        // Folded docs are never persisted, so no stored JSON holds "broll" edits.
        assert!(serde_json::from_str::<Edit>(
            r#"{"kind":"broll","start":0,"end":1,"media":"m","offset":0}"#
        )
        .is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p engine types::tests`
Expected: FAIL to compile: ``cannot find type `Frame` in this scope``, ``cannot find struct `Source` ``, and ``no variant named `Layer` ``.

- [ ] **Step 3: Add `Source`, `Frame` and `Edit::Layer`, and remove `Edit::Broll`**

In `engine/src/types.rs`, insert directly after the `CaptionPos` enum (before the `MAX_TITLES` comment):

```rust
/// A video on the main track, placed at a fixed stitched offset. Source 0 is
/// the project's own media at offset 0; every later one comes from an
/// `Op::AddSource` and starts where the one before it ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub media: String,
    pub offset: f64,
    pub duration: f64,
}

/// Where a layer's picture sits on the canvas: full frame, or a corner
/// picture-in-picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Frame {
    Full,
    PipTopLeft,
    PipTopRight,
    PipBottomLeft,
    PipBottomRight,
}
```

In the same file, replace the whole `Broll` variant of `Edit`:

```rust
    /// Show asset `media` from `offset` over main-source `[start, end)`; the main audio continues.
    Broll {
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
    },
```

with:

```rust
    /// Show `media` from `offset` on video track `track` (2 or 3) over
    /// main-track `[start, end)`, placed by `frame`. Upper tracks cover lower
    /// ones. `audio` is `None` when muted, else the layer's level in dB; the
    /// main audio always continues.
    Layer {
        track: u8,
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
        frame: Frame,
        audio: Option<f64>,
    },
```

Then, in `impl Edit`'s `range()`, replace the line `| Edit::Broll { start, end, .. }` with `| Edit::Layer { start, end, .. }`. `range()` becomes:

```rust
    pub fn range(&self) -> Range {
        match *self {
            Edit::Cut { start, end, .. }
            | Edit::Overdub { start, end, .. }
            | Edit::Caption { start, end, .. }
            | Edit::Layer { start, end, .. }
            | Edit::Audio { start, end, .. } => Range::new(start, end),
            Edit::Title { at, .. } => Range::new(at, at),
        }
    }
```

Also change the comment above `MAX_SPLITS` from `/// Upper bounds on splits, B-roll and audio inserts, for the same reason.` to `/// Upper bounds on splits, track-2 layers (B-roll) and audio inserts, for the same reason.`

- [ ] **Step 4: Fold `AddBroll`/`RemoveBroll` onto layers**

In `engine/src/ops.rs`, change the types import to:

```rust
use crate::types::{default_duck, CaptionPos, Edit, Frame, Range, TitleStyle, Transition};
```

Add these helpers directly after the existing `fn overlaps`:

```rust
/// Add a layer, replacing every layer it overlaps on its own track, which is
/// the old B-roll rule applied per track.
fn add_layer(doc: &mut ProjectDoc, layer: Edit) {
    let Edit::Layer { track, .. } = &layer else {
        return;
    };
    let (track, span) = (*track, layer.range());
    doc.edits.retain(
        |e| !matches!(e, Edit::Layer { track: t, .. } if *t == track && overlaps(e.range(), span)),
    );
    doc.edits.push(layer);
}

/// Remove the layer on `track` that starts at `start`.
fn remove_layer(doc: &mut ProjectDoc, track: u8, start: f64) {
    doc.edits.retain(|e| {
        !matches!(e, Edit::Layer { track: t, start: s, .. } if *t == track && (s - start).abs() < EPS)
    });
}
```

In `apply_op`, replace the two arms `Op::AddBroll { .. } => { .. }` and `Op::RemoveBroll { start } => doc.edits.retain(..)` with:

```rust
        // B-roll is a muted, full-frame layer on track 2; old logs replay unchanged.
        Op::AddBroll {
            start,
            end,
            media,
            offset,
        } => add_layer(
            doc,
            Edit::Layer {
                track: 2,
                start: *start,
                end: *end,
                media: media.clone(),
                offset: *offset,
                frame: Frame::Full,
                audio: None,
            },
        ),
        Op::RemoveBroll { start } => remove_layer(doc, 2, *start),
```

In the `tests` module of `engine/src/ops.rs`, change its import to `use crate::types::{CaptionPos, Frame, TitleStyle, Transition};`. In `broll_replaces_overlaps_and_audio_edits_by_start`, replace the first assertion:

```rust
        assert_eq!(
            doc.edits,
            vec![Edit::Broll {
                start: 2.0,
                end: 4.0,
                media: "asset-1".into(),
                offset: 0.0
            }]
        );
```

with:

```rust
        assert_eq!(
            doc.edits,
            vec![Edit::Layer {
                track: 2,
                start: 2.0,
                end: 4.0,
                media: "asset-1".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            }]
        );
```

- [ ] **Step 5: Point the edit-list and the ffmpeg planner at layers**

In `engine/src/editlist.rs`, replace `broll_windows` with:

```rust
/// Per segment, every layer overlay that intersects it, in edit order. Every
/// layer, whatever its track, frame or sound, is drawn as full-frame B-roll
/// until the planner learns tracks.
pub fn broll_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<Window>> {
    overlay_windows(
        segments,
        &ranges_of(edits, |e| matches!(e, Edit::Layer { .. })),
    )
}
```

In the `tests` module of `engine/src/editlist.rs`, change the import to `use crate::types::{CaptionPos, Frame, TitleStyle, Transition};`. Then replace each of the two `Edit::Broll { .. }` literals (one in the test that ends with `assert_eq!(broll_windows(&tl, &edits)[0][0].index, 0);`, one in `music_spans_a_title_but_captions_and_broll_do_not`). The first becomes:

```rust
            Edit::Layer {
                track: 2,
                start: 1.0,
                end: 2.0,
                media: "b".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
```

The second becomes:

```rust
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 10.0,
                media: "b".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
```

In `engine/src/ffmpeg.rs`, in `build_ffmpeg_args`, replace the input loop's

```rust
                let Edit::Broll { media, .. } = &edits[w.index] else {
```

with

```rust
                let Edit::Layer { media, .. } = &edits[w.index] else {
```

and replace the overlay loop's

```rust
                let Edit::Broll { start, offset, .. } = &edits[w.index] else {
```

with

```rust
                let Edit::Layer { start, offset, .. } = &edits[w.index] else {
```

In the `tests` module of `engine/src/ffmpeg.rs`, change the import to `use crate::types::{CaptionPos, Frame, TitleStyle};` and replace the four `Edit::Broll` literals.

In `broll_is_trimmed_delayed_scaled_and_overlaid_under_captions`:

```rust
        let edits = [Edit::Layer {
            track: 2,
            start: 2.0,
            end: 4.0,
            media: "b1".into(),
            offset: 1.5,
            frame: Frame::Full,
            audio: None,
        }];
```

In `broll_across_a_reordered_boundary_offsets_into_the_asset`:

```rust
        let edits = [Edit::Layer {
            track: 2,
            start: 4.0,
            end: 6.0,
            media: "b1".into(),
            offset: 0.0,
            frame: Frame::Full,
            audio: None,
        }];
```

In `a_caption_or_broll_spanning_a_title_draws_nothing_on_the_card`:

```rust
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 10.0,
                media: "b1".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
```

In `audio_only_export_mixes_music_and_ignores_broll`:

```rust
            Edit::Layer {
                track: 2,
                start: 1.0,
                end: 2.0,
                media: "b1".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
```

Then add this test after `broll_across_a_reordered_boundary_offsets_into_the_asset`. It pins today's behaviour, and Task 10 rewrites it:

```rust
    #[test]
    fn a_layer_on_any_track_exports_full_frame_and_muted_for_now() {
        let edits = [Edit::Layer {
            track: 3,
            start: 2.0,
            end: 4.0,
            media: "b1".into(),
            offset: 1.5,
            frame: Frame::PipTopRight,
            audio: Some(-6.0),
        }];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4")]);
        })
        .unwrap();
        let g = filter_complex(&args);
        assert!(g.contains("[1:v]trim=start=1.5:end=3.5,setpts=PTS-STARTPTS+2/TB,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1[v0b0];"), "{g}");
        assert!(
            g.contains("[v0bo0][v0b0]overlay=x=0:y=0:eof_action=pass:enable='between(t,2,4)'"),
            "{g}"
        );
        assert!(!g.contains("[1:a]"), "layer sound is not mixed yet: {g}");
    }
```

- [ ] **Step 6: Keep the server compiling against `Edit::Layer`**

`server/src/assets.rs`, in `references`, replace `Edit::Broll { media, .. } if media == asset_id => (b + 1, a),` with:

```rust
        Edit::Layer { media, .. } if media == asset_id => (b + 1, a),
```

In `asset_files` in the same file, replace `let (Edit::Broll { media, .. } | Edit::Audio { media, .. }) = edit else {` with:

```rust
        let (Edit::Layer { media, .. } | Edit::Audio { media, .. }) = edit else {
```

In the `references_count_by_kind` test, replace the `Edit::Broll { .. }` literal with:

```rust
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 1.0,
                media: "a".into(),
                offset: 0.0,
                frame: engine::Frame::Full,
                audio: None,
            },
```

`server/src/ops.rs`: in `validate`'s `Op::AddBroll` arm, replace `.filter(|e| matches!(e, Edit::Broll { .. }))` with:

```rust
                .filter(|e| matches!(e, Edit::Layer { track: 2, .. }))
```

`server/src/mcp.rs`: in `add_broll_and_add_audio_take_project_assets`, replace `assert!(matches!(&doc.edits[0], Edit::Broll { offset, .. } if *offset == 1.0));` with:

```rust
        assert!(matches!(&doc.edits[0], Edit::Layer { track: 2, offset, .. } if *offset == 1.0));
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p engine types::tests && cargo test`
Expected: PASS. The six new `types::tests` pass, `a_layer_on_any_track_exports_full_frame_and_muted_for_now` passes, and every existing engine and server test still passes. The B-roll filter strings are unchanged. If a B-roll ffmpeg assertion fails, the planner edit is wrong. Fix the planner, never the expected string.

- [ ] **Step 8: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add engine/src/types.rs engine/src/ops.rs engine/src/editlist.rs engine/src/ffmpeg.rs server/src/assets.rs server/src/ops.rs server/src/mcp.rs
git commit -m "engine: replace the B-roll edit with a Layer on track 2; export unchanged" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

- [ ] **Step 9: Write failing tests for the new ops and `ProjectDoc.sources`**

Append these tests inside the `tests` module of `engine/src/ops.rs`, before its closing brace:

```rust
    fn layer_op(track: u8, start: f64, end: f64) -> Op {
        Op::AddLayer {
            track,
            start,
            end,
            media: "asset-1".into(),
            offset: 0.0,
            frame: Frame::Full,
            audio: None,
        }
    }

    fn layer_spans(doc: &ProjectDoc) -> Vec<(u8, f64, f64)> {
        doc.edits
            .iter()
            .filter_map(|e| match e {
                Edit::Layer {
                    track, start, end, ..
                } => Some((*track, *start, *end)),
                _ => None,
            })
            .collect()
    }

    fn add_source(offset: f64, duration: f64) -> Op {
        Op::AddSource {
            media: "m2".into(),
            offset,
            duration,
        }
    }

    #[test]
    fn add_source_appends_the_source_and_splits_at_its_offset() {
        let doc = fold(&[
            op(1, Op::Split { at: 4.0 }),
            op(2, add_source(10.0, 5.0)),
            op(
                3,
                Op::AddSource {
                    media: "m3".into(),
                    offset: 15.0,
                    duration: 2.5,
                },
            ),
        ]);
        assert_eq!(
            doc.sources,
            vec![
                Source {
                    media: "m2".into(),
                    offset: 10.0,
                    duration: 5.0
                },
                Source {
                    media: "m3".into(),
                    offset: 15.0,
                    duration: 2.5
                },
            ]
        );
        assert_eq!(doc.splits, vec![4.0, 10.0, 15.0]);
    }

    #[test]
    fn undoing_add_source_removes_the_source_and_its_split() {
        let doc = fold(&[op(1, Op::Split { at: 4.0 }), undone(2, add_source(10.0, 5.0))]);
        assert!(doc.sources.is_empty());
        assert_eq!(doc.splits, vec![4.0]);
    }

    #[test]
    fn a_join_cannot_be_unsplit_but_other_splits_can() {
        let doc = fold(&[
            op(1, add_source(10.0, 5.0)),
            op(2, Op::Split { at: 12.0 }),
            op(3, Op::Unsplit { at: 10.0 }),
            op(4, Op::Unsplit { at: 12.0 }),
        ]);
        assert_eq!(doc.splits, vec![10.0]);
        // A split already at the join is not duplicated.
        let doc = fold(&[op(1, add_source(10.0, 5.0)), op(2, Op::Split { at: 10.0 })]);
        assert_eq!(doc.splits, vec![10.0]);
    }

    #[test]
    fn a_join_is_a_piece_start_that_moves_like_any_other() {
        let doc = fold(&[
            op(1, add_source(10.0, 5.0)),
            op(
                2,
                Op::Move {
                    piece: 10.0,
                    before: Some(0.0),
                },
            ),
        ]);
        assert_eq!(doc.order, vec![10.0, 0.0]);
    }

    #[test]
    fn add_layer_replaces_overlaps_on_its_own_track_only() {
        let doc = fold(&[
            op(1, layer_op(2, 1.0, 3.0)),
            op(2, layer_op(3, 1.0, 3.0)),
            op(3, layer_op(2, 2.0, 4.0)),
            op(4, layer_op(2, 5.0, 6.0)),
        ]);
        assert_eq!(
            layer_spans(&doc),
            vec![(3, 1.0, 3.0), (2, 2.0, 4.0), (2, 5.0, 6.0)]
        );
    }

    #[test]
    fn add_broll_folds_to_a_muted_full_frame_track_two_layer() {
        let doc = fold(&[
            op(1, layer_op(3, 1.0, 2.0)),
            op(
                2,
                Op::AddBroll {
                    start: 1.0,
                    end: 2.0,
                    media: "clip".into(),
                    offset: 0.5,
                },
            ),
        ]);
        assert_eq!(
            doc.edits[1],
            Edit::Layer {
                track: 2,
                start: 1.0,
                end: 2.0,
                media: "clip".into(),
                offset: 0.5,
                frame: Frame::Full,
                audio: None,
            }
        );
        // RemoveBroll only ever touches track 2.
        let doc = fold(&[
            op(1, layer_op(3, 1.0, 2.0)),
            op(2, broll(1.0, 2.0)),
            op(3, Op::RemoveBroll { start: 1.0 }),
        ]);
        assert_eq!(layer_spans(&doc), vec![(3, 1.0, 2.0)]);
    }

    #[test]
    fn set_layer_moves_track_and_sets_frame_and_audio_in_place() {
        let doc = fold(&[
            op(1, layer_op(2, 1.0, 3.0)),
            op(2, cut(8.0, 9.0)),
            op(3, layer_op(3, 2.0, 4.0)),
            op(4, layer_op(3, 6.0, 7.0)),
            op(
                5,
                Op::SetLayer {
                    track: 2,
                    start: 1.0,
                    to_track: 3,
                    frame: Frame::PipBottomLeft,
                    audio: Some(-3.0),
                },
            ),
        ]);
        // It keeps its place in the list; the V3 layer it now overlaps is gone.
        assert_eq!(
            doc.edits[0],
            Edit::Layer {
                track: 3,
                start: 1.0,
                end: 3.0,
                media: "asset-1".into(),
                offset: 0.0,
                frame: Frame::PipBottomLeft,
                audio: Some(-3.0),
            }
        );
        assert_eq!(layer_spans(&doc), vec![(3, 1.0, 3.0), (3, 6.0, 7.0)]);
        assert!(doc.edits[1].is_cut());
    }

    #[test]
    fn set_layer_on_a_missing_layer_is_a_no_op() {
        let before = fold(&[op(1, layer_op(2, 1.0, 3.0))]);
        let after = fold(&[
            op(1, layer_op(2, 1.0, 3.0)),
            op(
                2,
                Op::SetLayer {
                    track: 3,
                    start: 1.0,
                    to_track: 2,
                    frame: Frame::Full,
                    audio: None,
                },
            ),
        ]);
        assert_eq!(before, after);
    }

    #[test]
    fn remove_layer_matches_track_and_start() {
        let doc = fold(&[
            op(1, layer_op(2, 1.0, 2.0)),
            op(2, layer_op(3, 1.0, 2.0)),
            op(3, Op::RemoveLayer { track: 3, start: 1.0 }),
            op(4, Op::RemoveLayer { track: 2, start: 9.0 }),
        ]);
        assert_eq!(layer_spans(&doc), vec![(2, 1.0, 2.0)]);
    }

    #[test]
    fn source_and_layer_ops_serialise_to_the_client_shape() {
        let cases = [
            (
                Op::AddSource {
                    media: "m2".into(),
                    offset: 60.0,
                    duration: 30.5,
                },
                serde_json::json!({ "kind": "addsource", "media": "m2", "offset": 60.0, "duration": 30.5 }),
            ),
            (
                Op::AddLayer {
                    track: 3,
                    start: 1.0,
                    end: 4.0,
                    media: "asset-1".into(),
                    offset: 2.0,
                    frame: Frame::PipBottomRight,
                    audio: Some(-6.0),
                },
                serde_json::json!({
                    "kind": "addlayer", "track": 3, "start": 1.0, "end": 4.0, "media": "asset-1",
                    "offset": 2.0, "frame": "pipBottomRight", "audio": -6.0
                }),
            ),
            (
                Op::SetLayer {
                    track: 2,
                    start: 1.0,
                    to_track: 3,
                    frame: Frame::Full,
                    audio: None,
                },
                serde_json::json!({
                    "kind": "setlayer", "track": 2, "start": 1.0, "toTrack": 3,
                    "frame": "full", "audio": null
                }),
            ),
            (
                Op::RemoveLayer {
                    track: 2,
                    start: 1.0,
                },
                serde_json::json!({ "kind": "removelayer", "track": 2, "start": 1.0 }),
            ),
        ];
        for (op, json) in cases {
            assert_eq!(serde_json::to_value(&op).unwrap(), json);
            assert_eq!(serde_json::from_value::<Op>(json).unwrap(), op);
        }
        // Offset and audio may be omitted on the way in.
        let op: Op = serde_json::from_str(
            r#"{"kind":"addlayer","track":2,"start":0,"end":1,"media":"m","frame":"full"}"#,
        )
        .unwrap();
        assert_eq!(
            op,
            Op::AddLayer {
                track: 2,
                start: 0.0,
                end: 1.0,
                media: "m".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            }
        );
        // Stored B-roll ops still read.
        let op: Op =
            serde_json::from_str(r#"{"kind":"addbroll","start":1,"end":2,"media":"m"}"#).unwrap();
        assert!(matches!(op, Op::AddBroll { offset, .. } if offset == 0.0));
    }

    #[test]
    fn project_doc_carries_sources_and_reads_old_docs() {
        let doc = fold(&[op(1, add_source(10.0, 5.0))]);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            json["sources"],
            serde_json::json!([{ "media": "m2", "offset": 10.0, "duration": 5.0 }])
        );
        assert_eq!(json["splits"], serde_json::json!([10.0]));
        let old: ProjectDoc = serde_json::from_str(r#"{"edits":[],"speakerNames":[]}"#).unwrap();
        assert!(old.sources.is_empty());
    }
```

- [ ] **Step 10: Run the tests to verify they fail**

Run: `cargo test -p engine ops::tests`
Expected: FAIL to compile. There is ``no variant named `AddSource` `` (and `AddLayer`, `SetLayer`, `RemoveLayer`) on `Op`, ``no field `sources` on type `ProjectDoc` ``, and ``cannot find struct `Source` `` in the test module.

- [ ] **Step 11: Add the ops, `ProjectDoc.sources` and the fold rules**

In `engine/src/ops.rs`, change the types import to:

```rust
use crate::types::{default_duck, CaptionPos, Edit, Frame, Range, Source, TitleStyle, Transition};
```

Add these variants to `enum Op`, after `RemoveAudio`:

```rust
    /// Append a video to the main track at stitched `offset` (the current
    /// stitched end). The fold adds a permanent split there.
    #[serde(rename_all = "camelCase")]
    AddSource {
        media: String,
        offset: f64,
        duration: f64,
    },
    /// `audio: None` is muted; `Some(db)` mixes the layer's sound at that level.
    #[serde(rename_all = "camelCase")]
    AddLayer {
        track: u8,
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
        frame: Frame,
        audio: Option<f64>,
    },
    /// Move the layer on `track` starting at `start` to `to_track` and set
    /// its frame and sound.
    #[serde(rename_all = "camelCase")]
    SetLayer {
        track: u8,
        start: f64,
        to_track: u8,
        frame: Frame,
        audio: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    RemoveLayer {
        track: u8,
        start: f64,
    },
```

Add this field to `ProjectDoc`, after `order`:

```rust
    /// Videos appended by `AddSource`, in stitched order. The project's own
    /// media is source 0 and is *not* listed here; see `all_sources`.
    #[serde(default)]
    pub sources: Vec<Source>,
```

Add two helpers after `remove_layer`:

```rust
/// Add a split at `at` unless one is already there, keeping `splits` sorted.
fn add_split(doc: &mut ProjectDoc, at: f64) {
    if !doc.splits.iter().any(|s| (s - at).abs() < EPS) {
        doc.splits.push(at);
        doc.splits.sort_by(f64::total_cmp);
    }
}

/// Whether `at` is where an appended source begins: a permanent split.
fn is_join(doc: &ProjectDoc, at: f64) -> bool {
    doc.sources.iter().any(|s| (s.offset - at).abs() < EPS)
}
```

In `apply_op`, replace the `Op::Split` and `Op::Unsplit` arms with:

```rust
        Op::Split { at } => add_split(doc, *at),
        // A join between two sources stays split, so no piece spans two files.
        Op::Unsplit { at } => {
            if !is_join(doc, *at) {
                doc.splits.retain(|s| (s - at).abs() >= EPS);
            }
        }
```

Add these arms at the end of `apply_op`'s match, after `Op::RemoveAudio`:

```rust
        Op::AddSource {
            media,
            offset,
            duration,
        } => {
            doc.sources.push(Source {
                media: media.clone(),
                offset: *offset,
                duration: *duration,
            });
            add_split(doc, *offset);
        }
        Op::AddLayer {
            track,
            start,
            end,
            media,
            offset,
            frame,
            audio,
        } => add_layer(
            doc,
            Edit::Layer {
                track: *track,
                start: *start,
                end: *end,
                media: media.clone(),
                offset: *offset,
                frame: *frame,
                audio: *audio,
            },
        ),
        Op::SetLayer {
            track,
            start,
            to_track,
            frame,
            audio,
        } => {
            let Some(i) = doc.edits.iter().position(|e| {
                matches!(e, Edit::Layer { track: t, start: s, .. } if t == track && (s - start).abs() < EPS)
            }) else {
                return;
            };
            if let Edit::Layer {
                track: t,
                frame: f,
                audio: a,
                ..
            } = &mut doc.edits[i]
            {
                *t = *to_track;
                *f = *frame;
                *a = *audio;
            }
            // Edited in place; any other layer it now overlaps on its new track goes.
            let span = doc.edits[i].range();
            let mut k = 0;
            doc.edits.retain(|e| {
                let clash = k != i
                    && matches!(e, Edit::Layer { track: t, .. } if t == to_track && overlaps(e.range(), span));
                k += 1;
                !clash
            });
        }
        Op::RemoveLayer { track, start } => remove_layer(doc, *track, *start),
```

In the `tests` module, change the types import to `use crate::types::{CaptionPos, Frame, Source, TitleStyle, Transition};`.

- [ ] **Step 12: Reject the new ops in server validation until Task 3**

`server/src/ops.rs::validate` matches `Op` exhaustively. Add this arm after `Op::RemoveAudio { start } => check_range(*start, *start),`:

```rust
        // Task 3 validates these against the stitched timeline and the source
        // registry; until then nothing may store them.
        Op::AddSource { .. } | Op::AddLayer { .. } | Op::SetLayer { .. } | Op::RemoveLayer { .. } => {
            Err(AppError::bad_request_at(index, "not supported yet"))
        }
```

- [ ] **Step 13: Run the tests to verify they pass**

Run: `cargo test -p engine ops::tests && cargo test`
Expected: PASS. The eleven new `ops::tests` pass, along with all existing engine and server tests. `split_and_unsplit_keep_splits_sorted_and_unique` must still pass unchanged. It has no sources, so `Unsplit` behaves as before.

- [ ] **Step 14: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add engine/src/ops.rs server/src/ops.rs
git commit -m "engine: AddSource with its permanent join, and the AddLayer, SetLayer and RemoveLayer ops" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

- [ ] **Step 15: Write failing tests for stitched time and stitched words**

Append these tests inside the `tests` module of `engine/src/editlist.rs`, before its closing brace:

```rust
    fn src(media: &str, offset: f64, duration: f64) -> Source {
        Source {
            media: media.into(),
            offset,
            duration,
        }
    }

    /// [0,10) [10,15) [15,17.5)
    fn three() -> Vec<Source> {
        all_sources(
            src("m1", 0.0, 10.0),
            &[src("m2", 10.0, 5.0), src("m3", 15.0, 2.5)],
        )
    }

    #[test]
    fn all_sources_puts_the_project_media_first() {
        let all = three();
        assert_eq!(
            all.iter().map(|s| s.media.as_str()).collect::<Vec<_>>(),
            vec!["m1", "m2", "m3"]
        );
        assert_eq!(all_sources(src("m1", 0.0, 4.0), &[]), vec![src("m1", 0.0, 4.0)]);
    }

    #[test]
    fn stitched_duration_is_the_last_sources_end() {
        assert_eq!(stitched_duration(&three()), 17.5);
        assert_eq!(stitched_duration(&[src("m1", 0.0, 4.0)]), 4.0);
        assert_eq!(stitched_duration(&[]), 0.0);
    }

    #[test]
    fn locate_at_every_boundary() {
        let s = three();
        assert_eq!(locate(&s, 0.0), Some((0, 0.0)), "the very start");
        assert_eq!(locate(&s, 4.25), Some((0, 4.25)), "inside the first");
        assert_eq!(locate(&s, 9.5), Some((0, 9.5)), "just before a join");
        assert_eq!(locate(&s, 10.0), Some((1, 0.0)), "a join belongs to the later source");
        assert_eq!(locate(&s, 12.0), Some((1, 2.0)), "inside the second");
        assert_eq!(locate(&s, 15.0), Some((2, 0.0)), "the second join");
        assert_eq!(locate(&s, 17.5), Some((2, 2.5)), "the exact end is the last source at its duration");
        assert_eq!(locate(&s, 17.6), None, "past the end");
        assert_eq!(locate(&s, -0.1), None, "before the start");
        assert_eq!(locate(&[], 0.0), None, "no sources");
    }

    #[test]
    fn locate_snaps_within_eps_of_a_join_or_the_end() {
        let s = three();
        assert_eq!(locate(&s, 10.0 - EPS / 2.0), Some((1, 0.0)));
        assert_eq!(locate(&s, 17.5 + EPS / 2.0), Some((2, 2.5)));
        assert_eq!(locate(&s, -EPS / 2.0), Some((0, 0.0)));
    }

    #[test]
    fn locate_on_one_source_is_the_identity() {
        let s = [src("m1", 0.0, 10.0)];
        assert_eq!(locate(&s, 0.0), Some((0, 0.0)));
        assert_eq!(locate(&s, 3.5), Some((0, 3.5)));
        assert_eq!(locate(&s, 10.0), Some((0, 10.0)));
        assert_eq!(locate(&s, 10.5), None);
    }

    #[test]
    fn stitch_words_shifts_by_offset_and_prefixes_later_ids() {
        let w = |id: &str, start: f64, end: f64| Word {
            id: id.into(),
            text: id.into(),
            start,
            end,
        };
        let s = three();
        let first = [w("w0", 0.5, 1.0), w("w1", 1.0, 1.5)];
        let third = [w("w0", 0.0, 0.25)];
        let words = stitch_words(&[(&s[0], &first), (&s[1], &[]), (&s[2], &third)]);
        assert_eq!(
            words,
            vec![
                w("w0", 0.5, 1.0),
                w("w1", 1.0, 1.5),
                Word {
                    id: "2:w0".into(),
                    text: "w0".into(),
                    start: 15.0,
                    end: 15.25,
                },
            ]
        );
        assert!(stitch_words(&[]).is_empty());
    }
```

Change the `tests` module's import to `use crate::types::{CaptionPos, Frame, Source, TitleStyle, Transition};`.

- [ ] **Step 16: Run the tests to verify they fail**

Run: `cargo test -p engine editlist::tests`
Expected: FAIL to compile: ``cannot find function `all_sources` ``, `stitched_duration`, `locate` and `stitch_words`.

- [ ] **Step 17: Implement the stitched-time helpers**

In `engine/src/editlist.rs`, change the types import to:

```rust
use crate::types::{Edit, Range, Source, Transition, Word};
```

Add after `normalize_cuts`:

```rust
/// The project's own media (source 0) followed by the sources `AddSource`
/// appended (`ProjectDoc::sources`), in stitched order.
pub fn all_sources(first: Source, doc_sources: &[Source]) -> Vec<Source> {
    let mut all = Vec::with_capacity(doc_sources.len() + 1);
    all.push(first);
    all.extend_from_slice(doc_sources);
    all
}

/// End of the stitched timeline: the last source's offset plus its duration.
/// Zero with no sources.
pub fn stitched_duration(sources: &[Source]) -> f64 {
    sources.last().map_or(0.0, |s| s.offset + s.duration)
}

/// Which source holds stitched instant `t`, and the time inside that file.
/// An instant on a join (within `EPS`) belongs to the later source; the
/// stitched end maps to the last source at its duration. `None` before 0,
/// past the end, or with no sources.
pub fn locate(sources: &[Source], t: f64) -> Option<(usize, f64)> {
    let last = sources.len().checked_sub(1)?;
    let end = stitched_duration(sources);
    if t < -EPS || t > end + EPS {
        return None;
    }
    if t >= end - EPS {
        return Some((last, sources[last].duration));
    }
    let i = sources
        .iter()
        .rposition(|s| t >= s.offset - EPS)
        .unwrap_or(0);
    let s = &sources[i];
    Some((i, (t - s.offset).clamp(0.0, s.duration)))
}

/// The project's word list: each source's words shifted by its offset, in
/// source order. Part `k` must be source `k` (pass untranscribed sources with
/// no words). Source 0's ids are unchanged, so existing logs and clients keep
/// working; source `k > 0` ids become `"{k}:{id}"`, unique across files.
pub fn stitch_words(parts: &[(&Source, &[Word])]) -> Vec<Word> {
    parts
        .iter()
        .enumerate()
        .flat_map(|(k, (source, words))| {
            words.iter().map(move |w| Word {
                id: if k == 0 {
                    w.id.clone()
                } else {
                    format!("{k}:{}", w.id)
                },
                text: w.text.clone(),
                start: w.start + source.offset,
                end: w.end + source.offset,
            })
        })
        .collect()
}
```

- [ ] **Step 18: Run the tests to verify they pass**

Run: `cargo test -p engine editlist::tests`
Expected: PASS. That covers the six new tests and every existing `editlist` test.

- [ ] **Step 19: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add engine/src/editlist.rs
git commit -m "engine: stitched sources, locate and stitched words" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

- [ ] **Step 20: Write failing tests for namespaced speakers**

Append inside the `tests` module of `engine/src/speakers.rs`, before its closing brace:

```rust
    #[test]
    fn stitched_speakers_are_namespaced_per_source() {
        let first = [Some(0), Some(1), None, Some(0)];
        let third = [Some(2), Some(0)];
        let (count, labels) = stitch_speakers(&[(2, &first), (0, &[]), (3, &third)]);
        assert_eq!(count, 5);
        assert_eq!(
            labels,
            vec![Some(0), Some(1), None, Some(0), Some(4), Some(2)]
        );
    }

    #[test]
    fn a_count_below_the_labels_never_merges_two_sources() {
        // A label of 2 with a stated count of 1: the base still clears it.
        let (count, labels) = stitch_speakers(&[(1, &[Some(0), Some(2)]), (1, &[Some(0)])]);
        assert_eq!(labels, vec![Some(0), Some(2), Some(3)]);
        assert_eq!(count, 4);
    }

    #[test]
    fn stitching_one_source_is_the_identity() {
        let words = [Some(1), None, Some(0)];
        assert_eq!(stitch_speakers(&[(2, &words)]), (2, words.to_vec()));
        assert_eq!(stitch_speakers(&[]), (0, vec![]));
    }
```

- [ ] **Step 21: Run the tests to verify they fail**

Run: `cargo test -p engine speakers::tests`
Expected: FAIL to compile: ``cannot find function `stitch_speakers` ``.

- [ ] **Step 22: Implement `stitch_speakers`**

In `engine/src/speakers.rs`, add after `assign_speakers`:

```rust
/// Speaker labels for the stitched transcript, parallel to `stitch_words`
/// over the same sources. Each part is one source's speaker count and its
/// per-word labels, in source order. A source's labels move up by the
/// speakers of every source before it, so speaker 0 in two videos stays two
/// people. Returns the stitched count and labels.
pub fn stitch_speakers(parts: &[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>) {
    let mut base = 0u32;
    let mut labels = Vec::with_capacity(parts.iter().map(|(_, w)| w.len()).sum());
    for (count, words) in parts {
        labels.extend(words.iter().map(|s| s.map(|s| s + base)));
        let used = words.iter().flatten().map(|s| s + 1).max().unwrap_or(0);
        base += (*count).max(used);
    }
    (base, labels)
}
```

- [ ] **Step 23: Run the whole suite and lint**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: PASS. That covers the three new `speakers::tests` and every engine and server test. `npm test` also runs `vitest`, and this task does not touch the client. The client still compiles, because the server only emits `kind: "layer"` at runtime, which TypeScript does not check.

- [ ] **Step 24: Commit**

```bash
git add engine/src/speakers.rs
git commit -m "engine: namespace speaker labels per source for the stitched transcript" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 2: Client model — types, api, editor ops, locate/stitch mirror

After this task the client speaks the new document: sources with a stitched duration, `Layer` edits in place of B-roll, the four new op shapes and an upload that reports progress. Nothing on screen changes. B-roll is a track-2 layer everywhere.

**Files:**

- Create: `client/src/sources.ts`, `client/src/sources.test.ts`
- Modify: `client/src/types.ts`, `client/src/editlist.ts` + `client/src/editlist.test.ts`, `client/src/editor.ts` + `client/src/editor.test.ts`, `client/src/ops.ts` + `client/src/ops.test.ts`, `client/src/overlays.ts` + `client/src/overlays.test.ts`, `client/src/selection.ts` + `client/src/selection.test.ts`, `client/src/api.ts` + `client/src/api.test.ts`, `client/src/realtime.ts`, `client/src/useRealtime.ts`, `client/src/App.tsx`, `client/src/components/Overlays.tsx`, `client/src/components/Timeline.tsx` + `client/src/components/Timeline.test.tsx`, `client/src/components/Transcript.tsx`

**Interfaces:**

- Consumes: Part A's JSON shapes (`{"kind":"layer",…}`, `addsource`/`addlayer`/`setlayer` with `toTrack`/`removelayer`, `ProjectDoc.sources`, joins in `splits`); the `SourceView` shape that Task 3's `POST /api/projects/{id}/sources` returns and `GET /api/projects/{id}` lists as `project.sources` (typed here ahead of the server, from the contract; nothing in this task calls a running server).
- Produces:

  ```ts
  // types.ts
  export interface Source { media: string; offset: number; duration: number }
  export type Frame = 'full' | 'pipTopLeft' | 'pipTopRight' | 'pipBottomLeft' | 'pipBottomRight';
  export type LayerTrack = 2 | 3;
  export interface LayerEdit extends Range { kind: 'layer'; track: LayerTrack; media: string; offset: number; frame: Frame; audio: number | null }
  export type Edit = CutEdit | OverdubEdit | TitleEdit | CaptionEdit | LayerEdit | AudioEdit;
  export type TranscriptStatus = 'pending' | 'running' | 'ready' | 'error';
  export interface SourceView { index: number; mediaId: string; url: string; filename: string; kind: MediaKind; offset: number; duration: number; transcript: TranscriptStatus }
  // ProjectSummary gains: sources?: SourceView[]
  // editlist.ts
  export function stitchedDuration(sources: readonly Pick<Source, 'offset' | 'duration'>[]): number;
  export function locate(sources: readonly Pick<Source, 'offset' | 'duration'>[], t: number): { index: number; local: number } | null;
  export function sourceJoins(sources: readonly Pick<Source, 'offset' | 'duration'>[]): number[];
  export function isJoin(at: number, sources: readonly Pick<Source, 'offset' | 'duration'>[]): boolean;
  // editor.ts — EditorState gains `sources: Source[]`; `duration` is stitched
  | { type: 'load'; words: Word[]; duration: number; media?: string }
  | { type: 'setWords'; words: Word[] }
  | { type: 'addSource'; media: string; offset: number; duration: number }
  | { type: 'addLayer'; track: LayerTrack; media: string; offset: number; frame: Frame; audio: number | null; range?: [number, number] }
  | { type: 'setLayer'; track: LayerTrack; start: number; toTrack: LayerTrack; frame: Frame; audio: number | null }
  | { type: 'removeLayer'; track: LayerTrack; start: number }
  // (addBroll / removeBroll removed; `remote` gains `sources?: Source[]`)
  // ops.ts — Op gains addsource | addlayer | setlayer | removelayer; addbroll/removebroll removed; DocState gains sources?: Source[]
  // overlays.ts
  export function layers(edits: Edit[], track?: LayerTrack): LayerEdit[];
  export function layerAt(t: number, edits: Edit[], track: LayerTrack): LayerEdit | undefined;
  // (brolls / brollAt removed)
  // api.ts
  export function upload<T>(url: string, form: FormData, onProgress?: (fraction: number) => void): Promise<T>;
  export function uploadMedia(file: File, onProgress?: (fraction: number) => void): Promise<ProjectSummary>;
  export function addSource(projectId: string, file: File, onProgress?: (fraction: number) => void): Promise<SourceView>;
  export function transcribeProject(id: string): Promise<{ words: Word[]; sources?: SourceView[] }>;
  // transcribeMedia(id) keeps its signature and returns transcribeProject(id)'s words.
  // sources.ts
  export function sourceViewsOf(project: ProjectSummary): SourceView[];
  export function playableSources(sources: Source[], views: SourceView[]): SourceView[];
  export function unlistedSources(sources: Source[], views: SourceView[]): string[];
  export function isTranscribing(view: SourceView): boolean;
  ```

- [ ] **Step 1: Failing tests for `locate`, `stitchedDuration` and joins**

Append to `client/src/editlist.test.ts`, and add `EPS`, `isJoin`, `locate`, `sourceJoins` and `stitchedDuration` to its `./editlist` import (keep the list sorted) and `Source` to its `./types` import:

```ts
describe('stitched sources (mirror engine/src/editlist.rs)', () => {
  // The engine's `three()`: 10 s, 5 s and 2.5 s files end to end.
  const three: Source[] = [
    { media: 'm1', offset: 0, duration: 10 },
    { media: 'm2', offset: 10, duration: 5 },
    { media: 'm3', offset: 15, duration: 2.5 },
  ];

  it('ends where the last source ends', () => {
    expect(stitchedDuration(three)).toBe(17.5);
    expect(stitchedDuration([])).toBe(0);
  });

  it('locates at every boundary', () => {
    expect(locate(three, 0)).toEqual({ index: 0, local: 0 }); // the very start
    expect(locate(three, 4.25)).toEqual({ index: 0, local: 4.25 }); // inside the first
    expect(locate(three, 9.5)).toEqual({ index: 0, local: 9.5 }); // just before a join
    expect(locate(three, 10)).toEqual({ index: 1, local: 0 }); // a join belongs to the later source
    expect(locate(three, 12)).toEqual({ index: 1, local: 2 }); // inside the second
    expect(locate(three, 15)).toEqual({ index: 2, local: 0 }); // the second join
    expect(locate(three, 17.5)).toEqual({ index: 2, local: 2.5 }); // the exact end: last source at its duration
    expect(locate(three, 17.6)).toBeNull(); // past the end
    expect(locate(three, -0.1)).toBeNull(); // before the start
    expect(locate([], 0)).toBeNull(); // no sources
  });

  it('snaps within EPS of a join or the end', () => {
    expect(locate(three, 10 - EPS / 2)).toEqual({ index: 1, local: 0 });
    expect(locate(three, 17.5 + EPS / 2)).toEqual({ index: 2, local: 2.5 });
    expect(locate(three, -EPS / 2)).toEqual({ index: 0, local: 0 });
  });

  it('is the identity on one source', () => {
    const one: Source[] = [{ media: 'm1', offset: 0, duration: 10 }];
    expect(locate(one, 3.5)).toEqual({ index: 0, local: 3.5 });
    expect(locate(one, 10)).toEqual({ index: 0, local: 10 });
    expect(locate(one, 10.5)).toBeNull();
  });

  it('lists the joins, and knows an instant on one', () => {
    expect(sourceJoins(three)).toEqual([10, 15]);
    expect(sourceJoins(three.slice(0, 1))).toEqual([]);
    expect(isJoin(15, three)).toBe(true);
    expect(isJoin(15 + EPS / 2, three)).toBe(true);
    expect(isJoin(12, three)).toBe(false);
    expect(isJoin(0, three)).toBe(false);
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `npx vitest run client/src/editlist.test.ts`
Expected: FAIL. `locate`, `stitchedDuration`, `sourceJoins` and `isJoin` are not exported.

- [ ] **Step 3: Types and the stitched-time helpers**

In `client/src/types.ts`, replace the whole `BrollEdit` interface:

```ts
export interface BrollEdit extends Range {
  kind: 'broll';
  media: string;
  offset: number;
}
```

with:

```ts
/** A video on the main track, placed at a fixed offset on the stitched timeline. */
export interface Source {
  media: string;
  offset: number;
  duration: number;
}

/** Where a layer's picture sits: full frame, or a corner picture-in-picture. */
export type Frame = 'full' | 'pipTopLeft' | 'pipTopRight' | 'pipBottomLeft' | 'pipBottomRight';

/** The video tracks above the main one. B-roll is track 2. */
export type LayerTrack = 2 | 3;

/**
 * `media` from `offset` on track `track` over main-track `[start, end)`,
 * placed by `frame`. `audio` is null when muted, else the layer's level in dB.
 */
export interface LayerEdit extends Range {
  kind: 'layer';
  track: LayerTrack;
  media: string;
  offset: number;
  frame: Frame;
  audio: number | null;
}
```

Replace the `Edit` union line with:

```ts
export type Edit = CutEdit | OverdubEdit | TitleEdit | CaptionEdit | LayerEdit | AudioEdit;
```

After the `Media` interface add:

```ts
export type TranscriptStatus = 'pending' | 'running' | 'ready' | 'error';

/**
 * One file on the main track as GET /api/projects/:id lists it, in stitched
 * order; index 0 is the project's own media.
 */
export interface SourceView {
  index: number;
  mediaId: string;
  url: string;
  filename: string;
  kind: MediaKind;
  offset: number;
  duration: number;
  transcript: TranscriptStatus;
}
```

In `ProjectSummary`, after `media: Media;`, add:

```ts
  /** Every file on the main track. Sent by GET /api/projects/:id; absent in the list. */
  sources?: SourceView[];
```

In `client/src/editlist.ts`, add `Source` to the `./types` import (sorted: after `Range`), and append at the end of the file:

```ts
type Placed = Pick<Source, 'offset' | 'duration'>;

/** Where the stitched sources end: the last one's offset plus its duration. 0 with none. */
export function stitchedDuration(sources: readonly Placed[]): number {
  const last = sources[sources.length - 1];
  return last ? last.offset + last.duration : 0;
}

/**
 * The source holding stitched instant `t` and the time inside it. An instant
 * within EPS of a join belongs to the later source; the stitched end is the
 * last source at its full duration. Null before 0, past the end, or with no
 * sources. Mirrors the engine's `locate`.
 */
export function locate(
  sources: readonly Placed[],
  t: number,
): { index: number; local: number } | null {
  const last = sources.length - 1;
  const end = stitchedDuration(sources);
  if (last < 0 || t < -EPS || t > end + EPS) return null;
  if (t >= end - EPS) return { index: last, local: (sources[last] as Placed).duration };
  let index = 0;
  for (let i = last; i >= 0; i--) {
    if (t >= (sources[i] as Placed).offset - EPS) {
      index = i;
      break;
    }
  }
  const s = sources[index] as Placed;
  return { index, local: Math.min(Math.max(t - s.offset, 0), s.duration) };
}

/** The instants where one source ends and the next begins: every offset after the first. */
export function sourceJoins(sources: readonly Placed[]): number[] {
  return sources.slice(1).map((s) => s.offset);
}

/** True when `at` is a join. The fold never unsplits one. */
export function isJoin(at: number, sources: readonly Placed[]): boolean {
  return sourceJoins(sources).some((j) => Math.abs(j - at) < EPS);
}
```

- [ ] **Step 4: Run it and watch it pass**

Run: `npx vitest run client/src/editlist.test.ts`
Expected: PASS.

- [ ] **Step 5: Failing tests for the editor, ops, overlays and Delete**

In `client/src/editor.test.ts`, add `type DocState` from `./ops` to the imports:

```ts
import type { DocState } from './ops';
```

Replace the whole `it('addBroll replaces overlaps; audio edits by start; whole-edit range covers every word', …)` test with:

```ts
it('audio edits by start; whole-edit range covers every word', () => {
  let s = editorReducer(loaded, {
    type: 'addAudio',
    media: 'm',
    gain: -6,
    duck: true,
    range: null,
  });
  expect(s.edits[0]).toEqual({
    kind: 'audio',
    start: 0,
    end: 20,
    media: 'm',
    offset: 0,
    gain: -6,
    duck: true,
  });
  s = editorReducer(s, { type: 'editAudio', start: 0, gain: 3, duck: false });
  expect(s.edits[0]).toMatchObject({ gain: 3, duck: false });
  s = editorReducer(s, { type: 'removeAudio', start: 0 });
  expect(s.edits).toEqual([]);
});
```

Append to `client/src/editor.test.ts`:

```ts
describe('layers', () => {
  it('addLayer replaces the layers it overlaps on its own track only', () => {
    let s = editorReducer(select(loaded, 0, 1), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 1,
      frame: 'full',
      audio: null,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 3,
      media: 'c',
      offset: 0,
      frame: 'pipTopRight',
      audio: -6,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 0,
      frame: 'full',
      audio: null,
    });
    expect(s.edits).toEqual([
      {
        kind: 'layer',
        track: 3,
        start: 0.91,
        end: 2,
        media: 'c',
        offset: 0,
        frame: 'pipTopRight',
        audio: -6,
      },
      {
        kind: 'layer',
        track: 2,
        start: 0.91,
        end: 2,
        media: 'b',
        offset: 0,
        frame: 'full',
        audio: null,
      },
    ]);
  });

  it('setLayer edits in place and drops what it now overlaps on the new track; removeLayer by track and start', () => {
    let s = editorReducer(select(loaded, 1, 2), {
      type: 'addLayer',
      track: 3,
      media: 'c',
      offset: 0,
      frame: 'pipTopRight',
      audio: -6,
    });
    s = editorReducer(select(s, 1, 2), {
      type: 'addLayer',
      track: 2,
      media: 'b',
      offset: 0,
      frame: 'full',
      audio: null,
    });
    s = editorReducer(s, {
      type: 'setLayer',
      track: 3,
      start: 0.91,
      toTrack: 2,
      frame: 'pipBottomLeft',
      audio: 0,
    });
    expect(s.edits).toEqual([
      {
        kind: 'layer',
        track: 2,
        start: 0.91,
        end: 2,
        media: 'c',
        offset: 0,
        frame: 'pipBottomLeft',
        audio: 0,
      },
    ]);
    // Wrong track: nothing matches.
    expect(editorReducer(s, { type: 'removeLayer', track: 3, start: 0.91 }).edits).toHaveLength(1);
    expect(editorReducer(s, { type: 'removeLayer', track: 2, start: 0.91 }).edits).toEqual([]);
  });
});

describe('sources', () => {
  const first = editorReducer(initialEditor, { type: 'load', words, duration: 20, media: 'm0' });

  it("starts with the project's own media as the only source", () => {
    expect(first.sources).toEqual([{ media: 'm0', offset: 0, duration: 20 }]);
    expect(first.duration).toBe(20);
    expect(editorReducer(initialEditor, { type: 'load', words: [], duration: 0 }).sources).toEqual(
      [],
    );
  });

  it('addSource appends at the end with its join, and the duration is stitched', () => {
    const s = editorReducer(first, { type: 'addSource', media: 'm1', offset: 20, duration: 12.5 });
    expect(s.sources).toEqual([
      { media: 'm0', offset: 0, duration: 20 },
      { media: 'm1', offset: 20, duration: 12.5 },
    ]);
    expect(s.duration).toBe(32.5);
    expect(s.splits).toEqual([20]);
    // The server's broadcast of the same append can beat the upload's reply.
    expect(editorReducer(s, { type: 'addSource', media: 'm1', offset: 20, duration: 12.5 })).toBe(
      s,
    );
  });

  it("takes the fold's appended sources on sync, and drops them on its undo", () => {
    const doc: DocState = {
      headSeq: 2,
      edits: [],
      speakerNames: [],
      undoable: 2,
      redoable: null,
      splits: [3, 20],
      sources: [{ media: 'm1', offset: 20, duration: 10 }],
    };
    const s = editorReducer(first, { type: 'sync', doc });
    expect(s.sources.map((x) => x.media)).toEqual(['m0', 'm1']);
    expect(s.duration).toBe(30);
    expect(s.splits).toEqual([3, 20]);
    const undone = editorReducer(s, {
      type: 'sync',
      doc: { ...doc, headSeq: 3, sources: [], splits: [3] },
    });
    expect(undone.sources).toEqual([{ media: 'm0', offset: 0, duration: 20 }]);
    expect(undone.duration).toBe(20);
  });

  it("reads a peer's fold the same way, and an older server's fold as one source", () => {
    const r = editorReducer(first, {
      type: 'remote',
      headSeq: 5,
      edits: [],
      speakerNames: [],
      splits: [20],
      sources: [{ media: 'm1', offset: 20, duration: 4 }],
    });
    expect(r.duration).toBe(24);
    const old = editorReducer(first, { type: 'remote', headSeq: 6, edits: [], speakerNames: [] });
    expect(old.sources).toHaveLength(1);
    expect(old.duration).toBe(20);
  });

  it('never unsplits a join', () => {
    const s = editorReducer(first, { type: 'addSource', media: 'm1', offset: 20, duration: 5 });
    expect(editorReducer(s, { type: 'unsplit', at: 20 })).toBe(s);
  });

  it('setWords swaps in the stitched words and drops the selection', () => {
    const more = [...words, { id: '1:w0', text: 'later', start: 20.5, end: 21 }];
    const s = editorReducer(select(first, 1), { type: 'setWords', words: more });
    expect(s.words).toBe(more);
    expect(s.selection).toBeNull();
    expect(s.sources).toBe(first.sources);
  });
});
```

In `client/src/ops.test.ts`, replace:

```ts
expect(opForAction(selected01, { type: 'addBroll', media: 'b', offset: 1 })).toEqual({
  kind: 'addbroll',
  start: 0,
  end: 1.25,
  media: 'b',
  offset: 1,
});
```

with:

```ts
expect(
  opForAction(selected01, {
    type: 'addLayer',
    track: 2,
    media: 'b',
    offset: 1,
    frame: 'full',
    audio: null,
  }),
).toEqual({
  kind: 'addlayer',
  track: 2,
  start: 0,
  end: 1.25,
  media: 'b',
  offset: 1,
  frame: 'full',
  audio: null,
});
expect(
  opForAction(state, {
    type: 'setLayer',
    track: 2,
    start: 0,
    toTrack: 3,
    frame: 'pipTopLeft',
    audio: -3,
  }),
).toEqual({ kind: 'setlayer', track: 2, start: 0, toTrack: 3, frame: 'pipTopLeft', audio: -3 });
expect(opForAction(state, { type: 'removeLayer', track: 3, start: 0 })).toEqual({
  kind: 'removelayer',
  track: 3,
  start: 0,
});
```

and append inside the same `describe('clip and overlay operations', …)`:

```ts
it('sends no AddSource (the upload route appends it) and never unsplits a join', () => {
  const add = { type: 'addSource', media: 'm1', offset: 20, duration: 5 } as const;
  expect(opForAction(state, add)).toBeNull();
  const joined = editorReducer(state, add);
  expect(opForAction(joined, { type: 'unsplit', at: 20 })).toBeNull();
  expect(opForAction(joined, { type: 'unsplit', at: 2 })).toEqual({ kind: 'unsplit', at: 2 });
});
```

Replace `client/src/overlays.test.ts` with:

```ts
import { describe, expect, it } from 'vitest';

import { assetTime, audiosAt, gainToLinear, layerAt, layers, speaking } from './overlays';
import type { Edit, Word } from './types';

const edits: Edit[] = [
  {
    kind: 'layer',
    track: 2,
    start: 2,
    end: 4,
    media: 'b',
    offset: 1.5,
    frame: 'full',
    audio: null,
  },
  {
    kind: 'layer',
    track: 3,
    start: 6,
    end: 8,
    media: 'c',
    offset: 0,
    frame: 'pipTopLeft',
    audio: -6,
  },
  { kind: 'audio', start: 0, end: 10, media: 'm', offset: 3, gain: -6, duck: true },
];
const words: Word[] = [{ id: 'w', text: 'x', start: 1, end: 1.5 }];

describe('overlays', () => {
  it('finds the overlays under a source time and maps into the asset', () => {
    expect(layerAt(3, edits, 2)?.media).toBe('b');
    expect(layerAt(4, edits, 2)).toBeUndefined();
    expect(layerAt(3, edits, 3)).toBeUndefined();
    expect(layerAt(7, edits, 3)?.media).toBe('c');
    expect(audiosAt(9, edits)).toHaveLength(1);
    expect(assetTime(edits[0] as { start: number; offset: number }, 3)).toBe(2.5);
    expect(gainToLinear(-6)).toBeCloseTo(0.501, 3);
    expect(gainToLinear(0)).toBe(1);
    expect(speaking(1.2, words)).toBe(true);
    expect(speaking(1.7, words)).toBe(false);
  });

  it('lists layers on one track or on all of them', () => {
    expect(layers(edits).map((l) => l.media)).toEqual(['b', 'c']);
    expect(layers(edits, 2).map((l) => l.media)).toEqual(['b']);
    expect(layers(edits, 3).map((l) => l.media)).toEqual(['c']);
  });
});
```

In `client/src/selection.test.ts`, replace:

```ts
expect(deleteAction({ ...none, overlay: { kind: 'broll', start: 2 } })).toEqual({
  type: 'removeBroll',
  start: 2,
});
```

with:

```ts
expect(deleteAction({ ...none, overlay: { kind: 'broll', start: 2 } })).toEqual({
  type: 'removeLayer',
  track: 2,
  start: 2,
});
```

- [ ] **Step 6: Run them and watch them fail**

Run: `npx vitest run client/src/editor.test.ts client/src/ops.test.ts client/src/overlays.test.ts client/src/selection.test.ts`
Expected: FAIL. The unknown actions leave `state` unchanged, `sources` is undefined, `layers`/`layerAt` are not exported, and Delete still returns `removeBroll`.

- [ ] **Step 7: Editor, ops, overlays, Delete and the socket types**

`client/src/editor.ts`:

1. Replace the imports at the top with:

   ```ts
   import {
     EPS,
     isJoin,
     pieceStarts,
     rangeForWords,
     stitchedDuration,
     wordStatus,
   } from './editlist';
   import type { DocState } from './ops';
   import type {
     AudioEdit,
     CaptionEdit,
     CaptionPos,
     CutEdit,
     Edit,
     Frame,
     LayerEdit,
     LayerTrack,
     OverdubEdit,
     Source,
     TitleEdit,
     TitleStyle,
     Transition,
     Word,
   } from './types';
   ```

2. In `EditorState`, replace `duration: number;` with:

   ```ts
     /** Stitched length of every source on the main track, in seconds. */
     duration: number;
     /**
      * The main track's files in stitched order: the project's own media, then
      * the fold's appended sources. Empty before a project loads.
      */
     sources: Source[];
   ```

3. In `EditorAction`, replace the `load` member with:

   ```ts
     /** `media` is the project's own media id: source 0. */
     | { type: 'load'; words: Word[]; duration: number; media?: string }
     /** The stitched words again, once another source's transcript is ready. */
     | { type: 'setWords'; words: Word[] }
   ```

   Replace the two B-roll members:

   ```ts
     /** `range` `null` means the whole edit; `undefined` means the current selection. */
     | { type: 'addBroll'; media: string; offset: number; range?: [number, number] }
     | { type: 'removeBroll'; start: number }
   ```

   with:

   ```ts
     /** The optimistic echo of the AddSource the upload route appended. Never sent. */
     | { type: 'addSource'; media: string; offset: number; duration: number }
     /** `range` is the word range captured when the dialog opened; else the selection. */
     | {
         type: 'addLayer';
         track: LayerTrack;
         media: string;
         offset: number;
         frame: Frame;
         audio: number | null;
         range?: [number, number];
       }
     | {
         type: 'setLayer';
         track: LayerTrack;
         start: number;
         toTrack: LayerTrack;
         frame: Frame;
         audio: number | null;
       }
     | { type: 'removeLayer'; track: LayerTrack; start: number }
   ```

   In the `remote` member, after `order?: number[];` add `sources?: Source[];`. Then move the `/** \`range\` \`null\` means the whole edit; \`undefined\` means the current selection. */`comment down so that it sits above the`addAudio` member, which is where that meaning applies.

4. In `initialEditor`, after `duration: 0,` add `sources: [],`.

5. After `wholeRange`, add:

   ```ts
   /** The first source (the project's media) followed by a fold's appended sources. */
   function withAppended(state: EditorState, appended: Source[] | undefined): Source[] {
     const first = state.sources[0];
     return first ? [first, ...(appended ?? [])] : [...(appended ?? [])];
   }

   /** The stitched length of `sources`, or `fallback` when there are none yet. */
   function lengthOf(sources: Source[], fallback: number): number {
     return sources.length > 0 ? stitchedDuration(sources) : fallback;
   }
   ```

6. Replace the `load` case with:

   ```ts
       case 'load':
         return {
           ...initialEditor,
           words: action.words,
           duration: action.duration,
           sources:
             action.duration > 0
               ? [{ media: action.media ?? '', offset: 0, duration: action.duration }]
               : [],
         };

       case 'setWords':
         // Indices shift when another source's words arrive, so the selection goes.
         return { ...state, words: action.words, selection: null };
   ```

7. In the `sync` case, replace `const stale = action.doc.headSeq < state.headSeq;` and the returned object with:

   ```ts
   const stale = action.doc.headSeq < state.headSeq;
   const sources = stale ? state.sources : withAppended(state, action.doc.sources);
   return {
     ...state,
     edits: stale ? state.edits : action.doc.edits,
     speakerNames: stale ? state.speakerNames : action.doc.speakerNames,
     headSeq: stale ? state.headSeq : action.doc.headSeq,
     transition: stale ? state.transition : (action.doc.transition ?? 'none'),
     undoable: action.doc.undoable,
     redoable: action.doc.redoable,
     selection: stale ? state.selection : null,
     splits: stale ? state.splits : (action.doc.splits ?? []),
     order: stale ? state.order : (action.doc.order ?? []),
     sources,
     duration: lengthOf(sources, state.duration),
   };
   ```

8. Replace the `remote` case's returned object with:

   ```ts
   {
     const sources = withAppended(state, action.sources);
     return {
       ...state,
       edits: action.edits,
       speakerNames: action.speakerNames,
       transition: action.transition ?? 'none',
       headSeq: action.headSeq,
       splits: action.splits ?? [],
       order: action.order ?? [],
       sources,
       duration: lengthOf(sources, state.duration),
     };
   }
   ```

   (The `if (action.headSeq <= state.headSeq) return state;` guard above it stays.)

9. Replace the `unsplit` case with:

   ```ts
       case 'unsplit':
         // A join between two files is permanent, as in the engine's fold.
         if (isJoin(action.at, state.sources)) return state;
         return { ...state, splits: state.splits.filter((s) => !sameInstant(s, action.at)) };
   ```

10. Replace the `addBroll` and `removeBroll` cases with:

    ```ts
        case 'addSource': {
          if (state.sources.some((s) => sameInstant(s.offset, action.offset))) return state;
          const sources = [
            ...state.sources,
            { media: action.media, offset: action.offset, duration: action.duration },
          ];
          // Each file starts as its own clip, as the fold's permanent split does.
          const splits = state.splits.some((s) => sameInstant(s, action.offset))
            ? state.splits
            : [...state.splits, action.offset].sort((a, b) => a - b);
          return { ...state, sources, splits, duration: stitchedDuration(sources) };
        }

        case 'addLayer': {
          const range = action.range ?? selectedRange(state.selection);
          if (!range) return state;
          const span = rangeForWords(state.words, range[0], range[1], state.duration);
          // A new layer replaces those it overlaps on its own track, as B-roll did.
          const kept = state.edits.filter(
            (e) =>
              !(
                e.kind === 'layer' &&
                e.track === action.track &&
                e.start < span.end &&
                e.end > span.start
              ),
          );
          const layer: LayerEdit = {
            kind: 'layer',
            track: action.track,
            ...span,
            media: action.media,
            offset: action.offset,
            frame: action.frame,
            audio: action.audio,
          };
          return withEdits(state, [...kept, layer]);
        }

        case 'setLayer': {
          const at = state.edits.findIndex(
            (e) =>
              e.kind === 'layer' && e.track === action.track && sameInstant(e.start, action.start),
          );
          const found = state.edits[at];
          if (at === -1 || found?.kind !== 'layer') return state;
          const moved: LayerEdit = {
            ...found,
            track: action.toTrack,
            frame: action.frame,
            audio: action.audio,
          };
          // In place, as the fold does; a layer it now overlaps on the new track goes.
          const edits = state.edits.flatMap((e, i): Edit[] => {
            if (i === at) return [moved];
            const covered =
              e.kind === 'layer' &&
              e.track === action.toTrack &&
              e.start < moved.end &&
              e.end > moved.start;
            return covered ? [] : [e];
          });
          return { ...state, edits };
        }

        case 'removeLayer':
          return {
            ...state,
            edits: state.edits.filter(
              (e) =>
                !(
                  e.kind === 'layer' &&
                  e.track === action.track &&
                  sameInstant(e.start, action.start)
                ),
            ),
          };
    ```

`client/src/ops.ts`:

1. Replace the imports with:

   ```ts
   import { isJoin, rangeForWords } from './editlist';
   import type { EditorAction, EditorState } from './editor';
   import { selectedRange, wholeRange } from './editor';
   import type {
     CaptionPos,
     Edit,
     Frame,
     LayerTrack,
     Range,
     Source,
     TitleStyle,
     Transition,
   } from './types';
   ```

2. In `Op`, replace:

   ```ts
     | { kind: 'addbroll'; start: number; end: number; media: string; offset: number }
     | { kind: 'removebroll'; start: number }
   ```

   with:

   ```ts
     /** Appended by POST /api/projects/:id/sources; listed for completeness, never sent by the client. */
     | { kind: 'addsource'; media: string; offset: number; duration: number }
     | {
         kind: 'addlayer';
         track: LayerTrack;
         start: number;
         end: number;
         media: string;
         offset: number;
         frame: Frame;
         audio: number | null;
       }
     | {
         kind: 'setlayer';
         track: LayerTrack;
         start: number;
         toTrack: LayerTrack;
         frame: Frame;
         audio: number | null;
       }
     | { kind: 'removelayer'; track: LayerTrack; start: number }
   ```

3. In `DocState`, after `order?: number[];` add:

   ```ts
     /** Sources appended after the project's own media. Absent on folds from an older server. */
     sources?: Source[];
   ```

4. In `opForAction`, replace `case 'unsplit': return { kind: 'unsplit', at: action.at };` with:

   ```ts
       case 'unsplit':
         // The fold ignores an unsplit at a join, so there is nothing to send.
         return isJoin(action.at, state.sources) ? null : { kind: 'unsplit', at: action.at };
   ```

   and replace the `addBroll` and `removeBroll` cases with:

   ```ts
       // `addSource` is deliberately absent: the upload route appends that op
       // itself, and the reducer's copy is only its optimistic echo.
       case 'addLayer': {
         const range = action.range ?? selectedRange(state.selection);
         if (!range) return null;
         return {
           kind: 'addlayer',
           track: action.track,
           ...rangeForWords(state.words, range[0], range[1], state.duration),
           media: action.media,
           offset: action.offset,
           frame: action.frame,
           audio: action.audio,
         };
       }
       case 'setLayer':
         return {
           kind: 'setlayer',
           track: action.track,
           start: action.start,
           toTrack: action.toTrack,
           frame: action.frame,
           audio: action.audio,
         };
       case 'removeLayer':
         return { kind: 'removelayer', track: action.track, start: action.start };
   ```

`client/src/overlays.ts`: replace the header comment, the import and the two B-roll functions (`brolls`, `brollAt`) with:

```ts
// Overlay helpers for video layers (B-roll is track 2), background audio,
// gain, and the preview's speech envelope. Mirrors the engine's overlay
// handling for the client.

import type { AudioEdit, Edit, LayerEdit, LayerTrack, Word } from './types';
```

and, after `DUCK`:

```ts
/** Layer edits on one track, or on every track when `track` is omitted. */
export function layers(edits: Edit[], track?: LayerTrack): LayerEdit[] {
  return edits.filter(
    (e): e is LayerEdit => e.kind === 'layer' && (track === undefined || e.track === track),
  );
}
```

and, in place of `brollAt`:

```ts
/** The layer on `track` covering source time `t`, if any. */
export function layerAt(t: number, edits: Edit[], track: LayerTrack): LayerEdit | undefined {
  return layers(edits, track).find((l) => t >= l.start && t < l.end);
}
```

`client/src/selection.ts`: replace `? { type: 'removeBroll', start: overlay.start }` with `? { type: 'removeLayer', track: 2, start: overlay.start }`.

`client/src/realtime.ts`: change the import to `import type { Edit, Source } from './types';` and add `sources?: Source[];` after `order?: number[];` in each of the `hello`, `doc` and `resync` members of `ServerMsg`.

`client/src/useRealtime.ts`: change the types import to `import type { Edit, Source, Transition } from './types';` and add to `RemoteDoc`, after `order`:

```ts
  /** Sources appended after the project's own media. Absent from an older server. */
  sources?: Source[];
```

- [ ] **Step 8: Run them and watch them pass**

Run: `npx vitest run client/src/editor.test.ts client/src/ops.test.ts client/src/overlays.test.ts client/src/selection.test.ts`
Expected: PASS.

- [ ] **Step 9: Failing tests for the upload with progress and the source views**

Append to `client/src/api.test.ts`, and change its import to `import { addSource, ApiError, login, request, setUnauthorizedHandler } from './api';` and add `import type { SourceView } from './types';`:

```ts
/** Just enough XMLHttpRequest for `upload`: open, send, upload progress and a reply. */
class FakeXhr {
  static last: FakeXhr | null = null;
  method = '';
  url = '';
  body: unknown = null;
  status = 0;
  statusText = '';
  responseText = '';
  upload: {
    onprogress: ((e: { lengthComputable: boolean; loaded: number; total: number }) => void) | null;
  } = { onprogress: null };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  open(method: string, url: string) {
    this.method = method;
    this.url = url;
  }
  send(body: unknown) {
    this.body = body;
    FakeXhr.last = this;
  }
  respond(status: number, body: unknown) {
    this.status = status;
    this.statusText = status < 300 ? 'OK' : 'Bad Request';
    this.responseText = JSON.stringify(body);
    this.onload?.();
  }
}

const view: SourceView = {
  index: 1,
  mediaId: 'm1',
  url: '/data/m1/source.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 60,
  duration: 30.5,
  transcript: 'pending',
};

describe('addSource', () => {
  const file = new File(['abc'], 'take2.mp4', { type: 'video/mp4' });

  beforeEach(() => {
    FakeXhr.last = null;
    vi.stubGlobal('XMLHttpRequest', FakeXhr);
  });

  it('posts the file to /sources and reports upload progress', async () => {
    const onProgress = vi.fn();
    const done = addSource('p1', file, onProgress);
    const xhr = FakeXhr.last as unknown as FakeXhr;
    expect(xhr.method).toBe('POST');
    expect(xhr.url).toBe('/api/projects/p1/sources');
    expect((xhr.body as FormData).get('file')).toBeInstanceOf(File);
    xhr.upload.onprogress?.({ lengthComputable: true, loaded: 25, total: 100 });
    xhr.upload.onprogress?.({ lengthComputable: false, loaded: 50, total: 0 });
    xhr.respond(200, view);
    await expect(done).resolves.toEqual(view);
    expect(onProgress.mock.calls.map(([f]) => f)).toEqual([0.25, 1]);
  });

  it("rejects with the server's message", async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).respond(400, {
      error: 'a project holds at most 20 videos',
    });
    await expect(done).rejects.toMatchObject({
      message: 'a project holds at most 20 videos',
      status: 400,
    });
  });

  it('reports the session as gone on a 401', async () => {
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).respond(401, { error: 'sign in first' });
    await expect(done).rejects.toBeInstanceOf(ApiError);
    expect(onUnauthorized).toHaveBeenCalledOnce();
  });

  it('says the server is unreachable on a network error', async () => {
    const done = addSource('p1', file);
    (FakeXhr.last as unknown as FakeXhr).onerror?.();
    await expect(done).rejects.toMatchObject({ status: 0 });
  });
});
```

and change the vitest import at the top of `api.test.ts` to `import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';`.

Create `client/src/sources.test.ts`:

```ts
import { describe, expect, it } from 'vitest';

import { isTranscribing, playableSources, sourceViewsOf, unlistedSources } from './sources';
import type { Media, ProjectSummary, Source, SourceView } from './types';

const media: Media = {
  id: 'm0',
  filename: 'take1.mp4',
  ext: 'mp4',
  duration: 10,
  kind: 'video',
  url: '/data/m0/source.mp4',
};
const summary: ProjectSummary = { id: 'p', title: 'take1', role: 'owner', media, createdAt: 0 };

const view = (
  index: number,
  mediaId: string,
  offset: number,
  duration: number,
  transcript: SourceView['transcript'] = 'ready',
): SourceView => ({
  index,
  mediaId,
  url: `/data/${mediaId}/source.mp4`,
  filename: `${mediaId}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript,
});

describe('sourceViewsOf', () => {
  it('builds the one source from `media` for a server that does not list sources', () => {
    expect(sourceViewsOf(summary)).toEqual([
      {
        index: 0,
        mediaId: 'm0',
        url: '/data/m0/source.mp4',
        filename: 'take1.mp4',
        kind: 'video',
        offset: 0,
        duration: 10,
        transcript: 'ready',
      },
    ]);
  });

  it("uses the server's list when there is one", () => {
    const sources = [view(0, 'm0', 0, 10), view(1, 'm1', 10, 5, 'running')];
    expect(sourceViewsOf({ ...summary, sources })).toBe(sources);
  });
});

describe('playableSources', () => {
  const views = [view(0, 'm0', 0, 10), view(1, 'm1', 10, 5, 'running'), view(2, 'm2', 15, 4)];
  const fold: Source[] = [
    { media: 'm0', offset: 0, duration: 10 },
    { media: 'm1', offset: 10, duration: 5 },
  ];

  it("lays the fold's sources over the listed files; an undone source is not played", () => {
    expect(playableSources(fold, views).map((v) => [v.index, v.mediaId, v.transcript])).toEqual([
      [0, 'm0', 'ready'],
      [1, 'm1', 'running'],
    ]);
  });

  it('leaves out, and reports, a source the list does not know yet', () => {
    const more = [...fold, { media: 'm9', offset: 15, duration: 1 }];
    expect(playableSources(more, views)).toHaveLength(2);
    expect(unlistedSources(more, views)).toEqual(['m9']);
    expect(unlistedSources(fold, views)).toEqual([]);
  });

  it('knows which transcripts are still coming', () => {
    expect(isTranscribing(view(1, 'm1', 0, 1, 'pending'))).toBe(true);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'running'))).toBe(true);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'ready'))).toBe(false);
    expect(isTranscribing(view(1, 'm1', 0, 1, 'error'))).toBe(false);
  });
});
```

- [ ] **Step 10: Run them and watch them fail**

Run: `npx vitest run client/src/api.test.ts client/src/sources.test.ts`
Expected: FAIL. `addSource` is not exported, and `./sources` does not exist.

- [ ] **Step 11: `upload`, `addSource` and the source views**

In `client/src/api.ts`:

1. Change the types import to:

   ```ts
   import type {
     Asset,
     CutEdit,
     LibraryItem,
     ProjectSummary,
     SourceView,
     TokenInfo,
     User,
     Word,
   } from './types';
   ```

2. Above `request`, add:

   ```ts
   const UNREACHABLE = 'Could not reach the server. Is `cargo run -p server` running?';

   /** The server's `{ error }` text, else the status line. */
   function errorMessage(body: unknown, status: number, statusText: string): string {
     return body && typeof body === 'object' && 'error' in body && typeof body.error === 'string'
       ? body.error
       : `${status} ${statusText}`;
   }
   ```

   and change `request` to use them:

   ```ts
   export async function request<T>(url: string, init?: RequestInit): Promise<T> {
     let response: Response;
     try {
       response = await fetch(url, init);
     } catch {
       throw new ApiError(UNREACHABLE, 0);
     }
     const body: unknown = await response.json().catch(() => null);
     if (!response.ok) {
       // A failed sign-in is also a 401; it must not wipe an existing session.
       if (response.status === 401 && !url.startsWith('/api/auth/')) onUnauthorized?.();
       throw new ApiError(
         errorMessage(body, response.status, response.statusText),
         response.status,
       );
     }
     return body as T;
   }
   ```

3. Replace `uploadMedia` with:

   ```ts
   /**
    * A multipart POST that reports how much of the body has gone up (fetch
    * cannot), with `request`'s errors. Progress ends at 1 once the server answers.
    */
   export function upload<T>(
     url: string,
     form: FormData,
     onProgress?: (fraction: number) => void,
   ): Promise<T> {
     return new Promise<T>((resolve, reject) => {
       const xhr = new XMLHttpRequest();
       xhr.open('POST', url);
       if (onProgress) {
         xhr.upload.onprogress = (e) => {
           if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total);
         };
       }
       xhr.onerror = () => reject(new ApiError(UNREACHABLE, 0));
       xhr.onload = () => {
         let body: unknown = null;
         try {
           body = JSON.parse(xhr.responseText);
         } catch {
           body = null;
         }
         if (xhr.status < 200 || xhr.status >= 300) {
           if (xhr.status === 401) onUnauthorized?.();
           reject(new ApiError(errorMessage(body, xhr.status, xhr.statusText), xhr.status));
           return;
         }
         onProgress?.(1);
         resolve(body as T);
       };
       xhr.send(form);
     });
   }

   function fileForm(file: File): FormData {
     const form = new FormData();
     form.append('file', file, file.name);
     return form;
   }

   export function uploadMedia(
     file: File,
     onProgress?: (fraction: number) => void,
   ): Promise<ProjectSummary> {
     return upload<ProjectSummary>('/api/projects', fileForm(file), onProgress);
   }

   /** Append a file to the end of the project's main track (editors and owners). */
   export function addSource(
     projectId: string,
     file: File,
     onProgress?: (fraction: number) => void,
   ): Promise<SourceView> {
     return upload<SourceView>(`/api/projects/${projectId}/sources`, fileForm(file), onProgress);
   }
   ```

4. Replace `transcribeMedia` with:

   ```ts
   /**
    * The stitched words of every source whose transcript is ready, and each
    * source's status. Waits for source 0 only; the server transcribes the
    * others in the background and starts any that is `pending`. `sources` is
    * absent from a server that predates sources.
    */
   export function transcribeProject(
     id: string,
   ): Promise<{ words: Word[]; sources?: SourceView[] }> {
     return request(`/api/projects/${id}/transcribe`, { method: 'POST' });
   }

   export async function transcribeMedia(id: string): Promise<Word[]> {
     return (await transcribeProject(id)).words;
   }
   ```

Create `client/src/sources.ts`:

```ts
// The project's files as the server lists them, joined onto the fold's
// sources. The fold decides what is on the timeline (an undo can drop a
// source); the list, which can be a poll behind, supplies names, urls and
// transcript status.

import type { ProjectSummary, Source, SourceView } from './types';

/** The listed sources, or one built from `media` for a server that predates sources. */
export function sourceViewsOf(project: ProjectSummary): SourceView[] {
  if (project.sources && project.sources.length > 0) return project.sources;
  const m = project.media;
  return [
    {
      index: 0,
      mediaId: m.id,
      url: m.url,
      filename: m.filename,
      kind: m.kind,
      offset: 0,
      duration: m.duration,
      transcript: 'ready',
    },
  ];
}

/**
 * The fold's sources with their file details, in stitched order. A source the
 * list does not know yet (a peer's, just added) is left out until refetched.
 */
export function playableSources(sources: Source[], views: SourceView[]): SourceView[] {
  const out: SourceView[] = [];
  sources.forEach((s, index) => {
    const listed = views.find((v) => v.mediaId === s.media);
    if (listed) out.push({ ...listed, index, offset: s.offset, duration: s.duration });
  });
  return out;
}

/** Media ids on the timeline that the list does not name yet. */
export function unlistedSources(sources: Source[], views: SourceView[]): string[] {
  return sources.map((s) => s.media).filter((id) => !views.some((v) => v.mediaId === id));
}

export function isTranscribing(view: SourceView): boolean {
  return view.transcript === 'pending' || view.transcript === 'running';
}
```

- [ ] **Step 12: Run them and watch them pass**

Run: `npx vitest run client/src/api.test.ts client/src/sources.test.ts`
Expected: PASS.

- [ ] **Step 13: Route every B-roll user through track-2 layers**

`client/src/components/Overlays.tsx`: change the import to `import { assetTime, audiosAt, DUCK, gainToLinear, layerAt, speaking } from '../overlays';` and `const broll = brollAt(t, edits);` to:

```ts
// Track 2, drawn full frame and muted as B-roll always was; Task 9 stacks every layer.
const broll = layerAt(t, edits, 2);
```

`client/src/components/Timeline.tsx`: change `import { audios, brolls } from '../overlays';` to `import { audios, layers } from '../overlays';`, change the types import to `import type { Asset, AudioEdit, Edit, LayerEdit, Range, Word } from '../types';`, change the `bars` signature to `const bars = (kind: OverlayRef['kind'], list: (LayerEdit | AudioEdit)[]) =>`, and change `{bars('broll', brolls(edits))}` to `{bars('broll', layers(edits, 2))}`.

`client/src/components/Transcript.tsx`: change `import { audios, brolls } from '../overlays';` to `import { audios, layers } from '../overlays';` and `const b = startingAt(brolls(edits), i);` to `const b = startingAt(layers(edits, 2), i);`.

`client/src/components/Timeline.test.tsx`: replace the fixture line `{ kind: 'broll', start: 2, end: 4, media: 'a1', offset: 0 },` with:

```ts
  { kind: 'layer', track: 2, start: 2, end: 4, media: 'a1', offset: 0, frame: 'full', audio: null },
```

`client/src/App.tsx`:

1. `import { audios, brolls } from './overlays';` → `import { audios, layers } from './overlays';`
2. In the unknown-asset effect: `const unknown = [...brolls(editor.edits), ...audios(editor.edits)]` → `const unknown = [...layers(editor.edits), ...audios(editor.edits)]`
3. In the stale-overlay effect: `selectedOverlay.kind === 'broll' ? brolls(editor.edits) : audios(editor.edits)` → `selectedOverlay.kind === 'broll' ? layers(editor.edits, 2) : audios(editor.edits)`
4. In `load`: `dispatch({ type: 'load', words, duration: summary.media.duration });` → `dispatch({ type: 'load', words, duration: summary.media.duration, media: summary.media.id });`
5. On `<Transcript>`: `onBrollClick={(start) => edit({ type: 'removeBroll', start })}` → `onBrollClick={(start) => edit({ type: 'removeLayer', track: 2, start })}`
6. In `<BrollDialog>`'s `onSubmit`: `edit({ type: 'addBroll', media: asset.id, offset, range: brollRange });` →

   ```ts
   edit({
     type: 'addLayer',
     track: 2,
     media: asset.id,
     offset,
     frame: 'full',
     audio: null,
     range: brollRange,
   });
   ```

- [ ] **Step 14: Verify the whole client**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npx prettier --write client/src
npx vitest run && npx tsc -p client --noEmit && npm run lint
```

Expected: all pass. `grep -rn "broll'" client/src --include='*.ts'` finds only `OverlayRef`'s `'broll' | 'audio'` users (Timeline, selection, App, tests), which Task 8 renames.

- [ ] **Step 15: Commit**

```bash
git add client/src
git commit -m "client: sources on the stitched timeline, layer edits in place of B-roll, uploads with progress" -m "The editor keeps the fold's sources after the project's own media and measures the edit by their stitched length; locate and stitchedDuration mirror the engine. B-roll is now a track-2 layer everywhere, addSource/addLayer/setLayer/removeLayer join the editor and op mirrors, and addSource uploads through XHR so it can report progress.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 3: Server: migration, the /sources route, sources in the project response, op validation

The server learns that a project has several sources. After this task an editor can upload a
second file into a project. It lands at the end of the main track as an `AddSource`.
`GET /api/projects/{id}` lists every source. Every op is validated against the stitched
duration. Transcription is still first-source only; Task 4 extends it.

**Files:**

- Create: `server/migrations/0005_sources.sql`, `server/src/sources.rs`
- Modify:
  - `server/src/main.rs`, `server/src/app.rs`, `server/src/db.rs`
  - `server/src/projects.rs`, `server/src/ops.rs`
  - `server/src/bus.rs`, `server/src/ws.rs`
  - `server/src/assets.rs`, `server/src/routes.rs`
  - `server/src/test_util.rs`

**Interfaces:**

- Consumes (Task 1, `engine`):

  ```rust
  pub struct Source { pub media: String, pub offset: f64, pub duration: f64 }
  pub enum Frame { Full, PipTopLeft, PipTopRight, PipBottomLeft, PipBottomRight }
  pub fn all_sources(first: Source, doc_sources: &[Source]) -> Vec<Source>;
  pub fn stitched_duration(sources: &[Source]) -> f64;
  Edit::Layer { track: u8, start: f64, end: f64, media: String, offset: f64, frame: Frame, audio: Option<f64> }
  Op::AddSource { media: String, offset: f64, duration: f64 }
  Op::AddLayer { track: u8, start: f64, end: f64, media: String, offset: f64, frame: Frame, audio: Option<f64> }
  Op::SetLayer { track: u8, start: f64, to_track: u8, frame: Frame, audio: Option<f64> }
  Op::RemoveLayer { track: u8, start: f64 }
  ProjectDoc { pub sources: Vec<Source>, .. }  // AddSource fold: push + split at offset
  ```

- Produces (`server/src/sources.rs`):

  ```rust
  pub const MAX_SOURCES: usize = 20;
  #[serde(rename_all = "lowercase")] pub enum TranscriptStatus { Pending, Running, Ready, Error }
  #[serde(rename_all = "camelCase")] pub struct SourceView {
      pub index: usize, pub media_id: String, pub url: String, pub filename: String,
      pub kind: MediaKind, pub offset: f64, pub duration: f64, pub transcript: TranscriptStatus }
  pub async fn register(db: &SqlitePool, project_id: &str, media_id: &str, start_at: f64, duration: f64) -> AppResult<()>;
  pub async fn in_registry<'e, E: Executor<'e, Database = Sqlite>>(exec: E, project_id: &str, media_id: &str) -> AppResult<bool>;
  pub async fn registry(db: &SqlitePool, project_id: &str) -> AppResult<Vec<String>>;
  pub async fn first_source(state: &AppState, project: &Project) -> AppResult<Source>;
  pub async fn timeline(state: &AppState, project: &Project, doc: &ProjectDoc) -> AppResult<Vec<Source>>;
  pub fn transcript_status(state: &AppState, media_id: &str) -> TranscriptStatus;
  pub async fn views(state: &AppState, sources: &[Source]) -> AppResult<Vec<SourceView>>;
  pub async fn add_source(state: &Arc<AppState>, project: &Project, user: &User, meta: &Meta) -> AppResult<SourceView>;
  pub enum LayerMedia { Asset { kind: MediaKind, duration: f64 }, Source }
  pub async fn resolve_layer_media(conn: &mut SqliteConnection, project_id: &str, media: &str) -> AppResult<Option<LayerMedia>>;
  pub async fn layer_media(tx: &mut Transaction<'_, Sqlite>, state: &AppState, project: &Project, media: &str) -> AppResult<Option<(MediaKind, f64)>>;
  pub async fn source_file(state: &AppState, media: &str) -> AppResult<(PathBuf, Meta)>;
  pub async fn upload(State<Arc<AppState>>, ProjectAccess, Multipart) -> AppResult<Json<SourceView>>; // POST /api/projects/{id}/sources
  ```

- Produces (`server/src/ops.rs`): `DocState.sources: Vec<Source>`: the appended sources only, exactly
  the fold's `doc.sources` (the project's own media is not in it). `project.sources` on
  `GET /api/projects/{id}` is the full `SourceView[]`, first included.
- Produces (`server/src/bus.rs`): `ServerMsg::Hello.sources` and `ServerMsg::Doc.sources`,
  `Vec<Source>`, `#[serde(default)]`.
- Produces (`server/src/routes.rs`): `pub(crate) async fn store_upload(..) -> AppResult<Meta>`,
  which removes its directory on failure. `/thumbnails` also takes an optional
  `{ "media": String }` body.
- Produces (`server/src/test_util.rs`):
  `pub async fn seed_words(state: &Arc<AppState>, media_id: &str, texts: &[&str])`; and
  (`server/src/sources.rs`, `#[cfg(test)]`) `test_support::seed_source(state, duration, words) -> Meta`.

#### Round A: the table, its backfill and the registry

- [ ] **Step 1: Write the failing tests**

In `server/src/db.rs`, add `"project_sources"` to the `expected` list in `open_creates_schema`:

```rust
        for expected in [
            "users",
            "sessions",
            "projects",
            "project_members",
            "edit_ops",
            "project_sources",
        ] {
```

and add this test inside `mod tests`:

```rust
    /// A database from before sources: every project gets its own media as
    /// source 0 when 0005 runs.
    #[tokio::test]
    async fn sources_backfill_gives_every_project_its_first_source() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        let options = SqliteConnectOptions::from_str(&url)
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();
        let migrator = sqlx::migrate!("./migrations");
        migrator.run_to(4, &pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, display_name, color, created_at)
             VALUES ('u', 'u@example.com', 'h', 'U', '#000000', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, media) in [("p1", "m1"), ("p2", "m2")] {
            sqlx::query(
                "INSERT INTO projects (id, media_id, owner_id, title, created_at)
                 VALUES (?, ?, 'u', 'T', 0)",
            )
            .bind(id)
            .bind(media)
            .execute(&pool)
            .await
            .unwrap();
        }
        migrator.run(&pool).await.unwrap();
        let rows: Vec<(String, i64, String, f64, f64)> = sqlx::query_as(
            "SELECT project_id, position, media_id, start_at, duration
             FROM project_sources ORDER BY project_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("p1".into(), 0, "m1".into(), 0.0, 0.0),
                ("p2".into(), 0, "m2".into(), 0.0, 0.0),
            ]
        );
    }
```

Create `server/src/sources.rs` holding only the tests for now. Round A's Step 3 adds the code
above them.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::test_support::seed_media;
    use crate::test_util::{me, owned_project, register as sign_up, state};

    #[tokio::test]
    async fn a_new_project_registers_its_own_media_at_position_zero() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone()]
        );
        register(&state.db, &project.id, "m2", 10.0, 4.0).await.unwrap();
        register(&state.db, &project.id, "m3", 14.0, 1.0).await.unwrap();
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone(), "m2".to_owned(), "m3".to_owned()]
        );
        assert!(in_registry(&state.db, &project.id, "m2").await.unwrap());
        assert!(!in_registry(&state.db, &project.id, "elsewhere").await.unwrap());
    }

    #[tokio::test]
    async fn adopting_orphans_skips_media_that_is_already_a_source() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let second = seed_media(&state, 4.0).await;
        register(&state.db, &project.id, &second, 10.0, 4.0)
            .await
            .unwrap();
        let owner = me(&state, &ada).await;
        crate::projects::adopt_orphans(&state, &owner.id)
            .await
            .unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(n, 1, "a source's media must not become a project of its own");
    }
}
```

Add `mod sources;` to `server/src/main.rs`, between `mod routes;` and `#[cfg(test)] mod test_util;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server sources`

Expected: FAIL to compile, with `cannot find function `registry` in this scope` (and `register`,
`in_registry`). `cargo test -p server db::` would fail on `missing table project_sources` and on
`run_to` finding no version 5.

- [ ] **Step 3: Write the migration, the registry and the two call sites**

`server/migrations/0005_sources.sql`:

```sql
-- Every media item uploaded into a project, in upload order. A registry for
-- listing, membership checks and /data lookups: appended by POST /sources and
-- never shrunk by an undo. What is on the timeline is the fold's doc.sources.
CREATE TABLE project_sources (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    media_id   TEXT NOT NULL,
    start_at   REAL NOT NULL, -- stitched offset (OFFSET is an SQL keyword)
    duration   REAL NOT NULL,
    PRIMARY KEY (project_id, position)
);
CREATE INDEX project_sources_media ON project_sources(media_id);

-- Source 0 is the project's own media. Its timing lives in meta.json, which
-- SQL cannot read, so the row records zeros; nothing reads them back.
INSERT INTO project_sources (project_id, position, media_id, start_at, duration)
SELECT id, 0, media_id, 0.0, 0.0 FROM projects;
```

Put this above the `#[cfg(test)] mod tests` block in `server/src/sources.rs`:

```rust
//! A project's sources: the videos laid end to end on its main track.
//!
//! Two records describe them, on purpose. `project_sources` is a registry of
//! every media item uploaded into the project — for listing, membership
//! checks and `/data` lookups. `POST /sources` appends to it and nothing ever
//! removes a row, not even an undo. What is on the timeline is the fold's:
//! the project's own media first, then every live `AddSource`
//! (`engine::all_sources`). Registry position 0 is the project's own media;
//! its timing lives in `meta.json`, so that row records zeros.

use sqlx::{Sqlite, SqlitePool};

use crate::error::AppResult;

/// Most videos one project's main track may hold.
pub const MAX_SOURCES: usize = 20;

/// Record `media_id` as uploaded into `project_id`, after every earlier row.
/// One statement, so two concurrent uploads cannot take the same position.
pub async fn register(
    db: &SqlitePool,
    project_id: &str,
    media_id: &str,
    start_at: f64,
    duration: f64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO project_sources (project_id, position, media_id, start_at, duration)
         SELECT ?, COALESCE(MAX(position), -1) + 1, ?, ?, ?
         FROM project_sources WHERE project_id = ?",
    )
    .bind(project_id)
    .bind(media_id)
    .bind(start_at)
    .bind(duration)
    .bind(project_id)
    .execute(db)
    .await?;
    Ok(())
}

/// Whether `media_id` was ever uploaded into `project_id`. Takes any
/// executor so `validate` can ask inside its write transaction.
pub async fn in_registry<'e, E>(exec: E, project_id: &str, media_id: &str) -> AppResult<bool>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM project_sources WHERE project_id = ? AND media_id = ? LIMIT 1",
    )
    .bind(project_id)
    .bind(media_id)
    .fetch_optional(exec)
    .await?;
    Ok(row.is_some())
}

/// Every media id uploaded into the project, in upload order.
pub async fn registry(db: &SqlitePool, project_id: &str) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT media_id FROM project_sources WHERE project_id = ? ORDER BY position",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?)
}
```

In `server/src/projects.rs`, `create_project`, insert the registry row inside the existing
transaction, right after the `project_members` insert and before `tx.commit()`:

```rust
    // The project's own media is source 0 of its registry. Its timing lives
    // in meta.json, so the row records zeros, as the 0005 backfill does.
    sqlx::query(
        "INSERT INTO project_sources (project_id, position, media_id, start_at, duration)
         VALUES (?, 0, ?, 0.0, 0.0)",
    )
    .bind(&project.id)
    .bind(&project.media_id)
    .execute(&mut *tx)
    .await?;
```

In `adopt_orphans`, replace the count query so a source's media is not an orphan:

```rust
        // Media used by a project, as its own media or as a later source, is
        // not an orphan.
        let (n,): (i64,) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM projects WHERE media_id = ?)
                  + (SELECT COUNT(*) FROM project_sources WHERE media_id = ?)",
        )
        .bind(&meta.id)
        .bind(&meta.id)
        .fetch_one(&state.db)
        .await?;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server sources && cargo test -p server db::`

Expected: PASS, for `a_new_project_registers_its_own_media_at_position_zero`,
`adopting_orphans_skips_media_that_is_already_a_source`, `open_creates_schema`,
`open_is_idempotent` and `sources_backfill_gives_every_project_its_first_source`. Clippy will
still flag `MAX_SOURCES` as unused; Round B uses it.

#### Round B: the timeline in the document, `add_source`, and op validation

- [ ] **Step 5: Write the failing tests**

In `server/src/test_util.rs`, add a helper below `owned_project`:

```rust
/// Pre-seed `media_id`'s transcript cache with `texts`, one word a second
/// from 0 (each half a second long), so nothing shells out to whisper.
pub async fn seed_words(state: &Arc<AppState>, media_id: &str, texts: &[&str]) {
    let words: Vec<engine::Word> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| engine::Word {
            id: format!("w{i}"),
            text: (*text).to_owned(),
            start: i as f64,
            end: i as f64 + 0.5,
        })
        .collect();
    tokio::fs::write(
        state
            .config
            .data_dir
            .join(media_id)
            .join(crate::routes::WORDS_CACHE),
        serde_json::to_vec(&words).unwrap(),
    )
    .await
    .unwrap();
}
```

In `server/src/sources.rs`, add a `test_support` module above `mod tests`:

```rust
#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use crate::projects::test_support::seed_media;
    use crate::routes::{read_meta, Meta};
    use crate::test_util::seed_words;
    use crate::AppState;

    /// A `duration`-second media item whose transcript cache already holds
    /// `words`, one a second from 0. Not yet registered with any project.
    pub async fn seed_source(state: &Arc<AppState>, duration: f64, words: &[&str]) -> Meta {
        let id = seed_media(state, duration).await;
        seed_words(state, &id, words).await;
        read_meta(&state.config.data_dir.join(&id)).await.unwrap()
    }
}
```

Replace the `use` lines of `mod tests` in `server/src/sources.rs`, and add the helpers and tests
below the Round A tests:

```rust
    use axum::http::{Method, StatusCode};
    use engine::MediaKind;
    use serde_json::{json, Value};

    use super::test_support::seed_source;
    use super::*;
    use crate::assets::test_support::seed_asset;
    use crate::projects::test_support::seed_media;
    use crate::test_util::{app, call, json_req, me, owned_project, register as sign_up, state};

    async fn post_ops(
        state: &Arc<AppState>,
        cookie: &str,
        project: &str,
        ops: Vec<Value>,
    ) -> (StatusCode, Value) {
        let (status, body, _) = call(
            app(state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/ops"),
                Some(cookie),
                Some(json!({ "ops": ops })),
            ),
        )
        .await;
        (status, body)
    }

    fn add_op(id: &str, media: &str, offset: f64, duration: f64) -> Value {
        json!({ "opId": id, "kind": "addsource", "media": media, "offset": offset, "duration": duration })
    }

    #[tokio::test]
    async fn adding_a_source_appends_at_the_stitched_end_and_shows_in_the_project() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        let view = add_source(&state, &project, &owner, &second).await.unwrap();
        assert_eq!(view.index, 1);
        assert_eq!(view.offset, 10.0);
        assert_eq!(view.duration, 4.0);
        assert_eq!(view.transcript, TranscriptStatus::Ready);
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap(),
            vec![project.media_id.clone(), second.id.clone()]
        );

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::GET,
                &format!("/api/projects/{}", project.id),
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let sources = body["project"]["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["index"], 0);
        assert_eq!(sources[0]["mediaId"], project.media_id.as_str());
        assert_eq!(sources[0]["offset"], 0.0);
        assert_eq!(sources[0]["duration"], 10.0);
        assert_eq!(sources[0]["transcript"], "ready");
        assert_eq!(sources[1]["mediaId"], second.id.as_str());
        assert_eq!(sources[1]["url"], second.url.as_str());
        assert_eq!(sources[1]["filename"], "clip.mp4");
        assert_eq!(sources[1]["kind"], "video");
        assert_eq!(sources[1]["offset"], 10.0);
        assert_eq!(
            body["project"]["media"]["id"],
            project.media_id.as_str(),
            "media stays for one release"
        );
        // The doc carries the fold's appended sources only; project.sources lists all.
        assert_eq!(
            body["doc"]["sources"],
            json!([{ "media": second.id, "offset": 10.0, "duration": 4.0 }])
        );
        assert_eq!(body["doc"]["splits"], json!([10.0]), "the join is a split");

        // Past the end of the first file is now inside the project.
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "c", "kind": "cut", "start": 12.0, "end": 13.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "c2", "kind": "cut", "start": 13.0, "end": 14.5 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "past the stitched end");
    }

    #[tokio::test]
    async fn add_source_ops_need_the_end_the_registry_and_the_real_duration() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let loose = seed_source(&state, 4.0, &["d"]).await;

        let (status, body) =
            post_ops(&state, &ada, &project.id, vec![add_op("a", &loose.id, 10.0, 4.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("not uploaded"), "{body}");

        register(&state.db, &project.id, &loose.id, 10.0, 4.0)
            .await
            .unwrap();
        let (status, body) =
            post_ops(&state, &ada, &project.id, vec![add_op("b", &loose.id, 9.0, 4.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("at the end"), "{body}");
        let (status, _) =
            post_ops(&state, &ada, &project.id, vec![add_op("c", &loose.id, 10.0, 5.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a duration the media does not have");

        let (status, body) =
            post_ops(&state, &ada, &project.id, vec![add_op("d", &loose.id, 10.0, 4.0)]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["sources"].as_array().unwrap().len(), 1, "appended sources only");
    }

    #[tokio::test]
    async fn the_twenty_first_source_is_refused() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        for _ in 1..MAX_SOURCES {
            let m = seed_source(&state, 1.0, &[]).await;
            add_source(&state, &project, &owner, &m).await.unwrap();
        }
        let extra = seed_source(&state, 1.0, &[]).await;
        let err = add_source(&state, &project, &owner, &extra)
            .await
            .unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.to_string().contains("20"), "{err}");
        // The log refuses it too, not just the helper.
        register(&state.db, &project.id, &extra.id, 29.0, 1.0)
            .await
            .unwrap();
        let (status, body) =
            post_ops(&state, &ada, &project.id, vec![add_op("x", &extra.id, 29.0, 1.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[tokio::test]
    async fn undo_and_redo_of_sources_keep_the_main_track_contiguous() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let a = seed_source(&state, 4.0, &["d"]).await;
        let b = seed_source(&state, 2.0, &["e"]).await;
        let c = seed_source(&state, 3.0, &["f"]).await;
        add_source(&state, &project, &owner, &a).await.unwrap(); // seq 1
        add_source(&state, &project, &owner, &b).await.unwrap(); // seq 2

        let undo = |id: &str, seq: i64| json!({ "opId": id, "kind": "undo", "targetSeq": seq });
        let redo = |id: &str, seq: i64| json!({ "opId": id, "kind": "redo", "targetSeq": seq });
        let (status, body) = post_ops(&state, &ada, &project.id, vec![undo("u1", 1)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("added after"), "{body}");

        let (status, body) = post_ops(&state, &ada, &project.id, vec![undo("u2", 2)]).await; // seq 3
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["sources"].as_array().unwrap().len(), 1, "appended sources only");

        // c takes the end b left; b cannot come back at its old place.
        add_source(&state, &project, &owner, &c).await.unwrap(); // seq 4, offset 14
        let (status, body) = post_ops(&state, &ada, &project.id, vec![redo("r1", 3)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("old place"), "{body}");
        assert_eq!(
            registry(&state.db, &project.id).await.unwrap().len(),
            4,
            "the registry never shrinks"
        );
    }

    #[tokio::test]
    async fn layers_check_track_media_length_sound_and_their_target() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let clip = seed_asset(&state, &project, MediaKind::Video, 3.0).await;
        let song = seed_asset(&state, &project, MediaKind::Audio, 30.0).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        let unrelated = seed_media(&state, 4.0).await;

        let layer = |id: &str, track: u8, media: &str, offset: f64, audio: Value| {
            json!({ "opId": id, "kind": "addlayer", "track": track, "start": 1.0, "end": 2.0,
                    "media": media, "offset": offset, "frame": "pipTopRight", "audio": audio })
        };
        let bad = [
            layer("b1", 1, &clip.id, 0.0, Value::Null),        // the main track is not a layer
            layer("b2", 4, &clip.id, 0.0, Value::Null),        // no V4
            layer("b3", 2, &song.id, 0.0, Value::Null),        // audio is not a picture
            layer("b4", 2, "nope", 0.0, Value::Null),          // not ours
            layer("b5", 2, &unrelated, 0.0, Value::Null),      // a media, but not this project's
            layer("b6", 2, &clip.id, 0.0, json!(20.0)),        // too loud
            layer("b7", 3, &second.id, 3.5, Value::Null),      // runs past the source's end
        ];
        for op in bad {
            let (status, body) = post_ops(&state, &ada, &project.id, vec![op.clone()]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{op}: {body}");
        }
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![
                layer("ok1", 2, &clip.id, 0.0, Value::Null),
                layer("ok2", 3, &second.id, 0.5, json!(-6.0)), // a stretch of video 2 over video 1
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let layers = |body: &Value| -> Vec<Value> {
            body["edits"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "layer")
                .cloned()
                .collect()
        };
        assert_eq!(layers(&body).len(), 2);

        let set = |id: &str, track: u8, start: f64, to: u8| {
            json!({ "opId": id, "kind": "setlayer", "track": track, "start": start,
                    "toTrack": to, "frame": "full", "audio": null })
        };
        for op in [
            set("s1", 3, 1.0, 2), // V2 already has a layer starting at 1.0
            set("s2", 3, 1.0, 1), // not a layer track
            set("s3", 2, 5.0, 2), // nothing starts there
        ] {
            let (status, body) = post_ops(&state, &ada, &project.id, vec![op.clone()]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{op}: {body}");
        }
        let (status, body) = post_ops(&state, &ada, &project.id, vec![set("s4", 3, 1.0, 3)]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let v3 = layers(&body).into_iter().find(|l| l["track"] == 3).unwrap();
        assert_eq!(v3["frame"], "full");
        assert_eq!(v3["audio"], Value::Null);

        let (status, _) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "r0", "kind": "removelayer", "track": 5, "start": 1.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = post_ops(
            &state,
            &ada,
            &project.id,
            vec![json!({ "opId": "r1", "kind": "removelayer", "track": 3, "start": 1.0 })],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(layers(&body).len(), 1);
    }
```

In `server/src/ws.rs`, in `hello_carries_the_doc_and_peers_and_presence_flows`, add after
`assert_eq!(hello["headSeq"], 0);`:

```rust
        // Appended sources only, as the fold holds them: none in a new project.
        assert!(hello["sources"].as_array().unwrap().is_empty());
```

`server/src/assets.rs`'s `references_count_by_kind` test already uses an `Edit::Layer` literal:
Task 1 Step 6 moved it. Nothing to change there.

- [ ] **Step 6: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server sources`

Expected: FAIL to compile, with `cannot find function `add_source` in this scope` and
`cannot find type `TranscriptStatus``.

- [ ] **Step 7: Implement the timeline, `add_source` and validation**

Replace the `use` block at the top of `server/src/sources.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use engine::{all_sources, stitched_duration, MediaKind, Op, ProjectDoc, Source};
use serde::Serialize;
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};
use uuid::Uuid;

use crate::auth::User;
use crate::error::{AppError, AppResult};
use crate::ops::{apply_ops, ClientOp};
use crate::projects::Project;
use crate::routes::{read_meta, Meta, WORDS_CACHE};
use crate::AppState;
```

Append below `registry` (above `test_support`):

```rust
/// Where a source's transcript is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptStatus {
    Pending,
    Running,
    Ready,
    Error,
}

/// One source as the client sees it: the fold's placement plus the file's
/// name, kind and URL, and how far its transcript has got.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub index: usize,
    pub media_id: String,
    pub url: String,
    pub filename: String,
    pub kind: MediaKind,
    pub offset: f64,
    pub duration: f64,
    pub transcript: TranscriptStatus,
}

/// The project's own media as source 0.
pub async fn first_source(state: &AppState, project: &Project) -> AppResult<Source> {
    let meta = read_meta(&state.config.data_dir.join(&project.media_id)).await?;
    Ok(Source {
        media: project.media_id.clone(),
        offset: 0.0,
        duration: meta.duration,
    })
}

/// Every source on the main track, in stitched order, the first included.
pub async fn timeline(
    state: &AppState,
    project: &Project,
    doc: &ProjectDoc,
) -> AppResult<Vec<Source>> {
    Ok(all_sources(first_source(state, project).await?, &doc.sources))
}

/// A media item's transcript: ready once its words are cached.
pub fn transcript_status(state: &AppState, media_id: &str) -> TranscriptStatus {
    if state
        .config
        .data_dir
        .join(media_id)
        .join(WORDS_CACHE)
        .is_file()
    {
        TranscriptStatus::Ready
    } else {
        TranscriptStatus::Pending
    }
}

/// `sources` with each file's name, kind, URL and transcript status.
pub async fn views(state: &AppState, sources: &[Source]) -> AppResult<Vec<SourceView>> {
    let mut out = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let meta = read_meta(&state.config.data_dir.join(&source.media)).await?;
        out.push(SourceView {
            index,
            media_id: source.media.clone(),
            url: meta.url,
            filename: meta.filename,
            kind: meta.kind,
            offset: source.offset,
            duration: source.duration,
            transcript: transcript_status(state, &source.media),
        });
    }
    Ok(out)
}

/// Refuse a project whose main track already holds `count` sources, if that
/// is the most it may hold.
pub fn check_room(count: usize) -> AppResult<()> {
    if count >= MAX_SOURCES {
        Err(AppError::bad_request(format!(
            "a project holds at most {MAX_SOURCES} videos, and this one is full"
        )))
    } else {
        Ok(())
    }
}

/// Register `meta` with the project and append it to the end of the main
/// track as `user`. Validation in `apply_ops` checks the offset, the
/// registry, the duration and the cap again under the write lock, so a
/// concurrent add fails there rather than overlapping.
pub async fn add_source(
    state: &Arc<AppState>,
    project: &Project,
    user: &User,
    meta: &Meta,
) -> AppResult<SourceView> {
    let (_, doc) = crate::ops::load_doc(state, &project.id).await?;
    let sources = timeline(state, project, &doc).await?;
    check_room(sources.len())?;
    let offset = stitched_duration(&sources);
    register(&state.db, &project.id, &meta.id, offset, meta.duration).await?;
    apply_ops(
        state,
        project,
        user,
        vec![ClientOp {
            op_id: Uuid::new_v4().to_string(),
            op: Op::AddSource {
                media: meta.id.clone(),
                offset,
                duration: meta.duration,
            },
        }],
    )
    .await?;
    Ok(SourceView {
        index: sources.len(),
        media_id: meta.id.clone(),
        url: meta.url.clone(),
        filename: meta.filename.clone(),
        kind: meta.kind,
        offset,
        duration: meta.duration,
        transcript: transcript_status(state, &meta.id),
    })
}

/// What a layer's `media` names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerMedia {
    /// A project asset, with its kind and length.
    Asset { kind: MediaKind, duration: f64 },
    /// A media item in the project's registry: one of its own sources.
    Source,
}

/// The one rule for what a layer may show, shared by op validation
/// (`layer_media`) and the export's file lookup (`assets::asset_files`): a
/// project asset first, else any media in the project's registry. So a
/// stretch of video 2 can go over video 1, and a layer over an undone source
/// still renders, because the registry never shrinks. `None` when it is
/// neither.
pub async fn resolve_layer_media(
    conn: &mut SqliteConnection,
    project_id: &str,
    media: &str,
) -> AppResult<Option<LayerMedia>> {
    if let Some((kind, duration)) = crate::assets::find(&mut *conn, project_id, media).await? {
        return Ok(Some(LayerMedia::Asset { kind, duration }));
    }
    if in_registry(&mut *conn, project_id, media).await? {
        return Ok(Some(LayerMedia::Source));
    }
    Ok(None)
}

/// Kind and length of a media a layer may show (see `resolve_layer_media`),
/// read inside `validate`'s write transaction.
pub async fn layer_media(
    tx: &mut Transaction<'_, Sqlite>,
    state: &AppState,
    project: &Project,
    media: &str,
) -> AppResult<Option<(MediaKind, f64)>> {
    Ok(match resolve_layer_media(&mut **tx, &project.id, media).await? {
        Some(LayerMedia::Asset { kind, duration }) => Some((kind, duration)),
        Some(LayerMedia::Source) => {
            let meta = read_meta(&state.config.data_dir.join(media)).await?;
            Some((meta.kind, meta.duration))
        }
        None => None,
    })
}

/// A registry media's source file and metadata, for the export planner.
/// Callers check registry membership first; `media` is then one of ours.
pub async fn source_file(state: &AppState, media: &str) -> AppResult<(PathBuf, Meta)> {
    let dir = state.config.data_dir.join(media);
    let meta = read_meta(&dir).await?;
    Ok((dir.join(format!("source.{}", meta.ext)), meta))
}
```

In `server/src/ops.rs`:

1. Extend the engine import and add the sources import:

```rust
use engine::{
    all_sources, apply_op, fold, piece_starts, stitched_duration, Edit, MediaKind, Op,
    ProjectDoc, SeqOp, Source, Transition, EPS, MAX_AUDIO, MAX_BROLL, MAX_CAPTIONS,
    MAX_GAIN_DB, MAX_SPEAKERS, MAX_SPLITS, MAX_TITLES, MIN_GAIN_DB,
};
```

```rust
use crate::sources::MAX_SOURCES;
```

2. Add `sources` to `DocState` after `order`:

```rust
    /// Sources appended after the project's own media, exactly as the fold
    /// holds them (`doc.sources`). `GET /api/projects/:id` lists every source,
    /// the first included, in `project.sources`.
    pub sources: Vec<Source>,
```

and fill it in `doc_state`:

```rust
pub async fn doc_state(state: &AppState, project: &Project, user: &User) -> AppResult<DocState> {
    let (head_seq, doc) = load_doc(state, &project.id).await?;
    let (undoable, redoable) = undo_targets(&state.db, &project.id, &user.id).await?;
    Ok(DocState {
        head_seq,
        edits: doc.edits,
        speaker_names: doc.speaker_names,
        undoable,
        redoable,
        transition: doc.transition,
        splits: doc.splits,
        order: doc.order,
        sources: doc.sources,
    })
}
```

3. `validate`: in the doc comment, change "`duration` is read once by the caller" to "`first` is
   read once by the caller". In the signature, replace `duration: f64,` with `first: &Source,`.
   Make the first statement of the body:

```rust
    // Every range is checked against the stitched main track, which this
    // batch may itself have lengthened with an `AddSource`.
    let duration = stitched_duration(&all_sources(first.clone(), &current.sources));
```

Add these closures after `check_range`:

```rust
    let check_track = |track: u8| -> AppResult<()> {
        if track == 2 || track == 3 {
            Ok(())
        } else {
            Err(AppError::bad_request_at(index, "a layer goes on track 2 or 3"))
        }
    };
    let check_sound = |audio: Option<f64>| -> AppResult<()> {
        match audio {
            Some(db) if !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&db) => Err(
                AppError::bad_request_at(index, "layer sound must be between -30 and 12 dB"),
            ),
            _ => Ok(()),
        }
    };
    let layer_at = |track: u8, start: f64| {
        current.edits.iter().any(|e| {
            matches!(e, Edit::Layer { track: t, start: s, .. } if *t == track && (s - start).abs() < EPS)
        })
    };
```

In the `Op::Undo { target_seq } | Op::Redo { target_seq }` arm, replace the final `Ok(())` (after
the `is_undo_row` check) with:

```rust
            if want_undo_row {
                // A redo revives whatever that undo row undid. A source can
                // only come back where it was if that is still the end.
                let revived: Option<(String,)> = sqlx::query_as(
                    "SELECT op FROM edit_ops WHERE project_id = ? AND undone_by = ?",
                )
                .bind(&project.id)
                .bind(target_seq)
                .fetch_optional(&mut **tx)
                .await?;
                if let Some((json,)) = revived {
                    if let Ok(Op::AddSource { offset, .. }) = serde_json::from_str::<Op>(&json) {
                        if (offset - duration).abs() > EPS {
                            return Err(AppError::bad_request_at(
                                index,
                                "a video was added since; this one cannot come back in its old place",
                            ));
                        }
                    }
                }
            } else if kind == "addsource" {
                // Undoing a video in the middle would leave a hole in the
                // main track: the later videos keep their offsets.
                let (later,): (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM edit_ops WHERE project_id = ? AND seq > ?
                     AND undone_by IS NULL AND json_extract(op, '$.kind') = 'addsource'",
                )
                .bind(&project.id)
                .bind(target_seq)
                .fetch_one(&mut **tx)
                .await?;
                if later > 0 {
                    return Err(AppError::bad_request_at(
                        index,
                        "remove the videos added after this one first",
                    ));
                }
            }
            Ok(())
```

In the `Op::AddBroll` arm, count layers instead of B-roll edits (B-roll folds to a V2 layer):

```rust
            if current
                .edits
                .iter()
                .filter(|e| matches!(e, Edit::Layer { .. }))
                .count()
                >= MAX_BROLL
            {
                return Err(AppError::bad_request_at(index, "too many layers"));
            }
```

Delete the arm Task 1 Step 12 added after `Op::RemoveAudio`, the one that rejects
`AddSource | AddLayer | SetLayer | RemoveLayer` with "not supported yet". Then, before
`Op::RemoveBroll { start } => check_range(*start, *start),`, add these arms:

```rust
        Op::AddSource {
            media,
            offset,
            duration: length,
        } => {
            if all_sources(first.clone(), &current.sources).len() >= MAX_SOURCES {
                return Err(AppError::bad_request_at(
                    index,
                    format!("a project holds at most {MAX_SOURCES} videos"),
                ));
            }
            if (offset - duration).abs() > EPS {
                return Err(AppError::bad_request_at(
                    index,
                    format!("a video can only be added at the end, at {duration} s"),
                ));
            }
            if !crate::sources::in_registry(&mut **tx, &project.id, media).await? {
                return Err(AppError::bad_request_at(
                    index,
                    "that media was not uploaded to this project",
                ));
            }
            let meta = read_meta(&state.config.data_dir.join(media))
                .await
                .map_err(|_| AppError::bad_request_at(index, "that media is missing"))?;
            if *length <= 0.0 || (meta.duration - length).abs() > EPS {
                return Err(AppError::bad_request_at(
                    index,
                    "the duration does not match the media",
                ));
            }
            Ok(())
        }
        Op::AddLayer {
            track,
            start,
            end,
            media,
            offset,
            audio,
            ..
        } => {
            check_track(*track)?;
            check_range(*start, *end)?;
            if *end <= *start {
                return Err(AppError::bad_request_at(index, "layer range is empty"));
            }
            let Some((kind, length)) =
                crate::sources::layer_media(tx, state, project, media).await?
            else {
                return Err(AppError::bad_request_at(
                    index,
                    "that media does not belong to this project",
                ));
            };
            if kind != MediaKind::Video {
                return Err(AppError::bad_request_at(index, "a layer needs a video"));
            }
            if *offset < 0.0 || offset + (end - start) > length + EPS {
                return Err(AppError::bad_request_at(
                    index,
                    "the layer runs past the end of its video",
                ));
            }
            check_sound(*audio)?;
            if current
                .edits
                .iter()
                .filter(|e| matches!(e, Edit::Layer { .. }))
                .count()
                >= MAX_BROLL
            {
                return Err(AppError::bad_request_at(index, "too many layers"));
            }
            Ok(())
        }
        Op::SetLayer {
            track,
            start,
            to_track,
            audio,
            ..
        } => {
            check_track(*track)?;
            check_track(*to_track)?;
            if !layer_at(*track, *start) {
                return Err(AppError::bad_request_at(
                    index,
                    format!("no layer starts at {start} on V{track}"),
                ));
            }
            if to_track != track && layer_at(*to_track, *start) {
                return Err(AppError::bad_request_at(
                    index,
                    format!("V{to_track} already has a layer starting there"),
                ));
            }
            check_sound(*audio)
        }
        Op::RemoveLayer { track, start } => {
            check_track(*track)?;
            check_range(*start, *start)
        }
```

4. In `apply_ops`, replace

```rust
    let dir = state.config.data_dir.join(&project.media_id);
    let duration = read_meta(&dir).await?.duration;
```

with

```rust
    let first = crate::sources::first_source(state, project).await?;
```

pass `&first` in place of `duration` in the `validate(..)` call, and publish the fold's appended
sources with the doc:

```rust
        let (head_seq, doc) = load_doc(state, &project.id).await?;
        state.bus.publish(
            &project.id,
            crate::bus::ServerMsg::Doc {
                seq: head_seq,
                author_id: user.id.clone(),
                head_seq,
                edits: doc.edits,
                speaker_names: doc.speaker_names,
                transition: doc.transition,
                splits: doc.splits,
                order: doc.order,
                sources: doc.sources,
            },
        );
```

5. `get_project` adds `project.sources` and keeps `project.media`:

```rust
/// `GET /api/projects/:id` — the project summary, its sources, and its
/// folded document. `project.media` is the first source and stays for one
/// release, so a tab running an older client keeps working.
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Value>> {
    let summary = summary(&state, &access.project, access.role).await?;
    let doc = doc_state(&state, &access.project, &access.user).await?;
    // The doc holds the appended sources; the project lists every one, first included.
    let first = crate::sources::first_source(&state, &access.project).await?;
    let sources = crate::sources::views(&state, &all_sources(first, &doc.sources)).await?;
    let mut project = serde_json::to_value(summary)?;
    project["sources"] = serde_json::to_value(sources)?;
    Ok(Json(json!({ "project": project, "doc": doc })))
}
```

6. In the ops test `get_project_returns_summary_and_empty_doc`, add:

```rust
        assert_eq!(body["project"]["sources"].as_array().unwrap().len(), 1);
        assert_eq!(body["project"]["sources"][0]["offset"], 0.0);
        assert!(body["doc"]["sources"].as_array().unwrap().is_empty(), "appended sources only");
```

In `server/src/bus.rs`, change the import to `use engine::{Edit, Source, Transition};`. Add this
field to both `ServerMsg::Hello` (after `order`) and `ServerMsg::Doc` (after `order`):

```rust
        /// Sources appended after the project's own media (the fold's `doc.sources`).
        #[serde(default)]
        sources: Vec<Source>,
```

Add `sources: vec![],` to the three test literals of `ServerMsg::Doc`: `doc(seq)` and the
serialisation test in `bus.rs`, and the lag test in `ws.rs`.

In `server/src/ws.rs`, `session`, the `let hello = match load_doc(..) { .. };` block gains one field,
the fold's appended sources, after `order`. It reads:

```rust
    let hello = match load_doc(&state, &project_id).await {
        Ok((head_seq, doc)) => {
            let peers = state.bus.peers(&project_id);
            let you = peers
                .iter()
                .find(|p| p.conn_id == conn_id)
                .cloned()
                .expect("just subscribed");
            ServerMsg::Hello {
                head_seq,
                edits: doc.edits,
                speaker_names: doc.speaker_names,
                transition: doc.transition,
                splits: doc.splits,
                order: doc.order,
                sources: doc.sources,
                peers,
                you,
            }
        }
        Err(e) => ServerMsg::Error {
            code: "load".into(),
            detail: format!("{e:?}"),
        },
    };
```

In `server/src/assets.rs`, add `use crate::sources::LayerMedia;` to the imports, then replace
`references`, `asset_files` and the conflict message in `delete`:

```rust
/// How many layers and music edits use `asset_id`.
pub fn references(edits: &[Edit], asset_id: &str) -> (usize, usize) {
    edits.iter().fold((0, 0), |(l, a), e| match e {
        Edit::Layer { media, .. } if media == asset_id => (l + 1, a),
        Edit::Audio { media, .. } if media == asset_id => (l, a + 1),
        _ => (l, a),
    })
}

/// The file behind every layer and music edit, by media id. Music plays
/// project assets only. A layer resolves through
/// `sources::resolve_layer_media`, the same rule op validation uses: a
/// project asset, or any media in the project's registry.
pub async fn asset_files(
    state: &AppState,
    project: &Project,
    edits: &[Edit],
) -> AppResult<HashMap<String, PathBuf>> {
    let by_id: HashMap<String, Asset> = list_for(&state.db, project)
        .await?
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();
    let not_ours = |media: &str| {
        AppError::bad_request(format!("asset {media} does not belong to this project"))
    };
    let mut conn = state.db.acquire().await?;
    let mut files = HashMap::new();
    for edit in edits {
        let (media, layer) = match edit {
            Edit::Layer { media, .. } => (media, true),
            Edit::Audio { media, .. } => (media, false),
            _ => continue,
        };
        if files.contains_key(media) {
            continue;
        }
        let found = if layer {
            crate::sources::resolve_layer_media(&mut conn, &project.id, media).await?
        } else {
            by_id.get(media).map(|a| LayerMedia::Asset {
                kind: a.kind,
                duration: a.duration,
            })
        };
        let (path, name) = match found {
            Some(LayerMedia::Asset { .. }) => {
                let asset = by_id.get(media).ok_or_else(|| not_ours(media))?;
                (
                    assets_dir(state, project).join(format!("{}.{}", asset.id, asset.ext)),
                    asset.name.clone(),
                )
            }
            Some(LayerMedia::Source) => {
                let (path, meta) = crate::sources::source_file(state, media).await?;
                (path, meta.filename)
            }
            None => return Err(not_ours(media)),
        };
        if !path.is_file() {
            return Err(AppError::bad_request(format!(
                "the file for {name} is missing"
            )));
        }
        files.insert(media.clone(), path);
    }
    Ok(files)
}
```

```rust
    let (layers, audios) = references(&doc.edits, &asset.id);
    if layers + audios > 0 {
        return Err(AppError::conflict(format!(
            "{} is used by {layers} layer and {audios} music edit{}; remove them first",
            asset.name,
            if layers + audios == 1 { "" } else { "s" }
        )));
    }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server`

Expected: PASS, the whole server suite. That includes the five new `sources::tests`, the
extended `get_project_returns_summary_and_empty_doc` and
`hello_carries_the_doc_and_peers_and_presence_flows`, and the unchanged
`broll_and_audio_check_the_asset_kind_range_offset_and_gain`. If
`adding_a_source_appends_at_the_stitched_end_and_shows_in_the_project` fails on `splits`,
Task 1's `AddSource` fold is not adding the join split. Fix the engine, not the expectation.

#### Round C: the `/sources` route, unreadable files, thumbnails per source

- [ ] **Step 9: Write the failing tests**

Add these to `mod tests` in `server/src/sources.rs`:

```rust
    /// A one-field multipart upload of `bytes` as `filename` to `/sources`.
    fn upload_req(
        project: &str,
        cookie: &str,
        filename: &str,
        bytes: &[u8],
    ) -> axum::http::Request<axum::body::Body> {
        let mut body = format!(
            "--x\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n--x--\r\n");
        axum::http::Request::builder()
            .method(Method::POST)
            .uri(format!("/api/projects/{project}/sources"))
            .header(axum::http::header::COOKIE, cookie)
            .header(
                axum::http::header::CONTENT_TYPE,
                "multipart/form-data; boundary=x",
            )
            .body(axum::body::Body::from(body))
            .unwrap()
    }

    async fn media_dirs(state: &Arc<AppState>) -> usize {
        let mut entries = tokio::fs::read_dir(&state.config.data_dir).await.unwrap();
        let mut n = 0;
        while let Some(e) = entries.next_entry().await.unwrap() {
            if e.path().is_dir() {
                n += 1;
            }
        }
        n
    }

    #[tokio::test]
    async fn sources_route_checks_role_type_and_readability() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let bob = sign_up(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        crate::test_util::add_member(&state, &ada, &project.id, "bob@example.com", "viewer").await;

        let (status, _, _) = call(app(&state), upload_req(&project.id, &bob, "a.mp4", b"x")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "a.txt", b"hi")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let before = media_dirs(&state).await;
        let (status, body, _) = call(
            app(&state),
            upload_req(&project.id, &ada, "a.mp4", b"not really video"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("could not read media"),
            "{body}"
        );
        // Nothing appended: no registry row, no op, no directory left behind.
        assert_eq!(registry(&state.db, &project.id).await.unwrap().len(), 1);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert!(doc.sources.is_empty());
        assert_eq!(media_dirs(&state).await, before);
    }

    #[tokio::test]
    async fn sources_route_refuses_a_full_project_before_reading_the_file() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        for _ in 1..MAX_SOURCES {
            let m = seed_source(&state, 1.0, &[]).await;
            add_source(&state, &project, &owner, &m).await.unwrap();
        }
        // Unreadable, so only the cap can explain a message naming 20.
        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "a.mp4", b"junk")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("at most 20"), "{body}");
    }

    #[tokio::test]
    async fn sources_route_appends_a_real_file_at_the_end() {
        let (state, dir) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let wav = dir.path().join("tone.wav");
        let made = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=2")
            .args(["-ac", "1", "-ar", "16000"])
            .arg(&wav)
            .status()
            .await
            .expect("ffmpeg on PATH");
        assert!(made.success());
        let bytes = tokio::fs::read(&wav).await.unwrap();

        let (status, body, _) =
            call(app(&state), upload_req(&project.id, &ada, "tone.wav", &bytes)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["index"], 1);
        assert_eq!(body["offset"], 10.0);
        assert_eq!(body["kind"], "audio");
        assert_eq!(body["filename"], "tone.wav");
        assert!((body["duration"].as_f64().unwrap() - 2.0).abs() < 0.05, "{body}");
        let media = body["mediaId"].as_str().unwrap().to_owned();
        assert_eq!(body["url"], format!("/data/{media}/source.wav"));
        assert_eq!(registry(&state.db, &project.id).await.unwrap()[1], media);
        let (_, doc) = crate::ops::load_doc(&state, &project.id).await.unwrap();
        assert_eq!(doc.sources.len(), 1);
        assert_eq!(doc.sources[0].media, media);
    }

    #[tokio::test]
    async fn thumbnails_accept_any_registry_media_and_nothing_else() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        let stranger = seed_media(&state, 4.0).await;
        let thumbs = |media: &str| {
            json_req(
                Method::POST,
                &format!("/api/projects/{}/thumbnails", project.id),
                Some(&ada),
                Some(json!({ "media": media })),
            )
        };
        let (status, body, _) = call(app(&state), thumbs(&stranger)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("not one of this project's"),
            "{body}"
        );
        // A source gets past the check; the seeded file has no frames, so
        // ffmpeg fails and the route answers 502, not 400.
        let (status, body, _) = call(app(&state), thumbs(&second.id)).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    }
```

- [ ] **Step 10: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server sources_route thumbnails_accept`

Expected: FAIL.

- The route tests get `404 Not Found` (no `/sources` route), so the `FORBIDDEN`/`BAD_REQUEST`
  assertions fail.
- `thumbnails_accept_any_registry_media_and_nothing_else` fails its first assertion: the body
  is ignored and the first media renders. That gives 502, not 400.

- [ ] **Step 11: Implement the route, the upload cleanup and per-source thumbnails**

In `server/src/routes.rs`, make `store_upload` `pub(crate)` and remove its directory on any
failure after it is created:

```rust
/// Store a multipart `file` field as a new media item and probe it. On any
/// failure after the directory exists, the directory is removed, so a
/// rejected upload leaves nothing behind.
pub(crate) async fn store_upload(
    state: &AppState,
    mut field: axum::extract::multipart::Field<'_>,
) -> AppResult<Meta> {
    let filename = field.file_name().unwrap_or("upload").to_owned();
    let ext = Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(AppError::bad_request(format!(
            "unsupported file type .{ext}; use one of {}",
            ALLOWED_EXTENSIONS.join(", ")
        )));
    }

    let id = Uuid::new_v4().to_string();
    let dir = state.config.data_dir.join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating media dir")?;
    let source_name = format!("source.{ext}");
    let source = dir.join(&source_name);

    let stored = async {
        let mut file = tokio::fs::File::create(&source)
            .await
            .context("creating upload")?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::bad_request(format!("upload interrupted: {e}")))?
        {
            file.write_all(&chunk).await.context("writing upload")?;
        }
        file.flush().await.context("flushing upload")?;
        media::probe(&source)
            .await
            .map_err(|e| AppError::bad_request(format!("could not read media: {e:#}")))
    }
    .await;
    let probe = match stored {
        Ok(probe) => probe,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }
    };
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename,
        ext,
        duration: probe.duration,
        kind: probe.kind,
        video: probe.video,
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)
        .await
        .context("writing meta.json")?;
    tracing::info!(id = meta.id, duration = meta.duration, kind = ?meta.kind, "uploaded");
    Ok(meta)
}
```

Replace the head of `thumbnails`, down to `let dir = media_dir(&state, &id)?;`:

```rust
#[derive(Default, Deserialize)]
pub struct ThumbnailsRequest {
    /// Which of the project's sources; the first when omitted.
    media: Option<String>,
}

/// `POST /api/projects/:id/thumbnails` — a sprite sheet of frames for the
/// scrubber preview, rendered once per video and cached. The body is
/// optional; `{ "media": id }` picks one of the project's sources.
///
/// Readable by every member, viewers included: the scrubber preview is part
/// of playback, and the sheet is rendered once per media and cached.
pub async fn thumbnails(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<ThumbnailsRequest>>,
) -> AppResult<Json<Thumbnails>> {
    let id = match body.and_then(|Json(b)| b.media) {
        Some(media) => {
            if !crate::sources::in_registry(&state.db, &access.project.id, &media).await? {
                return Err(AppError::bad_request(
                    "that media is not one of this project's videos",
                ));
            }
            media
        }
        None => access.project.media_id.clone(),
    };
    let dir = media_dir(&state, &id)?;
```

(The rest of `thumbnails` is unchanged.)

Append the route to `server/src/sources.rs`, below `source_file`. Add
`use axum::extract::{Multipart, State};`, `use axum::Json;` and
`use crate::projects::ProjectAccess;` to its `use` block.

```rust
/// `POST /api/projects/:id/sources` — multipart with one `file` field.
/// Stores the media, probes it, registers it and appends `AddSource` at the
/// current end of the main track. Editors and owners only. A full project is
/// refused before a byte of the body is read; an unreadable file is refused
/// with nothing registered, appended or left on disk.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    mut multipart: Multipart,
) -> AppResult<Json<SourceView>> {
    access.require_edit()?;
    let (_, doc) = crate::ops::load_doc(&state, &access.project.id).await?;
    check_room(1 + doc.sources.len())?;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?
    {
        if field.name() == Some("file") {
            let meta = crate::routes::store_upload(&state, field).await?;
            return Ok(Json(
                add_source(&state, &access.project, &access.user, &meta).await?,
            ));
        }
    }
    Err(AppError::bad_request("missing `file` field"))
}
```

In `server/src/app.rs`, add `sources` to the `use crate::{..}` list and register the route after
the `/assets/{aid}` route:

```rust
        .route("/api/projects/{id}/sources", post(sources::upload))
```

- [ ] **Step 12: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server`

Expected: PASS, including `sources_route_checks_role_type_and_readability`,
`sources_route_refuses_a_full_project_before_reading_the_file`,
`sources_route_appends_a_real_file_at_the_end` and
`thumbnails_accept_any_registry_media_and_nothing_else`. The real-file test needs `ffmpeg` and
`ffprobe` on `PATH`, as the server itself does.

- [ ] **Step 13: Verify and commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test -p server
git add server/migrations/0005_sources.sql server/src/sources.rs server/src/main.rs server/src/app.rs \
  server/src/db.rs server/src/projects.rs server/src/ops.rs server/src/bus.rs server/src/ws.rs \
  server/src/assets.rs server/src/routes.rs server/src/test_util.rs
git commit -m "server: project sources — registry, /sources upload, stitched timeline in the doc, layer validation" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 4: Server: per-source transcription, stitched words and speakers, suggestions

Every source is transcribed, once per media, in the background. The project's words, speakers
and suggestions read straight through all sources in stitched time. `/transcribe` reports each
source's progress. The MCP agent reads the same stitched transcript.

**Files:**

- Modify:
  - `server/src/sources.rs`: jobs, status, stitching, suggestions, `project_transcript`
  - `server/src/routes.rs`: `transcribe`, `speakers`, `suggest`; `SPEAKERS_CACHE` becomes
    `pub(crate)`; remove `transcript_for`
  - `server/src/main.rs`: `AppState::transcripts`
  - `server/src/test_util.rs`: the `transcripts` field, and no whisper
  - `server/src/mcp.rs`: `tool_open_project`, `apply_suggestions`

**Interfaces:**

- Consumes (Task 1): `engine::stitch_words(parts: &[(&Source, &[Word])]) -> Vec<Word>` and
  `engine::stitch_speakers(parts: &[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>)`.
- Consumes (Task 3): `sources::{timeline, views, add_source, transcript_status,
TranscriptStatus, SourceView}`, `test_util::seed_words`, `sources::test_support::seed_source`.
- Produces (`server/src/sources.rs`):

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum TranscriptJob { Running, Failed }
  pub fn start_transcription(state: &Arc<AppState>, media_id: &str);
  pub async fn stitched_words(state: &AppState, sources: &[Source]) -> AppResult<Vec<Word>>;
  pub struct SpeakerPart<'a> { pub offset: f64, pub words: usize, pub speakers: Option<&'a Speakers> }
  pub fn stitch_speaker_labels(parts: &[SpeakerPart]) -> Speakers;
  pub async fn stitched_speakers(state: &AppState, sources: &[Source]) -> AppResult<Speakers>;
  pub async fn suggest_project(state: &AppState, sources: &[Source], two_word: bool) -> AppResult<Suggestions>;
  pub async fn project_transcript(state: &Arc<AppState>, project: &Project) -> AppResult<(Vec<Source>, Vec<Word>, Option<Speakers>)>;
  ```

- Produces (`AppState`): `pub transcripts: Mutex<HashMap<String, sources::TranscriptJob>>`.
- Produces (HTTP):
  - `POST /transcribe` → `{ words: Word[], sources: SourceView[] }`
  - `POST /speakers` → stitched `Speakers`
  - `POST /suggest` → stitched `Suggestions`

#### Round A: jobs, status and stitched words

- [ ] **Step 1: Write the failing tests**

In `server/src/test_util.rs`, `state()`, keep the test suite away from whisper, and add the new
field:

```rust
    config.admin_password = None;
    // Background transcription must never find a real whisper in tests: a
    // job fails fast instead, which is what the status tests rely on.
    config.whisper_bin = "type-n-stitch-no-whisper-in-tests".into();
```

```rust
        agents: Default::default(),
        transcripts: Mutex::new(HashMap::new()),
```

Add to `mod tests` in `server/src/sources.rs`:

```rust
    async fn transcribe(state: &Arc<AppState>, cookie: &str, project: &str) -> Value {
        let (status, body, _) = call(
            app(state),
            json_req(
                Method::POST,
                &format!("/api/projects/{project}/transcribe"),
                Some(cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    fn ids(body: &Value) -> Vec<String> {
        body["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap().to_owned())
            .collect()
    }

    fn statuses(body: &Value) -> Vec<String> {
        body["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["transcript"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn transcribe_stitches_ready_sources_and_reports_each_status() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        add_source(&state, &project, &owner, &second).await.unwrap();
        // A third whose transcription is already under way.
        let third = seed_media(&state, 2.0).await;
        let third = read_meta(&state.config.data_dir.join(&third)).await.unwrap();
        state
            .transcripts
            .lock()
            .unwrap()
            .insert(third.id.clone(), TranscriptJob::Running);
        add_source(&state, &project, &owner, &third).await.unwrap();

        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(ids(&body), ["w0", "w1", "w2", "1:w0", "1:w1"]);
        assert_eq!(body["words"][3]["start"], 10.0);
        assert_eq!(body["words"][4]["end"], 11.5);
        assert_eq!(statuses(&body), ["ready", "ready", "running"]);
        assert_eq!(body["sources"][2]["offset"], 14.0);

        // It finishes: the words are cached and the job is cleared.
        seed_words(&state, &third.id, &["f"]).await;
        state.transcripts.lock().unwrap().remove(&third.id);
        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(ids(&body).last().unwrap(), "2:w0");
        assert_eq!(body["words"][5]["start"], 14.0);
        assert_eq!(statuses(&body), ["ready", "ready", "ready"]);
    }

    #[tokio::test]
    async fn a_failed_transcription_reports_error_and_polling_does_not_retry_it() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        // No words and no source file: the job's ffmpeg step fails.
        let broken = seed_media(&state, 4.0).await;
        let broken = read_meta(&state.config.data_dir.join(&broken)).await.unwrap();
        let view = add_source(&state, &project, &owner, &broken).await.unwrap();
        assert_eq!(view.transcript, TranscriptStatus::Running);
        for _ in 0..200 {
            if transcript_status(&state, &broken.id) != TranscriptStatus::Running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(transcript_status(&state, &broken.id), TranscriptStatus::Error);

        let body = transcribe(&state, &ada, &project.id).await;
        assert_eq!(statuses(&body), ["ready", "error"]);
        assert_eq!(ids(&body).len(), 3, "the first source's words still come back");
        assert_eq!(
            transcript_status(&state, &broken.id),
            TranscriptStatus::Error,
            "polling must not start it again"
        );
    }
```

Extend that module's imports with `use crate::test_util::seed_words;`. `read_meta`,
`TranscriptJob` and `transcript_status` come in through `use super::*`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server transcri`

Expected: FAIL to compile, with `no field `transcripts`on type`AppState`` and
`cannot find type `TranscriptJob``.

- [ ] **Step 3: Implement jobs, status and stitched words**

In `server/src/main.rs`, add to `AppState` after `agents`:

```rust
    /// Background transcriptions of added sources, by media id: running, or
    /// failed until the server restarts. A finished one leaves no entry; its
    /// words cache is the record.
    pub transcripts: Mutex<HashMap<String, sources::TranscriptJob>>,
```

and initialise it in `main` with `transcripts: Mutex::new(HashMap::new()),`.

In `server/src/sources.rs`, add `use anyhow::Context;` and `use engine::Word;` to the imports.
Replace `transcript_status` and add the job code and `stitched_words` below it:

```rust
/// A background transcription the server started and has not cached yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptJob {
    Running,
    /// Kept until the server restarts, so a client polling `/transcribe`
    /// cannot set off an endless retry. The cause is in the log.
    Failed,
}

/// A media item's transcript: ready once its words are cached, otherwise
/// whatever its background job says, and pending when there is none.
pub fn transcript_status(state: &AppState, media_id: &str) -> TranscriptStatus {
    if state
        .config
        .data_dir
        .join(media_id)
        .join(WORDS_CACHE)
        .is_file()
    {
        return TranscriptStatus::Ready;
    }
    match state
        .transcripts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(media_id)
    {
        Some(TranscriptJob::Running) => TranscriptStatus::Running,
        Some(TranscriptJob::Failed) => TranscriptStatus::Error,
        None => TranscriptStatus::Pending,
    }
}

/// Transcribe `media_id` in the background unless it is cached, running or
/// has failed. The words land in the per-media cache, exactly as a
/// foreground `transcribe_item` would leave them.
pub fn start_transcription(state: &Arc<AppState>, media_id: &str) {
    if transcript_status(state, media_id) != TranscriptStatus::Pending {
        return;
    }
    {
        let mut jobs = state.transcripts.lock().unwrap_or_else(|e| e.into_inner());
        // Checked again under the lock: two callers may both have seen pending.
        if jobs.contains_key(media_id) {
            return;
        }
        jobs.insert(media_id.to_owned(), TranscriptJob::Running);
    }
    let state = state.clone();
    let id = media_id.to_owned();
    tokio::spawn(async move {
        let result = crate::routes::transcribe_item(&state, &id).await;
        let mut jobs = state.transcripts.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            // The cache file now answers `ready`.
            Ok(_) => {
                jobs.remove(&id);
            }
            Err(e) => {
                tracing::warn!(id, "background transcription failed: {e:?}");
                jobs.insert(id, TranscriptJob::Failed);
            }
        }
    });
}

/// A media item's cached words, or none while it is still being transcribed.
async fn cached_words(state: &AppState, media_id: &str) -> AppResult<Vec<Word>> {
    match tokio::fs::read_to_string(state.config.data_dir.join(media_id).join(WORDS_CACHE)).await
    {
        Ok(json) => Ok(serde_json::from_str(&json).context("parsing cached words")?),
        Err(_) => Ok(Vec::new()),
    }
}

/// The project's words: each ready source's words shifted by its offset, in
/// source order. A source still transcribing contributes nothing yet.
pub async fn stitched_words(state: &AppState, sources: &[Source]) -> AppResult<Vec<Word>> {
    let mut per_source = Vec::with_capacity(sources.len());
    for source in sources {
        per_source.push(cached_words(state, &source.media).await?);
    }
    let parts: Vec<(&Source, &[Word])> = sources
        .iter()
        .zip(&per_source)
        .map(|(s, w)| (s, w.as_slice()))
        .collect();
    Ok(engine::stitch_words(&parts))
}
```

In `add_source`, start the job right after `apply_ops(..).await?;` so the view reports it:

```rust
    start_transcription(state, &meta.id);
```

In `server/src/routes.rs`, replace `Transcript` and `transcribe`. Leave `transcript_for` in
place for now: `mcp.rs` still calls it, and Step 7 removes both together.

```rust
#[derive(Serialize)]
pub struct Transcript {
    /// Every ready source's words in stitched time.
    words: Vec<Word>,
    /// Every source, with how far its transcript has got.
    sources: Vec<crate::sources::SourceView>,
}

/// `POST /api/projects/:id/transcribe` — the project's stitched words, and
/// each source's transcript status.
///
/// The first source is transcribed before answering, as it always was, so
/// a client that predates sources gets its words exactly as before. Every
/// other source is transcribed in the background; the client asks again
/// while any is `pending` or `running`.
///
/// Deliberately open to every member, viewers included: the transcript *is*
/// the document, so a viewer cannot see the project without it. The result is
/// cached per media, so a viewer cannot force repeated work either.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Transcript>> {
    transcribe_item(&state, &access.project.media_id).await?;
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    for source in sources.iter().skip(1) {
        crate::sources::start_transcription(&state, &source.media);
    }
    let words = crate::sources::stitched_words(&state, &sources).await?;
    let sources = crate::sources::views(&state, &sources).await?;
    Ok(Json(Transcript { words, sources }))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server transcri`

Expected: PASS, for `transcribe_stitches_ready_sources_and_reports_each_status` and
`a_failed_transcription_reports_error_and_polling_does_not_retry_it`. The rest of the suite
still passes, because `transcript_for` and the MCP code that calls it are unchanged.

#### Round B: speakers, suggestions, and the agent's transcript

- [ ] **Step 5: Write the failing tests**

In `server/src/routes.rs`, make the speaker cache name visible to tests:
`pub(crate) const SPEAKERS_CACHE: &str = "speakers-v1.json";`.

Add to `mod tests` in `server/src/sources.rs`:

```rust
    /// Pre-seed `media`'s diarization cache.
    async fn seed_speakers(state: &Arc<AppState>, media: &str, speakers: &Speakers) {
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(media)
                .join(crate::routes::SPEAKERS_CACHE),
            serde_json::to_vec(speakers).unwrap(),
        )
        .await
        .unwrap();
    }

    fn turn(start: f64, end: f64, speaker: u32) -> engine::SpeakerTurn {
        engine::SpeakerTurn { start, end, speaker }
    }

    #[test]
    fn speaker_labels_are_namespaced_by_the_counts_before_them() {
        let a = Speakers {
            count: 2,
            words: vec![Some(0), Some(1), None],
            turns: vec![turn(0.0, 1.0, 0), turn(1.0, 2.0, 1)],
        };
        let b = Speakers {
            count: 1,
            words: vec![Some(0)],
            turns: vec![turn(0.0, 1.0, 0)],
        };
        let out = stitch_speaker_labels(&[
            SpeakerPart { offset: 0.0, words: 3, speakers: Some(&a) },
            SpeakerPart { offset: 10.0, words: 2, speakers: None },
            SpeakerPart { offset: 14.0, words: 1, speakers: Some(&b) },
        ]);
        assert_eq!(out.count, 3);
        assert_eq!(out.words, [Some(0), Some(1), None, None, None, Some(2)]);
        assert_eq!(out.turns.last(), Some(&turn(14.0, 15.0, 2)));

        // The base rule is the engine's: a count below the labels a source
        // uses never lets two sources share a speaker, in labels or turns.
        let low = Speakers {
            count: 1,
            words: vec![Some(2)],
            turns: vec![turn(0.0, 1.0, 2)],
        };
        let out = stitch_speaker_labels(&[
            SpeakerPart { offset: 0.0, words: 1, speakers: Some(&low) },
            SpeakerPart { offset: 5.0, words: 1, speakers: Some(&b) },
        ]);
        assert_eq!(out.words, [Some(2), Some(3)]);
        assert_eq!(out.turns.last(), Some(&turn(5.0, 6.0, 3)));
        assert_eq!(out.count, 4);
    }

    #[tokio::test]
    async fn speakers_read_through_every_source_up_to_the_first_untranscribed_one() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        seed_speakers(
            &state,
            &project.media_id,
            &Speakers {
                count: 2,
                words: vec![Some(0), Some(1), Some(0)],
                turns: vec![turn(0.0, 0.9, 0), turn(0.9, 1.9, 1), turn(1.9, 3.0, 0)],
            },
        )
        .await;
        let second = seed_source(&state, 4.0, &["d", "e"]).await;
        seed_speakers(
            &state,
            &second.id,
            &Speakers { count: 1, words: vec![Some(0), Some(0)], turns: vec![turn(0.0, 2.0, 0)] },
        )
        .await;
        add_source(&state, &project, &owner, &second).await.unwrap();

        let speakers = |state: Arc<AppState>| {
            let (ada, id) = (ada.clone(), project.id.clone());
            async move {
                let (status, body, _) = call(
                    app(&state),
                    json_req(
                        Method::POST,
                        &format!("/api/projects/{id}/speakers"),
                        Some(&ada),
                        None,
                    ),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                body
            }
        };
        let body = speakers(state.clone()).await;
        assert_eq!(body["count"], 3);
        assert_eq!(body["words"], json!([0, 1, 0, 2, 2]));
        assert_eq!(body["turns"][3], json!({ "start": 10.0, "end": 12.0, "speaker": 2 }));

        // A third source still transcribing, then a fourth that is ready:
        // the fourth gets no labels until the third is done, so its base
        // cannot move under a name.
        let third = seed_media(&state, 2.0).await;
        let third = read_meta(&state.config.data_dir.join(&third)).await.unwrap();
        state
            .transcripts
            .lock()
            .unwrap()
            .insert(third.id.clone(), TranscriptJob::Running);
        add_source(&state, &project, &owner, &third).await.unwrap();
        let fourth = seed_source(&state, 3.0, &["g"]).await;
        seed_speakers(
            &state,
            &fourth.id,
            &Speakers { count: 1, words: vec![Some(0)], turns: vec![turn(0.0, 1.0, 0)] },
        )
        .await;
        add_source(&state, &project, &owner, &fourth).await.unwrap();
        let body = speakers(state.clone()).await;
        assert_eq!(body["words"], json!([0, 1, 0, 2, 2, null]));
    }

    #[tokio::test]
    async fn suggestions_cover_every_ready_source_in_stitched_time() {
        let (state, _d) = state().await;
        let ada = sign_up(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = seed_source(&state, 4.0, &["um", "hello"]).await;
        // A silence in the second file, cached so nothing shells out.
        tokio::fs::write(
            state.config.data_dir.join(&second.id).join("silences-v1.json"),
            serde_json::to_vec(&[engine::Range::new(1.5, 3.2)]).unwrap(),
        )
        .await
        .unwrap();
        add_source(&state, &project, &owner, &second).await.unwrap();

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &format!("/api/projects/{}/suggest", project.id),
                Some(&ada),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // "um" is word 0 of the second file: [0, 1) there, [10, 11) here.
        assert_eq!(body["fillers"], json!([{ "kind": "cut", "start": 10.0, "end": 11.0 }]));
        // The pauses are the second file's own, shifted by its offset.
        let local = crate::routes::suggest_for(&state, &second.id, false)
            .await
            .unwrap();
        assert!(!local.pauses.is_empty());
        let shifted: Vec<Value> = local
            .pauses
            .iter()
            .map(|e| {
                let r = e.range();
                json!({ "kind": "cut", "start": r.start + 10.0, "end": r.end + 10.0 })
            })
            .collect();
        assert_eq!(body["pauses"], json!(shifted));
    }
```

In `server/src/mcp.rs`, `mod tests`, add:

```rust
    #[tokio::test]
    async fn open_project_reads_the_transcript_of_every_source() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = crate::sources::test_support::seed_source(&state, 4.0, &["d", "e"]).await;
        crate::sources::add_source(&state, &project, &owner, &second)
            .await
            .unwrap();
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        let out = session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        assert_eq!(out["duration"], 14.0);
        let transcript = out["transcript"].as_array().unwrap();
        assert_eq!(transcript.len(), 5);
        assert_eq!(transcript[3]["text"], "d");
        assert_eq!(transcript[3]["start"], 10.0);
        // The last word of the second file owns the gap to the stitched end.
        let cut = session.tool_cut(&identity, 4, 4).await.unwrap();
        assert_eq!(cut["touched"][0]["status"], "cut");
        let (_, doc) = ops::load_doc(&state, &project.id).await.unwrap();
        assert!(doc.edits.iter().any(|e| matches!(
            e,
            Edit::Cut { start, end, .. } if *start == 11.0 && *end == 14.0
        )));
    }
```

- [ ] **Step 6: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server speaker suggestions_cover open_project_reads`

Expected: FAIL to compile, with `cannot find function `stitch_speaker_labels``,
`cannot find struct `SpeakerPart``, and `cannot find type `Speakers`` in `sources::tests`.

- [ ] **Step 7: Implement speakers, suggestions and the agent's transcript**

In `server/src/sources.rs`, add `use engine::{Edit, SpeakerTurn};` and
`use crate::routes::{speakers_item, suggest_for, Speakers, Suggestions};` to the imports. Append:

```rust
/// One source's share of the stitched speaker list: where it sits, how many
/// words it has, and its diarization if there is one to use.
pub struct SpeakerPart<'a> {
    pub offset: f64,
    pub words: usize,
    pub speakers: Option<&'a Speakers>,
}

/// Concatenate per-source speaker labels, namespaced so speaker 0 of one
/// file and speaker 0 of the next are different people. The labels and the
/// count come from `engine::stitch_speakers`, the one implementation of the
/// base rule. This adds only what the engine does not know about: each
/// source's labels padded or cut to its word count, and its turns shifted
/// into stitched time by the same base as its labels. A part without
/// diarization labels its words `None` and adds no speakers.
pub fn stitch_speaker_labels(parts: &[SpeakerPart]) -> Speakers {
    // A cache out of step with its words is padded or cut to fit, so the
    // list stays parallel to the stitched words.
    let labels: Vec<(u32, Vec<Option<u32>>)> = parts
        .iter()
        .map(|part| match part.speakers {
            Some(s) => (
                s.count,
                (0..part.words)
                    .map(|i| s.words.get(i).copied().flatten())
                    .collect(),
            ),
            None => (0, vec![None; part.words]),
        })
        .collect();
    let slices: Vec<(u32, &[Option<u32>])> = labels
        .iter()
        .map(|(count, words)| (*count, words.as_slice()))
        .collect();
    let (count, words) = engine::stitch_speakers(&slices);
    let mut turns = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let Some(s) = part.speakers else {
            continue;
        };
        // The base the engine gave this part's labels: the stitched count of
        // the parts before it.
        let (base, _) = engine::stitch_speakers(&slices[..k]);
        turns.extend(s.turns.iter().map(|t| SpeakerTurn {
            start: t.start + part.offset,
            end: t.end + part.offset,
            speaker: t.speaker + base,
        }));
    }
    Speakers {
        count,
        words,
        turns,
    }
}

/// Speaker labels for the project's stitched words.
///
/// Labels stop at the first source that is not transcribed yet: only the
/// ready prefix is passed with its diarization, and every source after it
/// is passed with no labels (`None` for each word, no speakers) until it is. Otherwise that source's count,
/// once known, would shift every later base and move a name someone typed
/// onto a different voice. A source whose diarization fails counts as no
/// speakers. Fails only when no source could be diarized at all, with the
/// same error the single-file route gave.
pub async fn stitched_speakers(state: &AppState, sources: &[Source]) -> AppResult<Speakers> {
    let mut counts = Vec::with_capacity(sources.len());
    let mut diarized = Vec::with_capacity(sources.len());
    let mut labelled = true;
    let mut any = false;
    let mut first_error = None;
    for source in sources {
        counts.push(cached_words(state, &source.media).await?.len());
        labelled &= transcript_status(state, &source.media) == TranscriptStatus::Ready;
        let speakers = if labelled {
            match speakers_item(state, &source.media).await {
                Ok(s) => {
                    any = true;
                    Some(s)
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                    None
                }
            }
        } else {
            None
        };
        diarized.push(speakers);
    }
    if !any {
        return Err(first_error
            .unwrap_or_else(|| AppError::not_found("transcribe this media first")));
    }
    let parts: Vec<SpeakerPart> = sources
        .iter()
        .zip(&counts)
        .zip(&diarized)
        .map(|((s, n), sp)| SpeakerPart {
            offset: s.offset,
            words: *n,
            speakers: sp.as_ref(),
        })
        .collect();
    Ok(stitch_speaker_labels(&parts))
}

/// A suggested cut moved from a source's own time into stitched time.
fn shifted(edit: Edit, by: f64) -> Edit {
    match edit {
        Edit::Cut {
            start,
            end,
            transition,
        } => Edit::Cut {
            start: start + by,
            end: end + by,
            transition,
        },
        // The suggesters only ever produce cuts.
        other => other,
    }
}

/// Filler and pause cuts for every ready source, each computed on its own
/// words and silences and shifted by its offset. A cut never crosses a join:
/// each is bounded by its own file's duration. 404 while nothing is ready.
pub async fn suggest_project(
    state: &AppState,
    sources: &[Source],
    two_word: bool,
) -> AppResult<Suggestions> {
    let mut fillers = Vec::new();
    let mut pauses = Vec::new();
    let mut any = false;
    for source in sources {
        if transcript_status(state, &source.media) != TranscriptStatus::Ready {
            continue;
        }
        let part = suggest_for(state, &source.media, two_word).await?;
        any = true;
        fillers.extend(part.fillers.into_iter().map(|e| shifted(e, source.offset)));
        pauses.extend(part.pauses.into_iter().map(|e| shifted(e, source.offset)));
    }
    if !any {
        return Err(AppError::not_found("transcribe this media first"));
    }
    Ok(Suggestions { fillers, pauses })
}

/// Everything an agent reads at open: the project's sources, its stitched
/// words and its stitched speaker labels (`None` when diarization is not
/// available). The first source is transcribed first, as it always was; the
/// others are started in the background.
pub async fn project_transcript(
    state: &Arc<AppState>,
    project: &Project,
) -> AppResult<(Vec<Source>, Vec<Word>, Option<Speakers>)> {
    crate::routes::transcribe_item(state, &project.media_id).await?;
    let (_, doc) = crate::ops::load_doc(state, &project.id).await?;
    let sources = timeline(state, project, &doc).await?;
    for source in sources.iter().skip(1) {
        start_transcription(state, &source.media);
    }
    let words = stitched_words(state, &sources).await?;
    let speakers = stitched_speakers(state, &sources).await.ok();
    Ok((sources, words, speakers))
}
```

In `server/src/routes.rs`, replace `speakers` and `suggest`:

```rust
/// `POST /api/projects/:id/speakers` — who says each stitched word (cached
/// per media), namespaced per source.
///
/// Readable by every member, viewers included: speaker turns are part of
/// viewing the transcript, and the answer is cached per media.
pub async fn speakers(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Speakers>> {
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    crate::sources::stitched_speakers(&state, &sources)
        .await
        .map(Json)
}
```

```rust
/// `POST /api/projects/:id/suggest` — filler-word and long-pause cuts across
/// every ready source, in stitched time, which the client can apply as one
/// batch. The body is optional. Unlike the other read-only media routes this
/// one prepares edits, so it needs edit rights.
pub async fn suggest(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    body: Option<Json<SuggestRequest>>,
) -> AppResult<Json<Suggestions>> {
    access.require_edit()?;
    let two_word_fillers = body.is_some_and(|Json(b)| b.two_word_fillers);
    let (_, doc) = load_doc(&state, &access.project.id).await?;
    let sources = crate::sources::timeline(&state, &access.project, &doc).await?;
    crate::sources::suggest_project(&state, &sources, two_word_fillers)
        .await
        .map(Json)
}
```

In `server/src/routes.rs`, delete `transcript_for`; nothing calls it after the change below.

In `server/src/mcp.rs`, `tool_open_project`, replace the three reads (`meta`, `transcript_for`,
`load_doc`) with:

```rust
        let (sources, words, speakers) =
            crate::sources::project_transcript(&self.state, &project).await?;
        let duration = engine::stitched_duration(&sources);
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
```

Then set `open.duration = duration;`. In the returned JSON, use `"duration": duration` and
compute `outputDuration` from `duration`, not `meta.duration`.

In `apply_suggestions`, replace the `suggest_for` call and the later `load_doc` with:

```rust
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        let sources = crate::sources::timeline(&self.state, &project, &doc).await?;
        let suggestions = crate::sources::suggest_project(&self.state, &sources, false).await?;
        let suggested = match which {
            Suggestion::Fillers => suggestions.fillers,
            Suggestion::Pauses => suggestions.pauses,
        };
```

(delete the second `let (_, doc) = ops::load_doc(..)` that followed).

- [ ] **Step 8: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server`

Expected: PASS, the whole server suite. That includes
`speaker_labels_are_namespaced_by_the_counts_before_them`,
`speakers_read_through_every_source_up_to_the_first_untranscribed_one`,
`suggestions_cover_every_ready_source_in_stitched_time`,
`open_project_reads_the_transcript_of_every_source`, and the existing
`media_routes_require_membership_and_upload_creates_a_project`: `/suggest` still answers 404
before anything is transcribed. It also includes the existing MCP suggestion tests, which now
go through `suggest_project`.

- [ ] **Step 9: Verify and commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test -p server
git add server/src/sources.rs server/src/routes.rs server/src/main.rs server/src/test_util.rs server/src/mcp.rs
git commit -m "server: transcribe every source, stitch words and speakers, suggest across sources" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 5: Export — multi-source cuts normalised to the canvas

The planner learns that the main track is several files. Each output piece reads its own file at local time, pieces never straddle a join, and every file is fitted onto the first file's frame size and rate. The server hands the planner every source with its probed frame size, plus the stitched transcript for ducking. A one-file project renders exactly as before.

**Files:**

- Modify: `engine/src/ffmpeg.rs` (the planner and its tests)
- Modify: `server/src/routes.rs` (`start_export`, a new `source_meta` and `export_sources`)
- Modify: `server/src/ops.rs` (one test in the existing `tests` module)

**Interfaces:**

- Consumes (Task 1, contract):
  - `Source { media, offset, duration }` (`Clone + PartialEq + Debug`)
  - `locate(&[Source], f64) -> Option<(usize, f64)>`
  - `stitched_duration(&[Source]) -> f64`
  - `all_sources(Source, &[Source]) -> Vec<Source>`
  - `stitch_words(&[(&Source, &[Word])]) -> Vec<Word>`
  - `ProjectDoc.sources: Vec<Source>`
- Consumes (existing): `timeline_with`, `Segment`, `SegmentKind`, `EPS`, `Range::len`, `routes::read_meta`, `routes::media_dir`, `routes::read_words`, `routes::WORDS_CACHE`, `media::probe` (whose `Probe.video` is already the oriented `VideoInfo`).
- Produces (`engine/src/ffmpeg.rs`, re-exported from `engine`):

  ```rust
  /// One main-track file as the planner sees it.
  #[derive(Debug, Clone, PartialEq)]
  pub struct SourceInput {
      /// Where it sits on the stitched timeline.
      pub source: Source,
      /// The local file ffmpeg opens.
      pub path: PathBuf,
      pub kind: MediaKind,
      /// Frame size and rate, when the file has a picture and it was probed.
      pub video: Option<VideoInfo>,
  }
  pub fn canvas(sources: &[SourceInput]) -> VideoInfo;
  pub fn sources_kind(sources: &[SourceInput]) -> MediaKind;
  pub fn build_ffmpeg_args(
      sources: &[SourceInput],
      edits: &[Edit],
      opts: &ExportOptions,
  ) -> Result<Vec<String>, ExportError>;
  // ExportOptions: fields `duration` and `video` removed; everything else unchanged.
  ```

- Produces (`server/src/routes.rs`):

  ```rust
  /// Every main-track file of `project` in stitched order, and the stitched words.
  pub(crate) async fn export_sources(
      state: &AppState,
      project: &Project,
      doc_sources: &[Source],
  ) -> AppResult<(Vec<SourceInput>, Vec<Word>)>;
  ```

- [ ] **Step 1: Move the planner tests onto a list of sources**

In `engine/src/ffmpeg.rs`, inside `mod tests`, replace the `opts` helper with this version, which no longer sets `duration` or `video`:

```rust
    fn opts<'a>(
        kind: MediaKind,
        format: OutputFormat,
        output: &'a Path,
        overdub_audio: &'a HashMap<String, PathBuf>,
    ) -> ExportOptions<'a> {
        ExportOptions {
            kind,
            format,
            output,
            overdub_audio,
            title_images: titles(&[]),
            caption_images: captions(&[]),
            transition: Transition::None,
            splits: &[],
            order: &[],
            assets: assets(&[]),
            words: &[],
        }
    }
```

Replace the whole `args_with` helper with the following. It adds `HD`, `file`, `single` and `args_from`:

```rust
    const HD: VideoInfo = VideoInfo {
        width: 1280,
        height: 720,
        fps: 30.0,
    };

    /// One main-track file at `offset` for `duration` seconds.
    fn file(
        media: &str,
        path: &str,
        offset: f64,
        duration: f64,
        kind: MediaKind,
        video: Option<VideoInfo>,
    ) -> SourceInput {
        SourceInput {
            source: Source {
                media: media.into(),
                offset,
                duration,
            },
            path: PathBuf::from(path),
            kind,
            video,
        }
    }

    /// The ten-second single file every older test renders.
    fn single(kind: MediaKind) -> Vec<SourceInput> {
        match kind {
            MediaKind::Video => vec![file("m0", "in.mp4", 0.0, 10.0, kind, Some(HD))],
            MediaKind::Audio => vec![file("m0", "in.mp3", 0.0, 10.0, kind, None)],
        }
    }

    /// The whole argv, so tests can look at the inputs as well as the graph.
    fn args_with(
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> Result<Vec<String>, ExportError> {
        args_from(&single(kind), edits, kind, format, tweak)
    }

    /// Same, over an explicit list of main-track files.
    fn args_from(
        sources: &[SourceInput],
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> Result<Vec<String>, ExportError> {
        let none = HashMap::new();
        let out = PathBuf::from(format!("out.{}", format.extension()));
        let mut options = opts(kind, format, &out, &none);
        tweak(&mut options);
        build_ffmpeg_args(sources, edits, &options)
    }
```

Then point every direct call at a one-file list. The only remaining `Path::new("in.mp4")` and `Path::new("in.mp3")` in the file are these call sites:

```bash
sed -i '' -e 's/Path::new("in\.mp4")/\&single(MediaKind::Video)/g' \
          -e 's/Path::new("in\.mp3")/\&single(MediaKind::Audio)/g' engine/src/ffmpeg.rs
grep -n 'Path::new("in' engine/src/ffmpeg.rs   # expect no output
```

Replace `title_without_video_info_uses_the_default_frame_rate`. The "no video info" case now lives on the source:

```rust
    #[test]
    fn title_without_video_info_uses_the_default_frame_rate() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 1.0,
            text: "T".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }];
        let mut sources = single(MediaKind::Video);
        sources[0].video = None;
        let args = args_from(&sources, &edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.title_images = titles(&[(0, "/imgs/t.png")]);
        })
        .unwrap();
        assert!(has_run(&args, &["-framerate", "30", "-t", "1"]), "{args:?}");
    }
```

- [ ] **Step 2: Write the failing multi-source tests**

Append to `mod tests` in `engine/src/ffmpeg.rs`:

```rust
    const FHD: VideoInfo = VideoInfo {
        width: 1920,
        height: 1080,
        fps: 30.0,
    };
    const PAL: VideoInfo = VideoInfo {
        width: 1280,
        height: 720,
        fps: 25.0,
    };
    const AN: &str = AUDIO_NORMALIZE;
    /// What a 720p25 file gets to sit on a 1080p30 canvas.
    const FIT: &str = ",scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps=30";

    /// A 1080p30 file for 4 s, then a 720p25 file for 6 s.
    fn two_files() -> Vec<SourceInput> {
        vec![
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(FHD)),
            file("m1", "b.mov", 4.0, 6.0, MediaKind::Video, Some(PAL)),
        ]
    }

    #[test]
    fn one_source_graph_is_exactly_what_it_was() {
        let g = graph(&[cut(2.0, 4.0)], MediaKind::Video, OutputFormat::Mp4);
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=2,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [0:v]trim=start=4:end=10,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [0:a]atrim=start=4:end=10,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn two_sources_are_trimmed_from_their_own_files_and_fitted_to_the_first() {
        // The fold's permanent split at the join is in `splits`, as AddSource leaves it.
        let args = args_from(
            &two_files(),
            &[cut(1.0, 2.0)],
            MediaKind::Video,
            OutputFormat::Mp4,
            |o| o.splits = &[4.0],
        )
        .unwrap();
        assert_eq!(
            &args[..8],
            &["-y", "-hide_banner", "-loglevel", "error", "-i", "a.mp4", "-i", "b.mov"]
        );
        assert_eq!(
            filter_complex(&args),
            format!(
                "[0:v]trim=start=0:end=1,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=1,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [0:v]trim=start=2:end=4,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [0:a]atrim=start=2:end=4,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [1:v]trim=start=0:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v2];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a2];\
                 [v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[outv][outa]"
            )
        );
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
    }

    #[test]
    fn a_piece_that_crosses_a_join_is_split_there() {
        // No splits at all: the planner still reads each file for its own part.
        let g = filter_complex(
            &args_from(&two_files(), &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap(),
        )
        .to_owned();
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [1:v]trim=start=0:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v1];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn an_overdub_on_the_second_file_freezes_that_files_frame_on_the_canvas() {
        let edits = [Edit::Overdub {
            start: 5.0,
            end: 6.0,
            text: "hi".into(),
            audio_url: "/data/m/od.wav".into(),
            audio_duration: 2.0,
        }];
        let od = leak(HashMap::from([(
            "/data/m/od.wav".to_owned(),
            PathBuf::from("/srv/od.wav"),
        )]));
        let args = args_from(&two_files(), &edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.overdub_audio = od;
        })
        .unwrap();
        // Pieces: [0,4) file 0, [4,5) file 1, the 2 s hold, [6,10) file 1.
        assert!(has_run(&args, &["-i", "/srv/od.wav"]), "{args:?}");
        let g = filter_complex(&args);
        assert!(
            g.contains(&format!(
                "[1:v]trim=start=1:end=2,setpts=PTS-STARTPTS,select=eq(n\\,0),\
                 tpad=stop_mode=clone:stop_duration=2,trim=end=2,setpts=PTS-STARTPTS{FIT},setsar=1[v2];"
            )),
            "{g}"
        );
        assert!(g.contains("[2:a]aresample=48000"), "{g}");
        assert!(
            g.contains(&format!("[1:v]trim=start=2:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v3];")),
            "{g}"
        );
        assert!(g.ends_with("concat=n=4:v=1:a=1[outv][outa]"), "{g}");
    }

    #[test]
    fn an_audio_only_source_shows_black_on_the_canvas() {
        let sources = [
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(FHD)),
            file("m1", "b.m4a", 4.0, 6.0, MediaKind::Audio, None),
        ];
        let g = filter_complex(
            &args_from(&sources, &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap(),
        )
        .to_owned();
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a0];\
                 color=c=black:s=1920x1080:r=30:d=6,format=yuv420p,setsar=1[v1];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn the_canvas_is_the_first_file_with_a_picture() {
        assert_eq!(canvas(&two_files()), FHD);
        assert_eq!(sources_kind(&two_files()), MediaKind::Video);
        let audio_first = [
            file("m0", "a.m4a", 0.0, 4.0, MediaKind::Audio, None),
            file("m1", "b.mov", 4.0, 6.0, MediaKind::Video, Some(PAL)),
        ];
        assert_eq!(canvas(&audio_first), PAL);
        assert_eq!(sources_kind(&audio_first), MediaKind::Video);
        let audio_only = [file("m0", "a.m4a", 0.0, 4.0, MediaKind::Audio, None)];
        assert_eq!(canvas(&audio_only), DEFAULT_VIDEO);
        assert_eq!(sources_kind(&audio_only), MediaKind::Audio);
        let unprobed = [file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, None)];
        assert_eq!(canvas(&unprobed), DEFAULT_VIDEO);
    }

    #[test]
    fn rate_writes_ntsc_rates_as_fractions() {
        assert_eq!(rate(30.0), "30");
        assert_eq!(rate(25.0), "25");
        assert_eq!(rate(30000.0 / 1001.0), "30000/1001");
        assert_eq!(rate(24000.0 / 1001.0), "24000/1001");
        assert_eq!(rate(60000.0 / 1001.0), "60000/1001");
    }

    #[test]
    fn no_sources_is_nothing_to_export() {
        assert_eq!(
            args_from(&[], &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap_err(),
            ExportError::NothingToExport
        );
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p engine ffmpeg::
```

Expected: FAIL to compile, with errors such as:

```
error[E0422]: cannot find struct, variant or union type `SourceInput` in this scope
error[E0425]: cannot find function `canvas` in this scope
error[E0425]: cannot find function `sources_kind` in this scope
error[E0425]: cannot find function `rate` in this scope
error[E0308]: mismatched types (expected `&Path`, found `&[SourceInput]`)
```

- [ ] **Step 4: Implement the multi-source planner**

In `engine/src/ffmpeg.rs`:

Update the module doc's first paragraph:

```rust
//! Planning the ffmpeg render for an edit list.
//!
//! The main track is one or more source files laid end to end in stitched
//! time; each is its own input, in order. Every output piece becomes one
//! `trim`/`atrim` chain on the file that holds it (or, for an overdub, a
//! frozen first frame plus the synthesized WAV), fitted onto the canvas, and
//! the pieces are joined with the `concat` filter. Audio is resampled to a
//! common format first because `concat` refuses mismatched streams.
```

Replace the imports from `crate::editlist` and `crate::types`:

```rust
use crate::editlist::{
    audio_windows, broll_windows, caption_windows, joins, timeline_with, Segment, SegmentKind,
    Window, EPS, FADE,
};
use crate::types::{Edit, MediaKind, Range, Source, Transition, Word};
use crate::{locate, stitched_duration};
```

(Task 1 changed only the test module's imports in this file, so these lists are complete. Task 10 adds `Frame`.)

After `fn oriented`, add:

```rust
/// One main-track file as the planner sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInput {
    /// Where it sits on the stitched timeline.
    pub source: Source,
    /// The local file ffmpeg opens.
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Frame size and rate, when the file has a picture and it was probed.
    pub video: Option<VideoInfo>,
}

/// The frame every source is fitted onto: the first file with a picture
/// (an unprobed one counts as `DEFAULT_VIDEO`), else `DEFAULT_VIDEO`.
pub fn canvas(sources: &[SourceInput]) -> VideoInfo {
    sources
        .iter()
        .find(|s| s.kind == MediaKind::Video)
        .and_then(|s| s.video)
        .unwrap_or(DEFAULT_VIDEO)
}

/// `Video` when any source has a picture: the project renders to mp4.
pub fn sources_kind(sources: &[SourceInput]) -> MediaKind {
    if sources.iter().any(|s| s.kind == MediaKind::Video) {
        MediaKind::Video
    } else {
        MediaKind::Audio
    }
}
```

In `ExportOptions`, delete the `duration` field and the `video` field together with its doc comment. Change the doc on `kind` to:

```rust
    /// `Video` when any source has a picture; see `sources_kind`.
    pub kind: MediaKind,
```

Replace the start of `build_ffmpeg_args`: everything from its doc comment down to and including `let mut next_input = 1;`, together with the comment above that line. The new code:

```rust
/// Build the full ffmpeg argv (without the program name) that renders `edits`
/// over the main-track `sources` into `opts.output`. Input `k` is `sources[k]`.
pub fn build_ffmpeg_args(
    sources: &[SourceInput],
    edits: &[Edit],
    opts: &ExportOptions,
) -> Result<Vec<String>, ExportError> {
    if sources.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let placed: Vec<Source> = sources.iter().map(|s| s.source.clone()).collect();
    let segments = split_at_joins(
        timeline_with(stitched_duration(&placed), edits, opts.splits, opts.order),
        &placed,
    );
    if segments.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let with_video = opts.kind == MediaKind::Video;
    if !with_video && opts.format == OutputFormat::Mp4 {
        return Err(ExportError::FormatMismatch("mp4"));
    }
    let render_video = with_video && opts.format == OutputFormat::Mp4;

    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error"]
        .map(String::from)
        .to_vec();
    for s in sources {
        args.push("-i".into());
        args.push(s.path.to_string_lossy().into_owned());
    }

    let video_info = canvas(sources);
    let windows = caption_windows(&segments, edits);

    // Overdub WAVs, title cards, captions and inserts become extra inputs,
    // numbered in the order they are pushed after the sources.
    let mut next_input = sources.len();
```

In the per-segment loop, replace the lines from `let hold = seg.output.len();` through the end of the `let (v, a) = match seg.kind { … };` statement with:

```rust
        let hold = seg.output.len();
        let (k, local) = place(&placed, seg.source);
        let file = &sources[k];
        let picture = file.kind == MediaKind::Video;
        let (v, a) = match seg.kind {
            SegmentKind::Source => (
                render_video.then(|| {
                    if picture {
                        video_trim(k, local, &fit(file, video_info))
                    } else {
                        black(video_info, hold)
                    }
                }),
                audio_trim(k, local),
            ),
            SegmentKind::Overdub { index } => (
                render_video.then(|| {
                    if picture {
                        freeze_frame(
                            k,
                            local.start,
                            file.source.duration,
                            hold,
                            &fit(file, video_info),
                        )
                    } else {
                        black(video_info, hold)
                    }
                }),
                overdub_audio(overdub_input[&index], hold),
            ),
            SegmentKind::Title { index } => (
                render_video.then(|| title_still(title_input[&index], hold)),
                silence(hold),
            ),
        };
```

Replace `video_trim`, `audio_trim` and `freeze_frame` with these, and add the new helpers below them:

```rust
fn video_trim(input: usize, r: Range, fit: &str) -> String {
    format!(
        "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS{fit},setsar=1",
        fmt(r.start),
        fmt(r.end)
    )
}

fn audio_trim(input: usize, r: Range) -> String {
    format!(
        "[{input}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}",
        fmt(r.start),
        fmt(r.end)
    )
}

/// Take the frame at local time `at` of a file `duration` long and clone it
/// for `hold` seconds.
fn freeze_frame(input: usize, at: f64, duration: f64, hold: f64, fit: &str) -> String {
    // A one-second window guarantees at least one frame at any sane frame rate.
    let window_end = (at + 1.0).min(duration);
    format!(
        "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS,select=eq(n\\,0),\
         tpad=stop_mode=clone:stop_duration={hold},trim=end={hold},setpts=PTS-STARTPTS{fit},setsar=1",
        fmt(at),
        fmt(window_end),
        hold = fmt(hold),
    )
}

/// Black at the canvas size for `hold` seconds: the picture of an audio-only source.
fn black(canvas: VideoInfo, hold: f64) -> String {
    format!(
        "color=c=black:s={}x{}:r={}:d={},format=yuv420p,setsar=1",
        canvas.width,
        canvas.height,
        rate(canvas.fps),
        fmt(hold)
    )
}

/// Filters that bring `file`'s picture onto the canvas: nothing when it
/// already matches, else scale to fit, letterbox and resample the frame rate.
fn fit(file: &SourceInput, canvas: VideoInfo) -> String {
    let v = file.video.unwrap_or(DEFAULT_VIDEO);
    if v.width == canvas.width && v.height == canvas.height && (v.fps - canvas.fps).abs() < 1e-3 {
        return String::new();
    }
    format!(
        ",scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps={}",
        rate(canvas.fps),
        w = canvas.width,
        h = canvas.height,
    )
}

/// A frame rate for ffmpeg: NTSC rates as their exact fraction, others as
/// a plain number.
fn rate(fps: f64) -> String {
    let ntsc = fps * 1001.0 / 1000.0;
    if (ntsc - ntsc.round()).abs() < 1e-3 && (fps - fps.round()).abs() > 1e-3 {
        format!("{}000/1001", ntsc.round() as i64)
    } else {
        fmt(fps)
    }
}

/// The input holding a piece, and the piece's range in that file's own time.
fn place(placed: &[Source], r: Range) -> (usize, Range) {
    match locate(placed, r.start) {
        Some((k, local)) => (k, Range::new(local, local + r.len())),
        None => (0, r),
    }
}

/// Split every source piece at each join strictly inside it, so each piece
/// reads one file. Holds and titles are placed by their start and left whole.
fn split_at_joins(segments: Vec<Segment>, placed: &[Source]) -> Vec<Segment> {
    let joins: Vec<f64> = placed.iter().skip(1).map(|s| s.offset).collect();
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        if seg.kind != SegmentKind::Source {
            out.push(seg);
            continue;
        }
        let shift = seg.output.start - seg.source.start;
        let mut cursor = seg.source.start;
        for &j in &joins {
            if j > cursor + EPS && j < seg.source.end - EPS {
                out.push(Segment {
                    source: Range::new(cursor, j),
                    output: Range::new(cursor + shift, j + shift),
                    kind: SegmentKind::Source,
                });
                cursor = j;
            }
        }
        out.push(Segment {
            source: Range::new(cursor, seg.source.end),
            output: Range::new(cursor + shift, seg.output.end),
            kind: SegmentKind::Source,
        });
    }
    out
}
```

Every other part of the function (titles, captions, the B-roll block Task 1 left, music, fades, `concat`) is unchanged. It already reads `video_info`, which is now the canvas, and `segments`, which is now split at joins.

- [ ] **Step 5: Run the engine tests to verify they pass**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p engine ffmpeg::
```

Expected: PASS. That covers every older planner test with unchanged expectations, plus the eight new ones:

```
test ffmpeg::tests::one_source_graph_is_exactly_what_it_was ... ok
test ffmpeg::tests::two_sources_are_trimmed_from_their_own_files_and_fitted_to_the_first ... ok
test ffmpeg::tests::a_piece_that_crosses_a_join_is_split_there ... ok
test ffmpeg::tests::an_overdub_on_the_second_file_freezes_that_files_frame_on_the_canvas ... ok
test ffmpeg::tests::an_audio_only_source_shows_black_on_the_canvas ... ok
test ffmpeg::tests::the_canvas_is_the_first_file_with_a_picture ... ok
test ffmpeg::tests::rate_writes_ntsc_rates_as_fractions ... ok
test ffmpeg::tests::no_sources_is_nothing_to_export ... ok
```

If an older test's string changed, the single-source path is no longer identical. Fix the planner, not the expectation.

- [ ] **Step 6: Write the failing server test**

In `server/src/ops.rs`, inside `mod tests`, add:

```rust
    #[tokio::test]
    async fn export_sources_lists_every_file_with_stitched_words() {
        let (state, _d, _ada, _bob, project) = setup(None).await;
        let project = project_of(&state, &project).await;
        let second = seed_media(&state, 5.0).await;
        let words = vec![engine::Word {
            id: "w0".into(),
            text: "hi".into(),
            start: 1.0,
            end: 1.5,
        }];
        tokio::fs::write(
            state
                .config
                .data_dir
                .join(&second)
                .join(crate::routes::WORDS_CACHE),
            serde_json::to_vec(&words).unwrap(),
        )
        .await
        .unwrap();
        let doc_sources = [engine::Source {
            media: second.clone(),
            offset: 10.0,
            duration: 5.0,
        }];

        let (inputs, stitched) = crate::routes::export_sources(&state, &project, &doc_sources)
            .await
            .unwrap();

        assert_eq!(inputs.len(), 2);
        assert_eq!(
            inputs[0].source,
            engine::Source {
                media: project.media_id.clone(),
                offset: 0.0,
                duration: 10.0
            }
        );
        assert_eq!(
            inputs[0].path,
            state.config.data_dir.join(&project.media_id).join("source.mp4")
        );
        assert_eq!(inputs[1].source, doc_sources[0]);
        assert_eq!(
            inputs[1].path,
            state.config.data_dir.join(&second).join("source.mp4")
        );
        assert_eq!(inputs[1].kind, engine::MediaKind::Video);
        // The first file has no transcript: ducking is best-effort, not an error.
        assert_eq!(stitched.len(), 1);
        assert_eq!(stitched[0].id, "1:w0");
        assert_eq!(stitched[0].start, 11.0);
    }
```

- [ ] **Step 7: Run it to verify it fails**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p server export_sources_lists_every_file_with_stitched_words
```

Expected: FAIL to compile. `routes.rs` still calls the old `build_ffmpeg_args(&source, …)` with `duration`/`video` fields, and `export_sources` does not exist:

```
error[E0560]: struct `ExportOptions<'_>` has no field named `duration`
error[E0560]: struct `ExportOptions<'_>` has no field named `video`
error[E0425]: cannot find function `export_sources` in module `crate::routes`
```

- [ ] **Step 8: Pass every source to the planner**

In `server/src/routes.rs`, change the `engine` import to:

```rust
use engine::{
    all_sources, assign_speakers, build_ffmpeg_args, canvas, filler_cuts, output_duration,
    pause_cuts, silence_pause_cuts, sources_kind, stitch_words, stitched_duration, text,
    thumbnail_args, thumbnail_sheet, timeline_with, Edit, ExportError, ExportOptions, MediaKind,
    OutputFormat, Range, Source, SourceInput, SpeakerTurn, SuggestOptions, ThumbnailSheet,
    VideoInfo, Word, DEFAULT_VIDEO,
};
```

(Drop `DEFAULT_VIDEO` if clippy reports it unused after this step. Keep anything Tasks 3 and 4 added.)

Above `start_export`, add:

```rust
/// A media item's directory and meta. Media probed before frame sizes were
/// recorded is probed once more and the answer remembered, so later exports
/// skip the extra ffprobe. A failure is not fatal — the render falls back to
/// `DEFAULT_VIDEO` — but it is worth saying out loud.
async fn source_meta(state: &AppState, id: &str) -> AppResult<(PathBuf, Meta)> {
    let dir = media_dir(state, id)?;
    let mut meta = read_meta(&dir).await?;
    if meta.kind == MediaKind::Video && meta.video.is_none() {
        match media::probe(&dir.join(format!("source.{}", meta.ext))).await {
            Ok(probe) => {
                meta.video = probe.video;
                let json = serde_json::to_vec_pretty(&meta)?;
                if let Err(e) = write_json_atomic(&dir.join("meta.json"), &json).await {
                    tracing::warn!(id, "could not record the frame size: {e:#}");
                }
            }
            Err(e) => tracing::warn!(
                id,
                "could not re-probe the frame size, rendering at the default: {e:#}"
            ),
        }
    }
    Ok((dir, meta))
}

/// Every main-track file of `project` in stitched order, as the planner wants
/// them, and the stitched transcript for ducking. The project's own media is
/// source 0; `doc_sources` (the fold's) follow. A source with no transcript
/// yet contributes no words: ducking is best-effort.
pub(crate) async fn export_sources(
    state: &AppState,
    project: &Project,
    doc_sources: &[Source],
) -> AppResult<(Vec<SourceInput>, Vec<Word>)> {
    let first = read_meta(&media_dir(state, &project.media_id)?).await?;
    let placed = all_sources(
        Source {
            media: project.media_id.clone(),
            offset: 0.0,
            duration: first.duration,
        },
        doc_sources,
    );
    let mut inputs = Vec::with_capacity(placed.len());
    let mut transcripts = Vec::with_capacity(placed.len());
    for source in placed {
        let (dir, meta) = source_meta(state, &source.media).await?;
        transcripts.push(read_words(&dir).await.unwrap_or_default());
        inputs.push(SourceInput {
            path: dir.join(format!("source.{}", meta.ext)),
            kind: meta.kind,
            video: meta.video,
            source,
        });
    }
    let parts: Vec<(&Source, &[Word])> = inputs
        .iter()
        .zip(&transcripts)
        .map(|(i, w)| (&i.source, w.as_slice()))
        .collect();
    let words = stitch_words(&parts);
    Ok((inputs, words))
}
```

In `start_export`, replace everything from `let id = project.media_id.clone();` through the `let (title_images, caption_images) = …;` statement with:

```rust
    let id = project.media_id.clone();
    // Overdub WAVs and the rendered file live under the project's first media.
    let dir = media_dir(state, &id)?;
    let (_, doc) = load_doc(state, &project.id).await?;
    let (sources, words) = export_sources(state, project, &doc.sources).await?;
    let kind = sources_kind(&sources);
    let video = canvas(&sources);
    let duration = stitched_duration(
        &sources.iter().map(|s| s.source.clone()).collect::<Vec<_>>(),
    );
    // A title or caption with nothing but whitespace draws nothing; dropping
    // it here keeps the planner from asking for an image that would be blank.
    // `validate` rejects blank text on the way in, so this only catches
    // anything logged before that check existed.
    let edits: Vec<Edit> = doc
        .edits
        .into_iter()
        .filter(|e| match e {
            Edit::Caption { text, .. } | Edit::Title { text, .. } => !text.trim().is_empty(),
            _ => true,
        })
        .collect();
    let format = match format {
        None => OutputFormat::for_kind(kind),
        Some("mp4") => OutputFormat::Mp4,
        Some("mp3") => OutputFormat::Mp3,
        Some("wav") => OutputFormat::Wav,
        Some(other) => return Err(AppError::bad_request(format!("unknown format {other}"))),
    };
    let overdub_audio = overdub_files(&id, &dir, &edits)?;
    let asset_files = crate::assets::asset_files(state, project, &edits).await?;
    let (output, name) = next_numbered(&dir, "export", format.extension()).await?;

    // An audio-only render draws no picture, so it needs no images at all.
    // Text is rasterised at the canvas, the frame every source is fitted to.
    let render_video = kind == MediaKind::Video && format == OutputFormat::Mp4;
    let (title_images, caption_images) = if render_video {
        let dir = dir.clone();
        let edits = edits.clone();
        tokio::task::spawn_blocking(move || render_text_images(&dir, &edits, video))
            .await
            .context("rendering title and caption images")??
    } else {
        (HashMap::new(), HashMap::new())
    };
```

Replace the `build_ffmpeg_args(…)` call and the `planned` computation with:

```rust
    let args = build_ffmpeg_args(
        &sources,
        &edits,
        &ExportOptions {
            kind,
            format,
            output: &output,
            overdub_audio: &overdub_audio,
            title_images: &title_images,
            caption_images: &caption_images,
            transition: doc.transition,
            splits: &doc.splits,
            order: &doc.order,
            assets: &asset_files,
            words: &words,
        },
    )
    .map_err(|e| match e {
        // The planner and the renderer disagreed about the edit indices:
        // nothing the client sent can fix that.
        ExportError::MissingTitleImage(_) | ExportError::MissingCaptionImage(_) => {
            AppError::internal(e.to_string())
        }
        other => AppError::bad_request(other.to_string()),
    })?;
    let planned = output_duration(&timeline_with(duration, &edits, &doc.splits, &doc.order));
```

The rest of `start_export` (job bookkeeping, the spawned `ffmpeg_with_progress` task, the `/data/{id}/{name}` URL) is unchanged.

- [ ] **Step 9: Run the server tests and the whole suite**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p server export_
npm test && npm run lint
```

Expected: PASS. `export_sources_lists_every_file_with_stitched_words`, `export_uses_the_fold_and_viewers_may_not_export` (`planned` still 6.0) and `export_renders_title_images_before_ffmpeg` (`planned` still 11.0) are all ok. Lint is clean.

- [ ] **Step 10: Commit**

```bash
git add engine/src/ffmpeg.rs server/src/routes.rs server/src/ops.rs
git commit -m "export: render every source, each fitted and letterboxed onto the first file's canvas" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 6: Client — multi-file import on home and Insert → Add video

After this task, dropping several files on the home screen lists them sorted by name. They can be reordered by dragging (or ↑/↓) and imported as one project in that order, with per-file progress and per-file errors. In the editor, Insert → **Add video…** appends files at the end, showing progress in the viewer.

**Files:**

- Create: `client/src/importQueue.ts`, `client/src/importQueue.test.ts`, `client/src/components/ImportList.tsx`, `client/src/components/ImportList.module.css`, `client/src/components/Dropzone.test.tsx`
- Modify: `client/src/components/Dropzone.tsx`, `client/src/components/Home.module.css`, `client/src/components/TopBar.tsx` + `client/src/components/TopBar.test.tsx`, `client/src/App.tsx`, `client/src/App.module.css`

**Interfaces:**

- Consumes: Task 2's `uploadMedia(file, onProgress)`, `addSource(projectId, file, onProgress)`, `SourceView`, `sourceViewsOf`, the `addSource` editor action; existing `fetchProject`, `load`, `settle`.
- Produces:

  ```ts
  // importQueue.ts
  export const MEDIA_ACCEPT: string;
  export type ImportStatus = 'queued' | 'uploading' | 'done' | 'error';
  export interface ImportItem {
    id: string;
    name: string;
    status: ImportStatus;
    progress: number;
    error: string | null;
  }
  export interface ImportDeps {
    create?: (file: File, onProgress: (fraction: number) => void) => Promise<ProjectSummary>;
    append: (
      projectId: string,
      file: File,
      onProgress: (fraction: number) => void,
    ) => Promise<SourceView>;
  }
  export function byName(files: File[]): File[];
  export function moveItem<T>(list: T[], from: number, to: number): T[];
  export function itemsFor(files: File[]): ImportItem[];
  export function runImport(
    files: File[],
    deps: ImportDeps,
    onChange: (items: ImportItem[]) => void,
    projectId?: string | null,
  ): Promise<{ projectId: string | null; project: ProjectSummary | null; items: ImportItem[] }>;
  export function importFailures(items: ImportItem[]): string | null;
  // ImportList.tsx
  export function ImportList(props: {
    items: ImportItem[];
    onMove?: (from: number, to: number) => void;
    onRemove?: (index: number) => void;
  }): JSX.Element;
  // Dropzone.tsx — props: onImport(files: File[]) replaces onFile(file); gains imports: ImportItem[]
  // TopBar.tsx — EditorControls gains onAddVideos(files: File[]) and addingVideos: boolean
  ```

  DOM hooks: the home file input has `data-testid="media-input"`; the Insert menu's hidden input has `data-testid="add-video-input"`; `ImportList` is an `<ol aria-label="Files to import">`.

- [ ] **Step 1: Failing tests for the import queue**

Create `client/src/importQueue.test.ts`:

```ts
import { describe, expect, it } from 'vitest';

import { ApiError } from './api';
import {
  byName,
  importFailures,
  itemsFor,
  moveItem,
  runImport,
  type ImportDeps,
  type ImportItem,
} from './importQueue';
import type { ProjectSummary, SourceView } from './types';

const file = (name: string) => new File(['x'], name, { type: 'video/mp4' });

const summary: ProjectSummary = {
  id: 'p',
  title: 'b',
  role: 'owner',
  media: { id: 'm0', filename: 'b.mp4', ext: 'mp4', duration: 10, kind: 'video', url: '/m0' },
  createdAt: 0,
};
const view: SourceView = {
  index: 1,
  mediaId: 'm1',
  url: '/m1',
  filename: 'c.mp4',
  kind: 'video',
  offset: 10,
  duration: 5,
  transcript: 'pending',
};

describe('byName and moveItem', () => {
  it('sorts by name with natural numbering, ignoring case', () => {
    const names = byName([file('take10.mp4'), file('take2.mp4'), file('Take1.mov')]).map(
      (f) => f.name,
    );
    expect(names).toEqual(['Take1.mov', 'take2.mp4', 'take10.mp4']);
  });

  it('moves one item and keeps the rest in order', () => {
    expect(moveItem(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
    expect(moveItem(['a', 'b', 'c'], 2, 0)).toEqual(['c', 'a', 'b']);
    expect(moveItem(['a', 'b', 'c'], 1, 1)).toEqual(['a', 'b', 'c']);
    expect(moveItem(['a', 'b', 'c'], 0, 9)).toEqual(['a', 'b', 'c']);
  });

  it('starts every file queued', () => {
    expect(itemsFor([file('a.mp4')])).toEqual([
      { id: '0-a.mp4', name: 'a.mp4', status: 'queued', progress: 0, error: null },
    ]);
  });
});

describe('runImport', () => {
  function deps(calls: string[], failing: string[] = []): ImportDeps {
    return {
      create: async (f, onProgress) => {
        calls.push(`create ${f.name}`);
        if (failing.includes(f.name)) throw new ApiError('file too large', 413);
        onProgress(0.5);
        return summary;
      },
      append: async (id, f, onProgress) => {
        calls.push(`append ${id} ${f.name}`);
        if (failing.includes(f.name)) throw new ApiError('unreadable media', 400);
        onProgress(1);
        return view;
      },
    };
  }

  it('creates the project from the first file and appends the rest in order, one at a time', async () => {
    const calls: string[] = [];
    const seen: ImportItem[][] = [];
    const result = await runImport(
      [file('a.mp4'), file('b.mp4'), file('c.mp4')],
      deps(calls),
      (items) => seen.push(items),
    );
    expect(calls).toEqual(['create a.mp4', 'append p b.mp4', 'append p c.mp4']);
    expect(result.project).toBe(summary);
    expect(result.projectId).toBe('p');
    expect(result.items.map((i) => i.status)).toEqual(['done', 'done', 'done']);
    // a.mp4 reported its half-way mark, and no two files ever uploaded at once.
    expect(
      seen.some((items) => items[0]?.status === 'uploading' && items[0].progress === 0.5),
    ).toBe(true);
    expect(seen.some((items) => items.filter((i) => i.status === 'uploading').length > 1)).toBe(
      false,
    );
  });

  it('marks a failed file and carries on; the next file creates the project if the first fails', async () => {
    const calls: string[] = [];
    const result = await runImport(
      [file('bad.mp4'), file('b.mp4'), file('worse.mov'), file('c.mp4')],
      deps(calls, ['bad.mp4', 'worse.mov']),
      () => undefined,
    );
    expect(calls).toEqual([
      'create bad.mp4',
      'create b.mp4',
      'append p worse.mov',
      'append p c.mp4',
    ]);
    expect(result.items.map((i) => [i.status, i.error])).toEqual([
      ['error', 'file too large'],
      ['done', null],
      ['error', 'unreadable media'],
      ['done', null],
    ]);
    expect(importFailures(result.items)).toBe(
      'Could not import bad.mp4 (file too large); worse.mov (unreadable media)',
    );
    expect(importFailures(result.items.slice(1, 2))).toBeNull();
  });

  it('appends to an open project without creating one', async () => {
    const calls: string[] = [];
    const { append } = deps(calls);
    const result = await runImport([file('d.mp4')], { append }, () => undefined, 'p');
    expect(calls).toEqual(['append p d.mp4']);
    expect(result.project).toBeNull();
    expect(result.projectId).toBe('p');
  });
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `npx vitest run client/src/importQueue.test.ts`
Expected: FAIL. `./importQueue` does not exist.

- [ ] **Step 3: The import queue**

Create `client/src/importQueue.ts`:

```ts
// Importing several files as one project: order them, then upload one at a
// time. The first upload creates the project; the rest are appended to its
// main track. A file that fails is marked with the server's reason and the
// others carry on.

import type { ProjectSummary, SourceView } from './types';

/** What the pickers accept; the server probes and rejects anything unreadable. */
export const MEDIA_ACCEPT = '.mp3,.wav,.m4a,.mp4,.mov,audio/*,video/*';

export type ImportStatus = 'queued' | 'uploading' | 'done' | 'error';

export interface ImportItem {
  id: string;
  name: string;
  status: ImportStatus;
  /** Fraction of the file uploaded, 0 to 1. */
  progress: number;
  error: string | null;
}

export interface ImportDeps {
  /** Make a new project from `file`. Absent when appending to an open project. */
  create?: (file: File, onProgress: (fraction: number) => void) => Promise<ProjectSummary>;
  append: (
    projectId: string,
    file: File,
    onProgress: (fraction: number) => void,
  ) => Promise<SourceView>;
}

/** By file name, with natural numbering (take2 before take10), ignoring case. */
export function byName(files: File[]): File[] {
  return [...files].sort((a, b) =>
    a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }),
  );
}

/** `list` with the item at `from` moved to `to`; unchanged when either is out of range. */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length)
    return [...list];
  const next = [...list];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item as T);
  return next;
}

export function itemsFor(files: File[]): ImportItem[] {
  return files.map((f, i) => ({
    id: `${i}-${f.name}`,
    name: f.name,
    status: 'queued',
    progress: 0,
    error: null,
  }));
}

/**
 * Upload `files` in order. With no `projectId`, the first file that uploads
 * creates the project. `onChange` gets a new list on every status or progress
 * change.
 */
export async function runImport(
  files: File[],
  deps: ImportDeps,
  onChange: (items: ImportItem[]) => void,
  projectId: string | null = null,
): Promise<{ projectId: string | null; project: ProjectSummary | null; items: ImportItem[] }> {
  let items = itemsFor(files);
  const set = (i: number, patch: Partial<ImportItem>) => {
    items = items.map((item, k) => (k === i ? { ...item, ...patch } : item));
    onChange(items);
  };
  onChange(items);
  let id = projectId;
  let project: ProjectSummary | null = null;
  for (const [i, file] of files.entries()) {
    set(i, { status: 'uploading', progress: 0 });
    const onProgress = (fraction: number) => set(i, { progress: fraction });
    try {
      if (id === null) {
        if (!deps.create) throw new Error('no project to add this file to');
        project = await deps.create(file, onProgress);
        id = project.id;
      } else {
        await deps.append(id, file, onProgress);
      }
      set(i, { status: 'done', progress: 1 });
    } catch (err) {
      set(i, { status: 'error', error: err instanceof Error ? err.message : String(err) });
    }
  }
  return { projectId: id, project, items };
}

/** One line naming every failed file and why, or null when none failed. */
export function importFailures(items: ImportItem[]): string | null {
  const failed = items.filter((i) => i.status === 'error');
  if (failed.length === 0) return null;
  return `Could not import ${failed.map((i) => `${i.name} (${i.error ?? 'failed'})`).join('; ')}`;
}
```

- [ ] **Step 4: Run it and watch it pass**

Run: `npx vitest run client/src/importQueue.test.ts`
Expected: PASS.

- [ ] **Step 5: Failing tests for the import list on the home screen**

Create `client/src/components/Dropzone.test.tsx`:

```tsx
// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ImportItem } from '../importQueue';
import { Dropzone } from './Dropzone';

// The sample library fetches on mount; it is not under test here.
vi.mock('./Library', () => ({ Library: () => null }));

const file = (name: string) => new File(['x'], name, { type: 'video/mp4' });

function setup(imports: ImportItem[] = []) {
  const onImport = vi.fn();
  render(
    <Dropzone
      onImport={onImport}
      onLibraryClip={vi.fn()}
      busy={null}
      error={null}
      imports={imports}
    />,
  );
  return { onImport };
}

const choose = (files: File[]) =>
  fireEvent.change(screen.getByTestId('media-input'), { target: { files } });

const rows = () =>
  within(screen.getByRole('list', { name: 'Files to import' })).getAllByRole('listitem');

const names = () => rows().map((li) => li.querySelector('[title]')?.getAttribute('title'));

const importedNames = (onImport: ReturnType<typeof vi.fn>) =>
  (onImport.mock.calls[0]?.[0] as File[]).map((f) => f.name);

describe('Dropzone with several files', () => {
  it('imports a single file straight away, as before', () => {
    const { onImport } = setup();
    choose([file('a.mp4')]);
    expect(importedNames(onImport)).toEqual(['a.mp4']);
    expect(screen.queryByRole('list', { name: 'Files to import' })).toBeNull();
  });

  it('stages several files sorted by name, and imports them in the order shown', () => {
    const { onImport } = setup();
    choose([file('take10.mp4'), file('take2.mp4'), file('take1.mp4')]);
    expect(onImport).not.toHaveBeenCalled();
    expect(names()).toEqual(['take1.mp4', 'take2.mp4', 'take10.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Move take10.mp4 up' }));
    expect(names()).toEqual(['take1.mp4', 'take10.mp4', 'take2.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Import 3 files' }));
    expect(importedNames(onImport)).toEqual(['take1.mp4', 'take10.mp4', 'take2.mp4']);
  });

  it('reorders by dragging a row onto another', () => {
    setup();
    choose([file('a.mp4'), file('b.mp4'), file('c.mp4')]);
    const [first, , third] = rows();
    fireEvent.dragStart(third as HTMLElement);
    fireEvent.dragOver(first as HTMLElement);
    fireEvent.drop(first as HTMLElement);
    expect(names()).toEqual(['c.mp4', 'a.mp4', 'b.mp4']);
  });

  it('removes a staged file, and adds later files after the ones listed', () => {
    const { onImport } = setup();
    choose([file('b.mp4'), file('a.mp4')]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove a.mp4' }));
    expect(names()).toEqual(['b.mp4']);
    choose([file('z.mp4'), file('c.mp4')]);
    expect(names()).toEqual(['b.mp4', 'c.mp4', 'z.mp4']);
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(screen.queryByRole('list', { name: 'Files to import' })).toBeNull();
    expect(onImport).not.toHaveBeenCalled();
  });

  it('shows the running import with progress and per-file errors, and nothing to rearrange', () => {
    setup([
      { id: '0-a', name: 'a.mp4', status: 'done', progress: 1, error: null },
      { id: '1-b', name: 'b.mp4', status: 'uploading', progress: 0.42, error: null },
      { id: '2-c', name: 'c.mov', status: 'error', progress: 0, error: 'unreadable media' },
      { id: '3-d', name: 'd.mp4', status: 'queued', progress: 0, error: null },
    ]);
    expect(screen.getByText('Added')).toBeTruthy();
    expect(
      screen.getByRole('progressbar', { name: 'Uploading b.mp4' }).getAttribute('aria-valuenow'),
    ).toBe('42');
    expect(screen.getByRole('alert').textContent).toBe('unreadable media');
    expect(screen.getByText('Waiting')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Import/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /^Move/ })).toBeNull();
  });
});
```

- [ ] **Step 6: Run it and watch it fail**

Run: `npx vitest run client/src/components/Dropzone.test.tsx`
Expected: FAIL. The input has no `data-testid`, `Dropzone` takes `onFile`, and there is no list.

- [ ] **Step 7: `ImportList` and the multi-file `Dropzone`**

Create `client/src/components/ImportList.tsx`:

```tsx
import { useState, type DragEvent } from 'react';

import { cx } from '../cx';
import type { ImportItem } from '../importQueue';
import ui from '../styles/ui.module.css';
import styles from './ImportList.module.css';

interface Props {
  items: ImportItem[];
  /** Reorder a staged file. Absent once the import runs: the list is then read-only. */
  onMove?: (from: number, to: number) => void;
  onRemove?: (index: number) => void;
}

/** The files of an import, in the order they will land on the timeline. */
export function ImportList({ items, onMove, onRemove }: Props) {
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const editable = onMove !== undefined;

  const end = () => {
    setDragFrom(null);
    setOver(null);
  };
  const drop = (to: number) => (e: DragEvent) => {
    e.preventDefault();
    if (dragFrom !== null && dragFrom !== to) onMove?.(dragFrom, to);
    end();
  };

  return (
    <ol className={styles.list} aria-label="Files to import">
      {items.map((item, i) => (
        <li
          key={item.id}
          className={cx(
            styles.row,
            editable && styles.movable,
            over === i && dragFrom !== null && dragFrom !== i && styles.over,
            dragFrom === i && styles.dragging,
            item.status === 'error' && styles.failed,
          )}
          draggable={editable}
          onDragStart={
            editable
              ? (e) => {
                  // Firefox starts no drag without data.
                  e.dataTransfer?.setData('text/plain', item.name);
                  setDragFrom(i);
                }
              : undefined
          }
          onDragOver={
            editable
              ? (e) => {
                  e.preventDefault();
                  setOver(i);
                }
              : undefined
          }
          onDrop={editable ? drop(i) : undefined}
          onDragEnd={editable ? end : undefined}
        >
          {editable && (
            <span className={styles.handle} aria-hidden>
              ⋮⋮
            </span>
          )}
          <span className={styles.number}>{i + 1}</span>
          <span className={styles.name} title={item.name}>
            {item.name}
          </span>
          {editable ? (
            <span className={styles.actions}>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Move ${item.name} up`}
                disabled={i === 0}
                onClick={() => onMove?.(i, i - 1)}
              >
                ↑
              </button>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Move ${item.name} down`}
                disabled={i === items.length - 1}
                onClick={() => onMove?.(i, i + 1)}
              >
                ↓
              </button>
              <button
                type="button"
                className={cx(ui.iconButton, ui.ghost)}
                aria-label={`Remove ${item.name}`}
                onClick={() => onRemove?.(i)}
              >
                ✕
              </button>
            </span>
          ) : (
            <Status item={item} />
          )}
        </li>
      ))}
    </ol>
  );
}

function Status({ item }: { item: ImportItem }) {
  const percent = Math.round(item.progress * 100);
  switch (item.status) {
    case 'queued':
      return <span className={styles.status}>Waiting</span>;
    case 'uploading':
      return (
        <span className={styles.status}>
          <span
            className={styles.bar}
            role="progressbar"
            aria-label={`Uploading ${item.name}`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
          >
            <span className={styles.fill} style={{ width: `${percent}%` }} />
          </span>
          {percent}%
        </span>
      );
    case 'done':
      return <span className={cx(styles.status, styles.done)}>Added</span>;
    case 'error':
      return (
        <span className={cx(styles.status, ui.error)} role="alert">
          {item.error}
        </span>
      );
  }
}
```

Create `client/src/components/ImportList.module.css`:

```css
.list {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  width: min(560px, 100%);
  margin: 0;
  padding: 0;
  list-style: none;
}

.row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: 34px;
  padding: 2px var(--space-2);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  color: var(--text);
  font-size: var(--text-md);
}

.movable {
  cursor: grab;
}

.over {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.dragging {
  opacity: 0.5;
}

.failed {
  border-color: var(--danger);
}

.handle {
  color: var(--faint);
  letter-spacing: -2px;
}

.number {
  display: inline-grid;
  flex: none;
  place-items: center;
  min-width: 20px;
  height: 20px;
  border-radius: var(--radius-sm);
  background: var(--raised);
  color: var(--muted);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
}

.name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: var(--space-2);
  max-width: 50%;
  color: var(--muted);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}

.done {
  color: var(--ok);
}

.bar {
  position: relative;
  width: 80px;
  height: 4px;
  border-radius: 2px;
  background: var(--track);
  overflow: hidden;
}

.fill {
  position: absolute;
  inset: 0 auto 0 0;
  background: var(--accent);
  transition: width 0.15s;
}

.actions {
  display: inline-flex;
  flex: none;
  gap: 2px;
}
```

Replace `client/src/components/Dropzone.tsx` with:

```tsx
import { useRef, useState, type DragEvent, type ReactNode } from 'react';

import { cx } from '../cx';
import { byName, itemsFor, MEDIA_ACCEPT, moveItem, type ImportItem } from '../importQueue';
import ui from '../styles/ui.module.css';
import type { LibraryItem } from '../types';
import styles from './Home.module.css';
import { ImportList } from './ImportList';
import { Library } from './Library';

interface Props {
  /** Import these files as one project, in this order. */
  onImport: (files: File[]) => void;
  onLibraryClip: (item: LibraryItem) => void;
  busy: string | null;
  error: string | null;
  /** The import under way, file by file; empty when none is running. */
  imports: ImportItem[];
  /** Rendered between the how-to steps and the sample library. */
  children?: ReactNode;
}

export function Dropzone({ onImport, onLibraryClip, busy, error, imports, children }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  // Chosen but not yet imported, in the order they will be stitched.
  const [staged, setStaged] = useState<File[]>([]);

  const add = (list: FileList | File[] | null | undefined) => {
    const files = Array.from(list ?? []);
    if (files.length === 0) return;
    // One file with nothing staged opens straight away, as it always has.
    if (files.length === 1 && staged.length === 0) {
      onImport(files);
      return;
    }
    setStaged([...staged, ...byName(files)]);
  };

  const start = () => {
    const files = staged;
    setStaged([]);
    onImport(files);
  };

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setOver(false);
    add(e.dataTransfer.files);
  };

  return (
    <div className={styles.wrap}>
      <button
        type="button"
        className={cx(styles.dropzone, over && styles.over, busy && styles.busy)}
        onClick={() => input.current?.click()}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={onDrop}
        disabled={busy !== null}
      >
        <input
          ref={input}
          type="file"
          accept={MEDIA_ACCEPT}
          multiple
          hidden
          data-testid="media-input"
          onChange={(e) => {
            add(e.target.files);
            e.target.value = '';
          }}
        />
        {busy ? (
          <>
            <span className={ui.spinner} aria-hidden />
            <strong>{busy}</strong>
            <span className={ui.muted}>whisper.cpp is listening, hold on…</span>
          </>
        ) : staged.length > 0 ? (
          <>
            <strong>Drop more files</strong>
            <span className={ui.muted}>or click to add them; drag the list to set their order</span>
          </>
        ) : (
          <>
            <strong>Drop audio or video files</strong>
            <span className={ui.muted}>
              mp3, wav, m4a, mp4, mov, or click to browse. Several make one project, in order.
            </span>
          </>
        )}
      </button>
      {error && <p className={ui.error}>{error}</p>}
      {imports.length > 0 ? (
        <ImportList items={imports} />
      ) : (
        staged.length > 0 && (
          <div className={styles.staged}>
            <ImportList
              items={itemsFor(staged)}
              onMove={(from, to) => setStaged(moveItem(staged, from, to))}
              onRemove={(index) => setStaged(staged.filter((_, k) => k !== index))}
            />
            <div className={styles.stagedActions}>
              <button
                type="button"
                className={cx(ui.button, ui.ghost)}
                onClick={() => setStaged([])}
              >
                Clear
              </button>
              <button
                type="button"
                className={cx(ui.button, ui.primary)}
                onClick={start}
                disabled={busy !== null}
              >
                Import {staged.length === 1 ? '1 file' : `${staged.length} files`}
              </button>
            </div>
          </div>
        )
      )}
      <ol className={styles.how}>
        <li>Transcribe with word timestamps</li>
        <li>Select words, press Delete to cut them</li>
        <li>Overdub a phrase in a cloned voice</li>
        <li>Export the stitched result</li>
      </ol>
      {children}
      <Library onOpen={onLibraryClip} disabled={busy !== null} />
    </div>
  );
}
```

Append to `client/src/components/Home.module.css`:

```css
.staged {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  width: min(560px, 100%);
}

.stagedActions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  width: 100%;
}
```

- [ ] **Step 8: Run it and watch it pass**

Run: `npx vitest run client/src/components/Dropzone.test.tsx`
Expected: PASS.

- [ ] **Step 9: Failing test for Insert → Add video…**

In `client/src/components/TopBar.test.tsx`:

1. Change the testing-library import to `import { fireEvent, render, screen } from '@testing-library/react';`.
2. In `controls()`, after `onAddMusic: vi.fn(),` add `onAddVideos: vi.fn(),` and after `onSplit: vi.fn(),` add `addingVideos: false,`.
3. Change `setup` to take overrides:

   ```tsx
   function setup(exportState: ExportState, headSeq: number, overrides: Partial<EditorControls> = {}) {
     const editor = { ...controls(exportState, headSeq), ...overrides };
   ```

   (the rest of `setup` unchanged).

4. Append:

   ```tsx
   describe('Insert → Add video…', () => {
     it('opens a multi-file picker and hands over every chosen file', async () => {
       const user = userEvent.setup();
       const editor = setup({ status: 'idle' }, 1);
       const input = screen.getByTestId('add-video-input') as HTMLInputElement;
       expect(input.multiple).toBe(true);
       const click = vi.spyOn(input, 'click').mockImplementation(() => undefined);
       await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
       await user.click(await screen.findByRole('menuitem', { name: /Add video/ }));
       expect(click).toHaveBeenCalledOnce();
       const a = new File(['a'], 'a.mp4', { type: 'video/mp4' });
       const b = new File(['b'], 'b.mov', { type: 'video/quicktime' });
       fireEvent.change(input, { target: { files: [a, b] } });
       expect(editor.onAddVideos).toHaveBeenCalledWith([a, b]);
     });

     it('is disabled while videos are being added', async () => {
       const user = userEvent.setup();
       setup({ status: 'idle' }, 1, { addingVideos: true });
       await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
       const item = await screen.findByRole('menuitem', { name: /Add video/ });
       expect(item.getAttribute('aria-disabled')).toBe('true');
     });
   });
   ```

- [ ] **Step 10: Run it and watch it fail**

Run: `npx vitest run client/src/components/TopBar.test.tsx`
Expected: FAIL. There is no `add-video-input` and no **Add video…** item.

- [ ] **Step 11: The menu item**

In `client/src/components/TopBar.tsx`:

1. `import { useEffect, useState } from 'react';` → `import { useEffect, useRef, useState } from 'react';`, and add `import { MEDIA_ACCEPT } from '../importQueue';` after the `formatTime` import.
2. In `EditorControls`, after `onAddMusic: () => void;` add:

   ```ts
     /** Append these files to the end of the main track. */
     onAddVideos: (files: File[]) => void;
     /** An Add video… is still uploading; another waits for it. */
     addingVideos: boolean;
   ```

3. In `EditorTools`, after `const off = editor.readOnly;` add `const videoInput = useRef<HTMLInputElement>(null);`.
4. In the Insert menu's `DropdownMenu.Content`, before the `Title card` item, add:

   ```tsx
               <DropdownMenu.Item
                 className={styles.item}
                 disabled={editor.addingVideos}
                 onSelect={() => videoInput.current?.click()}
               >
                 Add video…
                 <span className={styles.hint}>appended at the end</span>
               </DropdownMenu.Item>
               <DropdownMenu.Separator className={styles.separator} />
   ```

5. Directly after the Insert menu's closing `</DropdownMenu.Root>`, add the input. It sits outside the portal, which unmounts when the menu closes:

   ```tsx
   <input
     ref={videoInput}
     type="file"
     accept={MEDIA_ACCEPT}
     multiple
     hidden
     data-testid="add-video-input"
     onChange={(e) => {
       const files = Array.from(e.target.files ?? []);
       e.target.value = '';
       if (files.length > 0) editor.onAddVideos(files);
     }}
   />
   ```

- [ ] **Step 12: Run it and watch it pass**

Run: `npx vitest run client/src/components/TopBar.test.tsx`
Expected: PASS.

- [ ] **Step 13: Wire both imports into the App**

In `client/src/App.tsx`:

1. Add `addSource,` as the first name in the `./api` import. Add these imports (keep the import block sorted as it is):

   ```ts
   import { ImportList } from './components/ImportList';
   import { byName, importFailures, runImport, type ImportItem } from './importQueue';
   import { sourceViewsOf } from './sources';
   ```

   and add `SourceView` to the `./types` import list.

2. After `const [speakers, setSpeakers] = useState<(number | null)[] | null>(null);` add:

   ```ts
   // The project's files as the server lists them: names, urls, transcript status.
   const [sourceViews, setSourceViews] = useState<SourceView[]>([]);
   // The home screen's import, file by file, while it runs.
   const [imports, setImports] = useState<ImportItem[]>([]);
   // Insert → Add video…, file by file. Failed files stay until dismissed.
   const [uploads, setUploads] = useState<ImportItem[]>([]);
   ```

3. In `goHome`, after `setAudioDialog(null);` add:

   ```ts
   setSourceViews([]);
   setUploads([]);
   ```

4. In `load`, replace:

   ```ts
   const [words, { doc }] = await Promise.all([
     transcribeMedia(summary.id),
     fetchProject(summary.id),
   ]);
   dispatch({ type: 'load', words, duration: summary.media.duration, media: summary.media.id });
   dispatch({ type: 'sync', doc });
   confirmed.current = doc;
   setProject(summary);
   ```

   with:

   ```ts
   const [words, fetched] = await Promise.all([
     transcribeMedia(summary.id),
     fetchProject(summary.id),
   ]);
   dispatch({ type: 'load', words, duration: summary.media.duration, media: summary.media.id });
   dispatch({ type: 'sync', doc: fetched.doc });
   confirmed.current = fetched.doc;
   setSourceViews(sourceViewsOf(fetched.project));
   setProject(summary);
   ```

5. Replace `onFile` with:

   ```ts
   // Several files make one project: the first creates it, the rest are
   // appended in the order chosen. Failures are reported once it opens.
   const onImport = useCallback(
     async (files: File[]) => {
       setLoadError(null);
       setBusy(
         files.length === 1
           ? `Uploading ${files[0]?.name ?? ''}`
           : `Importing ${files.length} files`,
       );
       const result = await runImport(
         files,
         { create: uploadMedia, append: addSource },
         setImports,
       );
       const failed = importFailures(result.items);
       setImports([]);
       const created = result.project;
       if (!created) {
         setBusy(null);
         setLoadError(failed ?? 'Nothing was imported.');
         return;
       }
       await load(`Opening ${created.title}`, () => Promise.resolve(created));
       if (failed) setLoadError(failed);
     },
     [load],
   );
   ```

6. After `onUploadAsset`, add:

   ```ts
   // Insert → Add video…: append at the end, one file at a time. The timeline
   // grows as each upload lands; the fold (broadcast, or the refetch below) confirms it.
   const onAddVideos = useCallback(
     async (files: File[]) => {
       if (!projectId || !canEdit || files.length === 0) return;
       const append = async (id: string, file: File, onProgress: (f: number) => void) => {
         const view = await addSource(id, file, onProgress);
         dispatch({
           type: 'addSource',
           media: view.mediaId,
           offset: view.offset,
           duration: view.duration,
         });
         setSourceViews((list) =>
           [...list.filter((v) => v.index !== view.index), view].sort((a, b) => a.index - b.index),
         );
         return view;
       };
       const result = await runImport(byName(files), { append }, setUploads, projectId);
       setUploads(result.items.filter((item) => item.status === 'error'));
       try {
         const fetched = await fetchProject(projectId);
         setSourceViews(sourceViewsOf(fetched.project));
         // Only when nothing of ours is in flight: an older fold must not hide an optimistic edit.
         if (queue.current?.pending === 0) settle(fetched.doc);
       } catch (err) {
         setLoadError(err instanceof Error ? err.message : String(err));
       }
     },
     [projectId, canEdit, settle],
   );
   ```

7. In `controls`, after `onAddMusic: () => setAudioDialog({ range: selected }),` add:

   ```ts
         onAddVideos: (files: File[]) => void onAddVideos(files),
         addingVideos: uploads.some((u) => u.status === 'queued' || u.status === 'uploading'),
   ```

8. Replace the `<Dropzone … />` opening tag with:

   ```tsx
             <Dropzone
               onImport={(files) => void onImport(files)}
               onLibraryClip={onLibraryClip}
               busy={busy}
               error={loadError}
               imports={imports}
             >
   ```

9. In the viewer section, before `<Player`, add:

   ```tsx
   {
     uploads.length > 0 && (
       <div className={styles.uploads}>
         <ImportList items={uploads} />
         {uploads.every((u) => u.status === 'error') && (
           <button type="button" className={cx(ui.button, ui.ghost)} onClick={() => setUploads([])}>
             Dismiss
           </button>
         )}
       </div>
     );
   }
   ```

In `client/src/App.module.css`, after `.viewer`, add:

```css
/* Insert → Add video… progress, above the picture while it runs. */
.uploads {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}
```

- [ ] **Step 14: Verify**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npx prettier --write client/src
npx vitest run && npx tsc -p client --noEmit && npm run lint && npm run build -w client
```

Expected: all pass. The full live check is Task 7's Step 12. It needs Task 3's `/sources` route, which has landed by now, so run the home-screen half here (restart the Rust server if it predates Task 3): drop two files, confirm the list is sorted and reorderable, import, and see per-file progress.

- [ ] **Step 15: Commit**

```bash
git add client/src
git commit -m "client: import several files as one project, and Insert → Add video appends more" -m "The home dropzone stages several files sorted by name, reorderable by drag or arrows, and imports them one at a time with per-file progress and errors: the first creates the project, the rest are appended. Insert → Add video… runs the same queue on the open project.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 7: Client — playback across files, source badges and dividers, transcribing blocks

After this task the preview plays straight through every file, in output order, in a frame of the canvas's shape: the first file with a picture, as the export picks it. Each clip on the Clips lane carries its video number, a divider marks where one file meets another, and a video still being transcribed shows "Transcribing video N…" in its clips until its words arrive.

**Files:**

- Create: `client/src/stitchedMedia.ts`, `client/src/stitchedMedia.test.ts`, `client/src/test/fakeVideo.ts`
- Modify: `client/src/usePlayback.ts` + `client/src/usePlayback.test.ts`, `client/src/components/Player.tsx` + `client/src/components/Player.module.css`, `client/src/components/Timeline.tsx` + `client/src/components/Timeline.module.css` + `client/src/components/Timeline.test.tsx`, `client/src/components/Transcript.tsx` + `client/src/components/Transcript.module.css` + `client/src/components/Transcript.test.tsx`, `client/src/App.tsx`

**Interfaces:**

- Consumes: Task 2's `locate`, `stitchedDuration`, `isJoin`, `SourceView`, `playableSources`, `unlistedSources`, `isTranscribing`, `sourceViewsOf`, `transcribeProject`, the `setWords` action, `EditorState.sources`; Tasks 3 and 4's `GET /api/projects/{id}` `project.sources` and `POST /transcribe` `{ words, sources }`; Task 6's `sourceViews` state in App; existing `usePlayback`, `playStep`, `timelineSegments`, `orderedPieces`.
- Produces:

  ```ts
  // usePlayback.ts
  export interface PlaybackMedia {
    currentTime: number;
    readonly paused: boolean;
    readonly ended: boolean;
    play(): Promise<void>;
    pause(): void;
    addEventListener(type: string, listener: () => void): void;
    removeEventListener(type: string, listener: () => void): void;
  }
  export function usePlayback(
    mediaRef: RefObject<PlaybackMedia | null>, // was RefObject<HTMLMediaElement | null>
    words: Word[],
    edits: Edit[],
    duration: number,
    src: string | undefined,
    ordered: Range[],
    segments: Segment[],
  ): Playback;
  // stitchedMedia.ts
  export interface PlayableSource {
    offset: number;
    duration: number;
    url: string;
    kind?: MediaKind;
  }
  export class StitchedMedia extends EventTarget implements PlaybackMedia {
    aspect: number | null; // first file with a picture: width / height; 'aspect' event on change
    readonly attach: (el: HTMLVideoElement | null) => void; // a stable callback ref
    setSources(list: PlayableSource[]): void;
    get preloading(): string | null;
  }
  export function useStitchedMedia(sources: PlayableSource[]): StitchedMedia;
  // Player props: `media` and `mediaRef` are replaced by `sources: SourceView[]` and `stitched: StitchedMedia`
  // Timeline props gain `sources: SourceView[]`; Transcript props gain `sources: SourceView[]`
  ```

  DOM hooks: a clip's badge is `[data-source="N"]` inside its `[data-clip]` button; each divider is `data-testid="source-join"`; a transcribing block is `role="status"` inside its clip's `<section data-clip-start>`.

- [ ] **Step 1: A fake video, and failing tests for the stitched clock and for playback across a join**

Create `client/src/test/fakeVideo.ts`:

```ts
import { vi } from 'vitest';

/**
 * Just enough of a <video> for the stitched clock: a src that loads paused at
 * 0 (no pause event, as in the HTML load algorithm), a settable clock,
 * play/pause and their events, and `loaded()` for a file's metadata arriving.
 */
export class FakeVideo extends EventTarget {
  private file = '';
  currentTime = 0;
  paused = true;
  ended = false;
  videoWidth = 0;
  videoHeight = 0;
  get src(): string {
    return this.file;
  }
  set src(url: string) {
    this.file = url;
    this.paused = true;
    this.ended = false;
    this.currentTime = 0;
  }
  play = vi.fn(() => {
    this.paused = false;
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  });
  pause = vi.fn(() => {
    if (this.paused) return;
    this.paused = true;
    this.dispatchEvent(new Event('pause'));
  });
  loaded(width = 0, height = 0) {
    this.videoWidth = width;
    this.videoHeight = height;
    this.dispatchEvent(new Event('loadedmetadata'));
  }
}
```

Create `client/src/stitchedMedia.test.ts`:

```ts
// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

import { StitchedMedia, type PlayableSource } from './stitchedMedia';
import { FakeVideo } from './test/fakeVideo';

const three: PlayableSource[] = [
  { offset: 0, duration: 10, url: '/a' },
  { offset: 10, duration: 5, url: '/b' },
  { offset: 15, duration: 2.5, url: '/c' },
];

function setup(list = three) {
  const media = new StitchedMedia();
  media.setSources(list);
  const video = new FakeVideo();
  media.attach(video as unknown as HTMLVideoElement);
  const events: string[] = [];
  for (const type of ['play', 'pause', 'seeked', 'ended'])
    media.addEventListener(type, () => events.push(type));
  return { media, video, events };
}

describe('StitchedMedia', () => {
  it('reads and seeks stitched time through the file that holds it', () => {
    const { media, video } = setup();
    expect(video.src).toBe('/a');
    video.currentTime = 4;
    expect(media.currentTime).toBe(4);
    media.currentTime = 6.5;
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(6.5);
  });

  it('swaps files when a seek lands in another source, holding the target until it loads', () => {
    const { media, video } = setup();
    media.currentTime = 12;
    expect(video.src).toBe('/b');
    expect(media.currentTime).toBe(12);
    video.loaded();
    expect(video.currentTime).toBe(2);
    expect(media.currentTime).toBe(12);
    // A join belongs to the later file; the exact end is the last file at its length.
    media.currentTime = 15;
    expect(video.src).toBe('/c');
    video.loaded();
    expect(video.currentTime).toBe(0);
    media.currentTime = 17.5;
    expect(video.currentTime).toBe(2.5);
  });

  it('keeps playing across a swap without telling the hook it paused', async () => {
    const { media, video, events } = setup();
    await media.play();
    expect(events).toEqual(['play']);
    media.currentTime = 10;
    // Whatever the element says while the new file loads is held back.
    video.dispatchEvent(new Event('pause'));
    video.dispatchEvent(new Event('timeupdate'));
    expect(media.paused).toBe(false);
    expect(events).toEqual(['play']);
    video.loaded();
    expect(video.play).toHaveBeenCalledTimes(2);
    expect(events).toEqual(['play', 'seeked', 'play']);
  });

  it('reports a play or pause made during a swap at once, and honours the last one', async () => {
    const { media, video, events } = setup();
    media.currentTime = 12;
    expect(media.paused).toBe(true);
    await media.play();
    expect(media.paused).toBe(false);
    expect(video.play).not.toHaveBeenCalled();
    media.pause();
    expect(events).toEqual(['play', 'pause']);
    video.loaded();
    expect(video.play).not.toHaveBeenCalled();
    expect(video.currentTime).toBe(2);
  });

  it('forwards a file ending, but is ended only at the last file', () => {
    const { media, video, events } = setup();
    video.ended = true;
    video.dispatchEvent(new Event('ended'));
    expect(events).toEqual(['ended']);
    expect(media.ended).toBe(false);
    media.currentTime = 16;
    video.loaded();
    video.ended = true;
    expect(media.ended).toBe(true);
  });

  it('takes the canvas from the first file with a picture, and keeps it', () => {
    const { media, video } = setup();
    const onAspect = vi.fn();
    media.addEventListener('aspect', onAspect);
    video.loaded(1920, 1080);
    expect(media.aspect).toBeCloseTo(16 / 9);
    media.currentTime = 12;
    video.loaded(640, 480);
    expect(media.aspect).toBeCloseTo(16 / 9);
    expect(onAspect).toHaveBeenCalledOnce();
  });

  it('skips a first file with no picture, as the export does', () => {
    const { media, video } = setup([
      { offset: 0, duration: 10, url: '/a', kind: 'audio' },
      { offset: 10, duration: 5, url: '/b', kind: 'video' },
    ]);
    video.loaded(0, 0);
    // The second file's shape is not known yet: no canvas, so the Player shows 16:9.
    expect(media.aspect).toBeNull();
    media.currentTime = 12;
    video.loaded(640, 480);
    expect(media.aspect).toBeCloseTo(4 / 3);
  });

  it('has no canvas while no file shows a picture', () => {
    const { media, video } = setup(three.slice(0, 1));
    video.loaded(0, 0);
    expect(media.aspect).toBeNull();
  });

  it('warms the next file', () => {
    const { media, video } = setup();
    expect(media.preloading).toBe('/b');
    media.currentTime = 11;
    video.loaded();
    expect(media.preloading).toBe('/c');
  });

  it('keeps the loaded file when a source is appended', () => {
    const { media, video } = setup(three.slice(0, 1));
    video.currentTime = 3;
    expect(media.preloading).toBeNull();
    media.setSources(three);
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(3);
    expect(media.preloading).toBe('/b');
  });

  it('does nothing outside the stitched timeline', () => {
    const { media, video } = setup();
    media.currentTime = 99;
    expect(video.src).toBe('/a');
    expect(video.currentTime).toBe(0);
  });
});
```

Append to `client/src/usePlayback.test.ts`, and add these imports at the top:

```ts
import { StitchedMedia } from './stitchedMedia';
import { FakeVideo } from './test/fakeVideo';
```

```ts
describe('usePlayback across two files', () => {
  // Two 5 s files end to end. The fold keeps the join (5) as a split.
  const sources = [
    { offset: 0, duration: 5, url: '/a.mp4' },
    { offset: 5, duration: 5, url: '/b.mp4' },
  ];

  let frames: FrameRequestCallback[] = [];
  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => undefined);
  });
  afterEach(() => vi.unstubAllGlobals());

  function setup(order: number[]) {
    const ordered = orderedPieces(10, [], [5], order);
    const segments = timelineSegments(10, [], [5], order);
    const video = new FakeVideo();
    const media = new StitchedMedia();
    media.setSources(sources);
    media.attach(video as unknown as HTMLVideoElement);
    const ref = { current: media };
    const hook = renderHook(() => usePlayback(ref, [], [], 10, '/a.mp4', ordered, segments));
    return { video, hook };
  }

  /** Run the playhead watchdog once, with the loaded file's clock at `local`. */
  function frameAt(video: FakeVideo, local: number) {
    video.currentTime = local;
    const pending = frames;
    frames = [];
    act(() => pending.forEach((cb) => cb(0)));
  }

  it('plays from the first file into the second at the join, then stops at the end', () => {
    const { video, hook } = setup([]);
    act(() => hook.result.current.toggle());
    expect(hook.result.current.playing).toBe(true);
    frameAt(video, 3);
    expect(hook.result.current.outputTime).toBeCloseTo(3);
    // The first file reaches its end: the next piece is the second file.
    frameAt(video, 5);
    expect(video.src).toBe('/b.mp4');
    expect(hook.result.current.currentTime).toBeCloseTo(5);
    expect(hook.result.current.playing).toBe(true);
    act(() => video.loaded());
    expect(video.play).toHaveBeenCalledTimes(2);
    frameAt(video, 2);
    expect(hook.result.current.currentTime).toBeCloseTo(7);
    expect(hook.result.current.outputTime).toBeCloseTo(7);
    frameAt(video, 5);
    expect(hook.result.current.atEnd).toBe(true);
    expect(hook.result.current.playing).toBe(false);
    expect(hook.result.current.outputTime).toBeCloseTo(10);
  });

  it("plays a reordered pair across files: the second file first, then the first, via the file's own end", () => {
    const { video, hook } = setup([5, 0]);
    act(() => hook.result.current.toggle());
    // Play starts at the first output piece, which is in the second file.
    expect(video.src).toBe('/b.mp4');
    expect(hook.result.current.playing).toBe(true);
    expect(hook.result.current.outputTime).toBeCloseTo(0);
    act(() => video.loaded());
    frameAt(video, 4);
    expect(hook.result.current.outputTime).toBeCloseTo(4);
    // The file runs out before the watchdog sees the piece end.
    act(() => {
      video.currentTime = 5;
      video.ended = true;
      video.dispatchEvent(new Event('ended'));
    });
    expect(video.src).toBe('/a.mp4');
    act(() => video.loaded());
    frameAt(video, 1);
    expect(hook.result.current.currentTime).toBeCloseTo(1);
    expect(hook.result.current.outputTime).toBeCloseTo(6);
  });

  it('seeks by output time into the other file', () => {
    const { video, hook } = setup([5, 0]);
    // Output 2 s is 2 s into the first output piece: source 7, in the second file.
    act(() => hook.result.current.seekOutput(2));
    expect(video.src).toBe('/b.mp4');
    expect(hook.result.current.currentTime).toBeCloseTo(7);
    expect(hook.result.current.outputTime).toBeCloseTo(2);
    act(() => video.loaded());
    expect(video.currentTime).toBe(2);
    // And back: output 7 s is source 2, in the first file.
    act(() => hook.result.current.seekOutput(7));
    expect(video.src).toBe('/a.mp4');
    expect(hook.result.current.outputTime).toBeCloseTo(7);
  });
});
```

- [ ] **Step 2: Run them and watch them fail**

Run: `npx vitest run client/src/stitchedMedia.test.ts client/src/usePlayback.test.ts`
Expected: FAIL. `./stitchedMedia` does not exist.

- [ ] **Step 3: The stitched clock, and `PlaybackMedia` for the hook**

In `client/src/usePlayback.ts`, above `export interface Playback`, add:

```ts
/**
 * What playback drives: a media element, or a `StitchedMedia` clock over
 * several files. Every time is in stitched seconds.
 */
export interface PlaybackMedia {
  currentTime: number;
  readonly paused: boolean;
  readonly ended: boolean;
  play(): Promise<void>;
  pause(): void;
  addEventListener(type: string, listener: () => void): void;
  removeEventListener(type: string, listener: () => void): void;
}
```

and change the first parameter of `usePlayback` from `mediaRef: RefObject<HTMLMediaElement | null>,` to `mediaRef: RefObject<PlaybackMedia | null>,`. Nothing else in the hook changes.

Create `client/src/stitchedMedia.ts`:

```ts
// A clock over the main track's files, played through one <video>. The
// playback hook reads and sets stitched seconds; this swaps the element's
// file when that time moves into another source, and holds the element's own
// events back while the new file loads, so the hook's piece tracking
// (`playIndex`, `ended`) never sees the swap.

import { useEffect, useState } from 'react';

import { locate } from './editlist';
import type { MediaKind } from './types';
import type { PlaybackMedia } from './usePlayback';

export interface PlayableSource {
  offset: number;
  duration: number;
  url: string;
  /** An audio-only file never supplies the canvas. `SourceView` carries it. */
  kind?: MediaKind;
}

/** Element events the playback hook listens for. */
const FORWARDED = ['play', 'pause', 'ended', 'timeupdate', 'seeked'] as const;

export class StitchedMedia extends EventTarget implements PlaybackMedia {
  private el: HTMLVideoElement | null = null;
  private list: PlayableSource[] = [];
  /** The source whose file the element holds (or is loading). */
  private index = 0;
  /** The url last given to the element. */
  private loadedUrl: string | null = null;
  /** The stitched instant a swap is heading for; null when no file is loading. */
  private pending: number | null = null;
  /** Play once the swap lands. */
  private resume = false;
  private preloader: HTMLVideoElement | null = null;
  /** Each file's width over height once its metadata is in (0: no picture), by url. */
  private shapes = new Map<string, number>();
  /**
   * The canvas: the first file with a picture, as the export picks it. Null
   * until that is known, and when no file has one; the Player then uses 16:9.
   */
  aspect: number | null = null;

  private readonly forward = (e: Event) => {
    if (this.pending !== null) return;
    this.dispatchEvent(new Event(e.type));
  };

  private readonly landed = () => {
    const el = this.el;
    if (!el) return;
    const url = this.list[this.index]?.url;
    if (url) this.learn(el, url);
    if (this.pending === null) return;
    const hit = locate(this.list, this.pending);
    const resume = this.resume;
    this.pending = null;
    this.resume = false;
    if (hit && hit.index === this.index) el.currentTime = hit.local;
    this.dispatchEvent(new Event('seeked'));
    if (resume)
      void el.play().catch(() => {
        // Autoplay refused: say so, so the transport shows paused.
        this.dispatchEvent(new Event('pause'));
      });
  };

  /** The callback ref for the Player's <video>. Stable for the clock's life. */
  readonly attach = (el: HTMLVideoElement | null): void => {
    if (el === this.el) return;
    if (this.el) {
      for (const type of FORWARDED) this.el.removeEventListener(type, this.forward);
      this.el.removeEventListener('loadedmetadata', this.landed);
    }
    this.el = el;
    this.loadedUrl = null;
    this.pending = null;
    this.resume = false;
    if (!el) return;
    for (const type of FORWARDED) el.addEventListener(type, this.forward);
    el.addEventListener('loadedmetadata', this.landed);
    const source = this.list[this.index];
    if (source) this.load(source.url);
  };

  /** The main track's files in stitched order. The loaded file stays if it is still there. */
  setSources(list: PlayableSource[]): void {
    this.list = list;
    if (this.index >= list.length) this.index = 0;
    const source = list[this.index];
    if (source && source.url !== this.loadedUrl) this.load(source.url);
    this.preloadNext();
    this.updateAspect();
  }

  /** The url being warmed for the next swap, if any. */
  get preloading(): string | null {
    return this.preloader?.getAttribute('src') ?? null;
  }

  get currentTime(): number {
    if (this.pending !== null) return this.pending;
    return (this.list[this.index]?.offset ?? 0) + (this.el?.currentTime ?? 0);
  }

  set currentTime(t: number) {
    const hit = locate(this.list, t);
    const el = this.el;
    if (!hit || !el) return;
    if (hit.index === this.index) {
      // Mid-swap to this file: just aim the landing elsewhere.
      if (this.pending !== null) this.pending = t;
      else el.currentTime = hit.local;
      return;
    }
    // A swap started while playing carries on playing.
    if (this.pending === null && !el.paused) this.resume = true;
    this.index = hit.index;
    this.pending = t;
    this.load((this.list[hit.index] as PlayableSource).url);
    this.preloadNext();
  }

  get paused(): boolean {
    if (this.pending !== null) return !this.resume;
    return this.el?.paused ?? true;
  }

  get ended(): boolean {
    return (
      this.pending === null && this.index === this.list.length - 1 && (this.el?.ended ?? false)
    );
  }

  play(): Promise<void> {
    if (!this.el) return Promise.resolve();
    if (this.pending !== null) {
      if (!this.resume) {
        this.resume = true;
        this.dispatchEvent(new Event('play'));
      }
      return Promise.resolve();
    }
    return this.el.play();
  }

  pause(): void {
    if (this.pending !== null) {
      if (this.resume) {
        this.resume = false;
        this.dispatchEvent(new Event('pause'));
      }
      return;
    }
    this.el?.pause();
  }

  private load(url: string) {
    if (!this.el) return;
    this.loadedUrl = url;
    this.el.src = url;
  }

  /** Record `url`'s picture shape from an element whose metadata just loaded. */
  private learn(el: HTMLVideoElement, url: string) {
    const shape = el.videoWidth > 0 && el.videoHeight > 0 ? el.videoWidth / el.videoHeight : 0;
    this.shapes.set(url, shape);
    this.updateAspect();
  }

  /** The first file with a picture, in stitched order, once every file before it is known. */
  private updateAspect() {
    let next: number | null = null;
    for (const s of this.list) {
      if (s.kind === 'audio') continue;
      const shape = this.shapes.get(s.url);
      // Not loaded yet: an earlier file may still turn out to have a picture.
      if (shape === undefined) break;
      if (shape > 0) {
        next = shape;
        break;
      }
    }
    if (next !== this.aspect) {
      this.aspect = next;
      this.dispatchEvent(new Event('aspect'));
    }
  }

  /** Point a detached, muted element at the next file so its swap starts warm. */
  private preloadNext() {
    const next = this.list[this.index + 1];
    if (!next || typeof document === 'undefined') return;
    if (!this.preloader) {
      const preloader = document.createElement('video');
      preloader.preload = 'auto';
      preloader.muted = true;
      // A warmed file's shape counts toward the canvas before it plays.
      preloader.addEventListener('loadedmetadata', () => {
        const url = preloader.getAttribute('src');
        if (url) this.learn(preloader, url);
      });
      this.preloader = preloader;
    }
    if (this.preloader.getAttribute('src') !== next.url)
      this.preloader.setAttribute('src', next.url);
  }
}

/** One clock per mounted editor, fed the current sources. */
export function useStitchedMedia(sources: PlayableSource[]): StitchedMedia {
  const [media] = useState(() => new StitchedMedia());
  useEffect(() => media.setSources(sources), [media, sources]);
  return media;
}
```

- [ ] **Step 4: Run them and watch them pass**

Run: `npx vitest run client/src/stitchedMedia.test.ts client/src/usePlayback.test.ts`
Expected: PASS, including the existing `FakeMedia` tests. `npx tsc -p client --noEmit` still fails at `App.tsx` and `Player.tsx` (they pass `HTMLVideoElement` refs). Steps 9–10 fix that.

- [ ] **Step 5: Failing tests for badges, joins and transcribing blocks**

In `client/src/components/Timeline.test.tsx`, add `SourceView` to the `../types` import, add `sources: [],` to the `props` object in `setup` (after `splits,`), and append:

```tsx
const video = (index: number, offset: number, duration = 5): SourceView => ({
  index,
  mediaId: `m${index}`,
  url: `/data/m${index}/source.mp4`,
  filename: `take${index + 1}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript: 'ready',
});

describe('Timeline sources', () => {
  const badges = () =>
    screen
      .getAllByRole('button', { name: /^Clip \d/ })
      .map((c) => c.querySelector('[data-source]')?.textContent);

  it('badges each clip with its video and marks where two videos meet', () => {
    // Split at 5 (the join) and reversed: output is video 2, then video 1.
    setup({ sources: [video(0, 0), video(1, 5)] });
    expect(badges()).toEqual(['2', '1']);
    const joins = screen.getAllByTestId('source-join');
    expect(joins).toHaveLength(1);
    expect(joins[0]?.style.left).toBe('50%');
    expect(
      screen
        .getAllByRole('button', { name: /^Clip \d/ })[0]
        ?.querySelector('[data-source]')
        ?.getAttribute('title'),
    ).toBe('Video 2 · take2.mp4');
  });

  it('draws joins only between neighbours from different videos', () => {
    const three = [2, 5];
    setup({
      sources: [video(0, 0), video(1, 5)],
      splits: three,
      ordered: orderedPieces(10, edits, three, []),
      segments: timelineSegments(10, edits, three, []),
    });
    expect(badges()).toEqual(['1', '1', '2']);
    const joins = screen.getAllByTestId('source-join');
    expect(joins).toHaveLength(1);
    expect(joins[0]?.style.left).toBe('50%');
  });

  it('shows neither for a single video', () => {
    setup({ sources: [video(0, 0, 10)] });
    expect(document.querySelector('[data-source]')).toBeNull();
    expect(screen.queryAllByTestId('source-join')).toHaveLength(0);
  });
});
```

In `client/src/components/Transcript.test.tsx`, change the imports and `setup` so overrides can be passed:

```tsx
// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { SourceView, Word } from '../types';
import { Transcript } from './Transcript';
```

```tsx
function setup(overrides: Partial<ComponentProps<typeof Transcript>> = {}) {
```

and inside the `<Transcript … />` element add `sources={[]}` before `tool="range"`, and `{...overrides}` as its last attribute. Then append:

```tsx
const video = (
  index: number,
  offset: number,
  duration: number,
  transcript: SourceView['transcript'],
): SourceView => ({
  index,
  mediaId: `m${index}`,
  url: `/m${index}`,
  filename: `take${index + 1}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript,
});

describe('Transcript with several videos', () => {
  // The three words fill video 1, [0, 3); video 2 is [3, 8) with no words yet.
  const twoClips = {
    ordered: [
      { start: 0, end: 3 },
      { start: 3, end: 8 },
    ],
    splits: [3],
  };

  it('greys out a video that is still transcribing, in its own clip', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'running')] });
    const block = screen.getByRole('status');
    expect(block.textContent).toBe('Transcribing video 2…');
    expect(block.closest('section')?.getAttribute('data-clip-start')).toBe('3');
  });

  it('says so when a transcript failed, and shows nothing once it is ready', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'error')] });
    expect(screen.getByRole('status').textContent).toBe('Video 2 could not be transcribed');
  });

  it('labels a join as where a video starts, not as a split Delete could join', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'ready')] });
    expect(screen.queryByRole('status')).toBeNull();
    const divider = screen.getByRole('button', { name: /^Video 2 · Clip 2/ });
    expect(divider.getAttribute('title')).toMatch(/video 2 starts/i);
  });
});
```

- [ ] **Step 6: Run them and watch them fail**

Run: `npx vitest run client/src/components/Timeline.test.tsx client/src/components/Transcript.test.tsx`
Expected: FAIL. No badges, joins or status blocks are rendered yet.

- [ ] **Step 7: Badges and joins on the Clips lane; transcribing blocks in the transcript**

`client/src/components/Timeline.tsx`:

1. Change `import { formatTime } from '../editlist';` to `import { formatTime, locate } from '../editlist';` and add `SourceView` to the `../types` import.
2. In `TimelineProps`, after `onCut`, add:

   ```ts
     /** The main track's files in stitched order; badges and joins show from two up. */
     sources: SourceView[];
   ```

3. After `const nameOf = …`, add:

   ```ts
   const multi = props.sources.length > 1;
   /** The file a clip plays. No piece spans a join: the fold keeps every join as a split. */
   const videoOf = (piece: Range) => props.sources[locate(props.sources, piece.start)?.index ?? 0];
   ```

4. In the Clips lane's `ordered.map`, after `const text = firstWords(words, piece);` add `const src = multi ? videoOf(piece) : undefined;`, and replace the button's children `<span>{text || '…'}</span>` with:

   ```tsx
   {
     src && (
       <span
         className={styles.sourceBadge}
         data-source={src.index + 1}
         title={`Video ${src.index + 1} · ${src.filename}`}
       >
         {src.index + 1}
       </span>
     );
   }
   <span className={styles.text}>{text || '…'}</span>;
   ```

5. Directly after the clips' `ordered.map(...)` (before `{dropAt !== null && …}`), add:

   ```tsx
   {
     multi &&
       ordered.map((piece, k) => {
         const prev = ordered[k - 1];
         if (!prev || videoOf(prev)?.index === videoOf(piece)?.index) return null;
         return (
           <span
             key={`join-${piece.start}`}
             data-testid="source-join"
             className={styles.sourceJoin}
             style={{ left: pct(spans[k]?.start ?? 0) }}
             aria-hidden
           />
         );
       });
   }
   ```

6. Replace the hover preview block at the end with:

   ```tsx
   {
     hover &&
       length > 0 &&
       (() => {
         const frameTime = outputToSource(hover.t, segments);
         // The sprite sheet is the first file's; past it there is no frame to show.
         const inFirst = frameTime < (props.sources[1]?.offset ?? Infinity);
         return (
           <ScrubPreview
             label={formatTime(hover.t)}
             frameTime={frameTime}
             x={hover.x}
             trackWidth={hover.width}
             thumbs={inFirst ? thumbs : null}
           />
         );
       })();
   }
   ```

`client/src/components/Timeline.module.css`: in `.clip`, add these three declarations after `text-align: left;`:

```css
display: flex;
align-items: center;
gap: 4px;
```

and append:

```css
.clip .text {
  flex: 1;
  min-width: 0;
}

/* The clip's video number, from two videos up. */
.clip .sourceBadge {
  display: grid;
  flex: none;
  place-items: center;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  border-radius: 3px;
  background: var(--raised);
  color: var(--muted);
  font-size: 10px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

/* Where one video meets another, in output order. */
.sourceJoin {
  position: absolute;
  top: -3px;
  bottom: -3px;
  z-index: 1;
  width: 2px;
  margin-left: -1px;
  border-radius: 1px;
  background: var(--text-strong);
  opacity: 0.6;
  pointer-events: none;
}
```

`client/src/components/Transcript.tsx`:

1. Add `isJoin` and `locate` to the `../editlist` import (sorted), and `SourceView` to the `../types` import.
2. In `Props`, after `onAudioClick`, add:

   ```ts
     /** The main track's files in stitched order. A file still transcribing shows as a greyed block in its clips. */
     sources: SourceView[];
   ```

   and add `sources,` to the destructured parameters after `onAudioClick,`.

3. Replace the `{clips.map((clip, k) => ( <section …> … </section> ))}` block with:

   ```tsx
   {
     clips.map((clip, k) => {
       const start = clip.piece.start;
       // Which file this clip plays, when there is more than one.
       const video = sources.length > 1 ? sources[locate(sources, start)?.index ?? 0] : undefined;
       const join = isJoin(start, sources);
       return (
         <section key={`clip-${start}`} className={styles.clip} data-clip-start={start}>
           {clips.length > 1 &&
             (() => {
               // A join is a split nobody can remove: it never shows "selected".
               const split = isSplit(start) && !join;
               const selected =
                 split && selectedClip !== null && Math.abs(selectedClip - start) < EPS;
               return (
                 <button
                   type="button"
                   className={cx(
                     styles.clipDivider,
                     (split || join) && styles.split,
                     selected && styles.dividerSelected,
                   )}
                   title={
                     join
                       ? `Video ${(video?.index ?? 0) + 1} starts here — click to jump here`
                       : split
                         ? 'Click to select · Delete joins it to the clip before'
                         : 'Clip boundary from a cut — click to jump here'
                   }
                   onClick={() => onClipClick(start)}
                 >
                   {video ? `Video ${video.index + 1} · ` : ''}Clip {k + 1} ·{' '}
                   {formatTime(clip.piece.end - clip.piece.start)}
                 </button>
               );
             })()}
           {video && video.transcript !== 'ready' && (
             <p
               role="status"
               className={cx(
                 styles.transcribing,
                 video.transcript === 'error' && styles.transcribeFailed,
               )}
             >
               {video.transcript === 'error'
                 ? `Video ${video.index + 1} could not be transcribed`
                 : `Transcribing video ${video.index + 1}…`}
             </p>
           )}
           {splitTurns(clip.tokens, speakers).map((turn) => {
             const first = turn.tokens[0];
             const key = first ? `turn-${tokenStart(first)}` : 'turn';
             return (
               <div
                 key={key}
                 className={cx(styles.turn, turnContains(turn, activeWord) && styles.speaking)}
                 style={speakerStyle(turn.speaker)}
               >
                 {turn.speaker !== null && (
                   <SpeakerTag
                     speaker={turn.speaker}
                     name={speakerLabel(turn.speaker, speakerNames)}
                     onRename={(name) => onRenameSpeaker(turn.speaker as number, name)}
                     readOnly={readOnly}
                   />
                 )}
                 <p className={styles.speech}>{turn.tokens.map(renderToken)}</p>
               </div>
             );
           })}
         </section>
       );
     });
   }
   ```

`client/src/components/Transcript.module.css`: after `.dividerSelected`, add:

```css
/* A video whose transcript has not arrived: its clip is usable, its words are not. */
.transcribing {
  margin: 0 0 var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px dashed var(--border);
  border-radius: var(--radius);
  background: var(--raised);
  color: var(--muted);
  font-size: var(--text-md);
  line-height: 1.5;
  animation: breathe 1.6s ease-in-out infinite;
}

.transcribeFailed {
  color: var(--danger);
  animation: none;
}

@keyframes breathe {
  50% {
    opacity: 0.55;
  }
}

@media (prefers-reduced-motion: reduce) {
  .transcribing {
    animation: none;
  }
}
```

- [ ] **Step 8: Run them and watch them pass**

Run: `npx vitest run client/src/components/Timeline.test.tsx client/src/components/Transcript.test.tsx`
Expected: PASS.

- [ ] **Step 9: The Player plays through the clock in a frame of the canvas's shape**

Replace `client/src/components/Player.tsx` with:

```tsx
import { useCallback, useMemo, useSyncExternalStore } from 'react';

import { cx } from '../cx';
import {
  captionsAt,
  cutRanges,
  formatTime,
  joins,
  locate,
  nearDipJoin,
  overdubs,
  stitchedDuration,
} from '../editlist';
import type { StitchedMedia } from '../stitchedMedia';
import { timelineLength, type Segment } from '../timeline';
import type { Asset, Edit, SourceView, Transition, Word } from '../types';
import type { Playback } from '../usePlayback';
import { Overlays } from './Overlays';
import styles from './Player.module.css';
import { Caption, TitleCard } from './TitleCard';

interface Props {
  /** The main track's files in stitched order, placed as the fold places them. */
  sources: SourceView[];
  /** The clock that plays them through one <video>; the element attaches to it. */
  stitched: StitchedMedia;
  edits: Edit[];
  assets: Asset[];
  words: Word[];
  playback: Playback;
  /** The edit in output time; the same layout the timeline draws. */
  segments: Segment[];
  /** The project default transition, for the dip preview. */
  transition: Transition;
}

export function Player({
  sources,
  stitched,
  edits,
  assets,
  words,
  playback,
  segments,
  transition,
}: Props) {
  // Dims the frame near a dipping join. Uses the reordered layout, so the
  // preview dims at the same joins the export fades.
  const joinList = useMemo(() => joins(segments, edits, transition), [segments, edits, transition]);
  const fading = nearDipJoin(playback.currentTime, segments, joinList);
  const outputLength = timelineLength(segments);
  const cutCount = cutRanges(edits).length;
  const overdubCount = overdubs(edits).length;

  // The canvas is the first file with a picture (16:9 until one is known);
  // every file is contained inside it, as the export letterboxes each one
  // onto that file's resolution.
  const subscribe = useCallback(
    (onChange: () => void) => {
      stitched.addEventListener('aspect', onChange);
      return () => stitched.removeEventListener('aspect', onChange);
    },
    [stitched],
  );
  const aspect = useSyncExternalStore(subscribe, () => stitched.aspect);
  // No picture anywhere: today's audio frame, as the export renders no video.
  const audioOnly = sources.length > 0 && sources.every((s) => s.kind === 'audio');
  const here = locate(sources, playback.currentTime);
  // An audio-only file further along shows black under its words.
  const dark = !audioOnly && here !== null && sources[here.index]?.kind === 'audio';
  const total = stitchedDuration(sources);

  return (
    <div className={styles.player}>
      <div
        className={cx(
          styles.frame,
          audioOnly && styles.audio,
          dark && styles.dark,
          fading && styles.fading,
        )}
        style={audioOnly ? undefined : { aspectRatio: aspect ?? 16 / 9 }}
      >
        <video
          ref={stitched.attach}
          preload="auto"
          playsInline
          onClick={playback.toggle}
          muted={playback.overdubbing !== null}
        />
        <Overlays edits={edits} assets={assets} words={words} playback={playback} />
        {(audioOnly || dark) && <div className={styles.audioBadge}>audio</div>}
        {playback.overdubbing && (
          <div className={styles.overdubBadge}>Overdub: “{playback.overdubbing.text}”</div>
        )}
        {captionsAt(playback.currentTime, edits).map((c, i) => (
          <Caption key={`${i}-${c.start}-${c.position}`} caption={c} />
        ))}
        {playback.titling && <TitleCard title={playback.titling} />}
      </div>

      <div className={styles.transport}>
        <button
          type="button"
          className={styles.play}
          onClick={playback.toggle}
          aria-label={playback.playing ? 'Pause' : 'Play'}
          title="Space"
        >
          {playback.playing ? <PauseIcon /> : <PlayIcon />}
        </button>
        <span className={styles.time}>
          {formatTime(playback.outputTime)}
          <span className={styles.total}> / {formatTime(outputLength)}</span>
        </span>
        <span className={styles.meta}>
          {sources.length > 1
            ? `${sources.length} videos · ${formatTime(total)}`
            : `source ${formatTime(total)}`}
          {cutCount > 0 && ` · ${cutCount} ${cutCount === 1 ? 'cut' : 'cuts'}`}
          {overdubCount > 0 && ` · ${overdubCount} ${overdubCount === 1 ? 'overdub' : 'overdubs'}`}
        </span>
      </div>
    </div>
  );
}

function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden>
      <path d="M7 5v14l12-7z" fill="currentColor" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden>
      <path d="M6 5h4v14H6zM14 5h4v14h-4z" fill="currentColor" />
    </svg>
  );
}
```

In `client/src/components/Player.module.css`, after the `.fading video` rule, add:

```css
/* A later audio-only file: black under its words, and the frame keeps its shape. */
.dark video {
  opacity: 0;
}

.dark .audioBadge {
  position: absolute;
}
```

- [ ] **Step 10: Wire the clock, polling and fresh words into the App**

In `client/src/App.tsx`:

1. Imports:
   - `import { EPS, orderedPieces, rangeForWords, titles } from './editlist';` becomes `import { EPS, isJoin, orderedPieces, rangeForWords, titles } from './editlist';`
   - `import { sourceViewsOf } from './sources';` becomes `import { isTranscribing, playableSources, sourceViewsOf, unlistedSources } from './sources';`
   - Add `transcribeProject,` to the `./api` import (keep it sorted).
   - Add `import { useStitchedMedia } from './stitchedMedia';`

2. Replace:

   ```ts
   const mediaRef = useRef<HTMLVideoElement>(null);
   const playback = usePlayback(
     mediaRef,
     editor.words,
     editor.edits,
     editor.duration,
     project?.media.url,
     ordered,
     segments,
   );
   ```

   with:

   ```ts
   // The fold's sources with their files: what the player plays, what the
   // timeline badges and the transcript greys out while it transcribes.
   const playable = useMemo(
     () => playableSources(editor.sources, sourceViews),
     [editor.sources, sourceViews],
   );
   // One <video> behind a stitched clock, so playback stays in stitched time.
   const stitched = useStitchedMedia(playable);
   const mediaRef = useMemo(() => ({ current: stitched }), [stitched]);
   const playback = usePlayback(
     mediaRef,
     editor.words,
     editor.edits,
     editor.duration,
     playable[0]?.url,
     ordered,
     segments,
   );
   ```

3. After the `sourceViews` state (Task 6), add:

   ```ts
   // How many ready sources the current words cover; one more finishing brings its words in.
   const [wordsReady, setWordsReady] = useState(0);
   ```

4. In `goHome`: replace `mediaRef.current?.pause();` with `stitched.pause();`, add `setWordsReady(0);` after `setSourceViews([]);`, and change its dependency list from `[]` to `[stitched]`.

5. In `load`, replace `setSourceViews(sourceViewsOf(fetched.project));` with:

   ```ts
   const views = sourceViewsOf(fetched.project);
   setSourceViews(views);
   setWordsReady(views.filter((v) => v.transcript === 'ready').length);
   ```

6. After the `onAddVideos` callback (Task 6), add:

   ```ts
   // While any file is still transcribing, poll the project's list (like the
   // export poll). Only the list is updated, so a poll never clears a selection.
   const waiting = sourceViews.some(isTranscribing);
   useEffect(() => {
     if (!projectId || !waiting) return;
     let cancelled = false;
     const timer = setInterval(() => {
       fetchProject(projectId)
         .then(({ project: fresh }) => {
           if (cancelled) return;
           const views = sourceViewsOf(fresh);
           setSourceViews(views);
           // `pending` means nothing has started it (the server restarted since
           // it was added). /transcribe starts it and answers without waiting.
           if (views.some((v) => v.transcript === 'pending'))
             void transcribeProject(projectId).catch(() => undefined);
         })
         .catch(() => {
           // Try again on the next tick.
         });
     }, 2000);
     return () => {
       cancelled = true;
       clearInterval(timer);
     };
   }, [projectId, waiting]);

   // A peer's AddSource names a file we have not listed: fetch the list once.
   const unlisted = unlistedSources(editor.sources, sourceViews).join(',');
   useEffect(() => {
     if (!projectId || !unlisted) return;
     let cancelled = false;
     fetchProject(projectId)
       .then(({ project: fresh }) => {
         if (!cancelled) setSourceViews(sourceViewsOf(fresh));
       })
       .catch(() => undefined);
     return () => {
       cancelled = true;
     };
   }, [projectId, unlisted]);

   // Another file's transcript is ready: fetch the stitched words again.
   const readyCount = sourceViews.filter((v) => v.transcript === 'ready').length;
   useEffect(() => {
     if (!projectId || readyCount <= wordsReady) return;
     let cancelled = false;
     transcribeProject(projectId)
       .then(({ words: list, sources: fresh }) => {
         if (cancelled) return;
         // The words cover exactly the sources this reply calls ready.
         if (fresh) {
           setSourceViews(fresh);
           setWordsReady(fresh.filter((v) => v.transcript === 'ready').length);
         } else {
           setWordsReady(readyCount);
         }
         dispatch({ type: 'setWords', words: list });
       })
       .catch((err: unknown) => {
         if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
       });
     return () => {
       cancelled = true;
     };
   }, [projectId, readyCount, wordsReady]);
   ```

7. The speakers effect: change its dependency list from `[projectId]` to `[projectId, words]`, so the labels follow the stitched words. (`words` is the `editor.words` already destructured above it.)

8. `onClipClick`: replace its first line with:

   ```ts
   // A join between two files is a boundary, never a split Delete can remove.
   const split =
     editor.splits.some((s) => Math.abs(s - start) < EPS) && !isJoin(start, editor.sources);
   setSelectedClip(split ? start : null);
   ```

   and change its dependency list to `[playback, editor.splits, editor.sources]`.

   Replace the `hasClipSelection` definition with:

   ```ts
   const hasClipSelection =
     selectedClip !== null &&
     editor.splits.some((s) => Math.abs(s - selectedClip) < EPS) &&
     !isJoin(selectedClip, editor.sources);
   ```

9. JSX:
   - `<Player media={project.media} mediaRef={mediaRef} …/>` → drop both props and add `sources={playable}` and `stitched={stitched}` in their place.
   - `<Transcript>`: add `sources={playable}` after `onAudioClick={openAudio}`.
   - `<Timeline>`: add `sources={playable}` after `onCut={…}`.

- [ ] **Step 11: Verify**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npx prettier --write client/src
npx vitest run && npx tsc -p client --noEmit && npm run lint && npm run build -w client
```

Expected: all pass.

- [ ] **Step 12: Live check**

This needs Tasks 1, 3 and 4, which have landed by now (restart the Rust server if it predates them). Make three clips: two from the sample at different resolutions, and one from the library.

```bash
mkdir -p /tmp/tns-clips
ffmpeg -y -loglevel error -ss 0 -t 12 -i samples/sample.mp4 -c:v libx264 -c:a aac /tmp/tns-clips/clip1.mp4
ffmpeg -y -loglevel error -ss 12 -t 10 -i samples/sample.mp4 -vf scale=640:360 -c:v libx264 -c:a aac /tmp/tns-clips/clip2.mp4
ffmpeg -y -loglevel error -ss 30 -t 15 -i samples/library/conversation-interview.mp4 -c:v libx264 -c:a aac /tmp/tns-clips/clip10.mp4
```

Start the app if it is not running (`export PATH="$HOME/.cargo/bin:$PATH"; npm run dev &`, wait for `listening on`), open `http://localhost:5174`, and with the browser tools:

1. **Home.** Upload all three (`mcp__claude-in-chrome__file_upload` on `[data-testid="media-input"]`). The list reads `clip1.mp4, clip2.mp4, clip10.mp4` (natural order). Drag `clip10.mp4` to the top and back, and use ↓ once, then restore the order. Click **Import 3 files**. Each row shows a progress bar and then **Added**, one file at a time.
2. **Editor.**
   - The transcript shows video 1's words.
   - Videos 2 and 3 show "Transcribing video 2…" and "Transcribing video 3…", greyed and pulsing, and each is replaced by words when it finishes, with no reload.
   - The Clips lane shows badges 1, 2 and 3 with a divider at each join.
   - Clicking a join's divider in the transcript seeks but never selects it, and Delete does nothing.
3. **Playback.**
   - Press Space at 0:00. Playback runs through video 1 into video 2 without stopping: the transport stays on Pause and the output time keeps counting.
   - Video 2 (640×360) sits letterboxed inside a frame with video 1's aspect.
   - Drag the video 2 clip before video 1 on the timeline and play from the start: video 2 plays first, then video 1.
   - Seek by clicking the ruler into each clip. The picture and time are right each time.
4. **Range and undo.**
   - Make a Range cut across the join between videos 1 and 2 and play over it. The cut is skipped and playback carries on in video 2.
   - Undo, then play again: it plays through the join.
5. **Add video.** Insert ▾ → **Add video…**, choose `clip1.mp4` again. A progress row shows above the picture, a fourth clip with badge 4 appears at the end, and "Transcribing video 4…" shows until its words arrive. The upload gets a new random media id (there is no dedup by content), so the same file is transcribed again.
6. **Collaboration.** Open the project in a second tab and add a video from the first. The second tab's timeline grows and its badges and player pick up the new file without a reload.
7. **Errors.** From the home screen, import a text file renamed `bad.mp4` together with a real clip. The bad row shows the server's error in red, the project opens with the good clip, and the banner reads "Could not import bad.mp4 (…)".
8. Screenshot the editor with three badges and a transcribing block, in dark and light themes.

If the browser tools are unavailable, say so plainly in the report and do not claim these checks.

- [ ] **Step 13: Commit**

```bash
git add client/src
git commit -m "client: play straight through several files, badge clips by video, show transcripts as they arrive" -m "A stitched clock drives one <video>: it swaps files when stitched time crosses a join and holds the element's events back while the next file loads, so usePlayback's piece tracking is unchanged. The canvas takes the shape of the first file with a picture, as the export does; clips carry their video number with a divider where videos meet, and a video still transcribing shows as a greyed block until its words are fetched.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 8: Layers UI — V2/V3 lanes, the Layer dialog, the selection toolbar and transcript tags

After this task, B-roll is gone from the UI. Select words, then choose Insert ▾ → **Layer…** or the floating toolbar's **Layer** button. A dialog lets you pick an uploaded clip or one of the project's own videos, choose V2 or V3, full frame or a PiP corner, and sound off or on at a level. The timeline shows V3, V2, Clips and Music from top to bottom; V3 is a thin "+ V3" row until it holds a clip. Clicking a layer bar or its transcript tag selects the layer, Delete removes it, and double-clicking the bar reopens the dialog to change its track, frame or sound. The Player still shows at most one full-frame B-roll-style layer until Task 9.

**Files:**

- Create: `client/src/components/LayerDialog.tsx`, `client/src/components/LayerDialog.module.css`, `client/src/components/LayerDialog.test.tsx`, `client/src/components/SpeakerIcon.tsx`
- Modify: `client/src/overlays.ts`, `client/src/overlays.test.ts`, `client/src/selection.ts`, `client/src/selection.test.ts`, `client/src/components/AssetPicker.tsx`, `client/src/components/AssetPicker.module.css`, `client/src/components/Timeline.tsx`, `client/src/components/Timeline.module.css`, `client/src/components/Timeline.test.tsx`, `client/src/components/SelectionToolbar.tsx`, `client/src/components/SelectionToolbar.test.tsx`, `client/src/components/Transcript.tsx`, `client/src/components/Transcript.module.css`, `client/src/components/Transcript.test.tsx`, `client/src/components/TopBar.tsx`, `client/src/components/TopBar.test.tsx`, `client/src/App.tsx`
- Delete: `client/src/components/BrollDialog.tsx`, `client/src/components/BrollDialog.module.css`

**Interfaces:**

- Consumes:
  - from Task 2: `LayerEdit`, `Frame`, `LayerTrack`, `SourceView` (`types.ts`); `layers(edits, track?)` (`overlays.ts`); the `addLayer`, `setLayer` and `removeLayer` editor actions.
  - from Task 7: `playable: SourceView[]` in App; the `sources: SourceView[]` props of `Timeline`, `Transcript` and `Player`.
  - existing: `audios`, `gainToLinear` (`overlays.ts`); `overlaySpans` (`timeline.ts`); `DialogFrame`; `AssetPicker`; `cx`; `formatTime`, `rangeForWords`, `EPS` (`editlist.ts`).
- Produces:

  ```ts
  // overlays.ts (Task 2's `layers(edits, track?)` and `layerAt` stay)
  export type { LayerTrack }; // re-exported from types.ts: 2 | 3
  export function layersOn(track: LayerTrack, edits: Edit[]): LayerEdit[];
  /** Layers covering source time t, lower tracks first (paint order). */
  export function layersAt(t: number, edits: Edit[]): LayerEdit[];
  export const FRAMES: readonly Frame[];
  export function frameLabel(frame: Frame): string; // 'Full' | 'PiP ↖' | 'PiP ↗' | 'PiP ↙' | 'PiP ↘'
  export function layerTag(layer: LayerEdit, name: string): string; // 'V3 · clip.mp4 · PiP ↗'
  export function layerVolume(audio: number | null): number | null; // null = muted
  export interface PipPlacement { width: string; top?: string; bottom?: string; left?: string; right?: string }
  export function pipPlacement(frame: Frame): PipPlacement | null; // null for 'full'
  export function mediaName(id: string, assets: Asset[], sources: SourceView[]): string | undefined;
  export function mediaUrl(id: string, assets: Asset[], sources: SourceView[]): string | undefined;
  // selection.ts
  export type OverlayRef =
    | { kind: 'layer'; track: LayerTrack; start: number }
    | { kind: 'audio'; start: number };
  export function overlayKey(ref: OverlayRef): string; // 'v2:12.5' | 'audio:0'
  export function sameOverlay(a: OverlayRef | null, b: OverlayRef): boolean;
  // AssetPicker.tsx
  export function sourceAsset(s: SourceView): Asset;
  // LayerDialog.tsx
  export interface LayerChoice { media: string; offset: number; track: LayerTrack; frame: Frame; audio: number | null }
  export const DEFAULT_LEVEL = -6;
  export function LayerDialog(props: {
    assets: Asset[]; sources: SourceView[]; initial?: LayerEdit | undefined;
    original: string; rangeLength: number;
    onUpload: (f: File) => Promise<Asset>; onSubmit: (choice: LayerChoice) => void;
    onRemove?: (() => void) | undefined; onCancel: () => void;
  }): JSX.Element;
  // Timeline.tsx: TimelineProps gains (its `sources` prop is Task 7's)
  onOpenLayer: (track: LayerTrack, start: number) => void;
  // Transcript.tsx: Props lose onBrollClick, gain (its `sources` prop is Task 7's)
  onLayerClick: (track: LayerTrack, start: number) => void;
  // SelectionToolbar.tsx: onBroll → onLayer
  // TopBar.tsx EditorControls: onAddBroll → onAddLayer
  ```

  DOM hooks: lanes carry `data-lane="v3" | "v2" | "clips" | "music"` in that order. Layer bars carry `data-overlay="v2:<start>"` or `data-overlay="v3:<start>"`. Transcript tags carry `data-layer-tag="<track>:<start>"`.

- [ ] **Step 1: Failing test for the layer helpers**

Append to `client/src/overlays.test.ts`, and merge these imports into the ones at the top of the file (Task 2 already imports `layerAt` and `layers` from `./overlays` and `Edit` and `Word` from `./types`):

```ts
import {
  FRAMES,
  frameLabel,
  layers,
  layersAt,
  layersOn,
  layerTag,
  layerVolume,
  mediaName,
  mediaUrl,
  pipPlacement,
} from './overlays';
import type { Asset, LayerEdit, SourceView } from './types';

const v2: LayerEdit = {
  kind: 'layer',
  track: 2,
  start: 2,
  end: 6,
  media: 'a1',
  offset: 0,
  frame: 'full',
  audio: null,
};
const v3: LayerEdit = {
  kind: 'layer',
  track: 3,
  start: 3,
  end: 5,
  media: 'm2',
  offset: 10,
  frame: 'pipTopRight',
  audio: -6,
};
const stacked: Edit[] = [
  v3,
  { kind: 'audio', start: 0, end: 10, media: 'mu', offset: 0, gain: 0, duck: true },
  v2,
];
const upload: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: null,
  height: null,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const take2: SourceView = {
  index: 1,
  mediaId: 'm2',
  url: '/data/m2.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 30,
  duration: 20,
  transcript: 'ready',
};

describe('layers', () => {
  it('lists layers per track, and those under a time lower track first', () => {
    expect(layers(stacked)).toEqual([v3, v2]);
    expect(layersOn(2, stacked)).toEqual([v2]);
    expect(layersOn(3, stacked)).toEqual([v3]);
    expect(layersAt(4, stacked)).toEqual([v2, v3]);
    expect(layersAt(5, stacked)).toEqual([v2]);
    expect(layersAt(6, stacked)).toEqual([]);
  });

  it('labels a layer the way its transcript tag reads', () => {
    expect(FRAMES).toEqual([
      'full',
      'pipTopLeft',
      'pipTopRight',
      'pipBottomLeft',
      'pipBottomRight',
    ]);
    expect(frameLabel('full')).toBe('Full');
    expect(frameLabel('pipTopLeft')).toBe('PiP ↖');
    expect(frameLabel('pipTopRight')).toBe('PiP ↗');
    expect(frameLabel('pipBottomLeft')).toBe('PiP ↙');
    expect(frameLabel('pipBottomRight')).toBe('PiP ↘');
    expect(layerTag(v3, 'take2.mp4')).toBe('V3 · take2.mp4 · PiP ↗');
    expect(layerTag(v2, 'a1.mp4')).toBe('V2 · a1.mp4');
  });

  it('turns a level in dB into a preview volume, and no sound into muted', () => {
    expect(layerVolume(null)).toBeNull();
    expect(layerVolume(0)).toBe(1);
    expect(layerVolume(-6)).toBeCloseTo(0.501, 3);
    // HTMLMediaElement.volume tops out at 1; the export applies the boost.
    expect(layerVolume(6)).toBe(1);
  });

  it('places a picture-in-picture 30% wide, 4% in from its corner', () => {
    expect(pipPlacement('full')).toBeNull();
    expect(pipPlacement('pipTopLeft')).toEqual({ width: '30%', top: '4%', left: '4%' });
    expect(pipPlacement('pipTopRight')).toEqual({ width: '30%', top: '4%', right: '4%' });
    expect(pipPlacement('pipBottomLeft')).toEqual({ width: '30%', bottom: '4%', left: '4%' });
    expect(pipPlacement('pipBottomRight')).toEqual({ width: '30%', bottom: '4%', right: '4%' });
  });

  it('finds a layer’s file among the uploads or the project’s own videos', () => {
    expect(mediaName('a1', [upload], [take2])).toBe('a1.mp4');
    expect(mediaName('m2', [upload], [take2])).toBe('take2.mp4');
    expect(mediaName('zz', [upload], [take2])).toBeUndefined();
    expect(mediaUrl('a1', [upload], [take2])).toBe('/data/p/assets/a1');
    expect(mediaUrl('m2', [upload], [take2])).toBe('/data/m2.mp4');
  });
});
```

(Merge rather than add a second import line from the same module, or the linter complains.)

- [ ] **Step 2: Run it and watch it fail**

Run: `npx vitest run client/src/overlays.test.ts`
Expected: FAIL. `layersOn`, `layersAt`, `FRAMES`, `frameLabel`, `layerTag`, `layerVolume`, `pipPlacement`, `mediaName` and `mediaUrl` are not exported. (`layers` is Task 2's.)

- [ ] **Step 3: Implement the layer helpers**

In `client/src/overlays.ts`, change the header comment and the type import:

```ts
// Overlay helpers for layers (V2/V3), background audio, gain, and the
// preview's speech envelope. Mirrors the engine's overlay handling for the client.

import type {
  Asset,
  AudioEdit,
  Edit,
  Frame,
  LayerEdit,
  LayerTrack,
  SourceView,
  Word,
} from './types';

/** A stacked video track (2 or 3). V1 is the main track and never holds a layer. */
export type { LayerTrack };
```

Keep `DUCK`, `audios`, `audiosAt`, `assetTime`, `gainToLinear` and `speaking` as they are, and Task 2's `layers(edits, track?)` and `layerAt(t, edits, track)`. Append:

```ts
export function layersOn(track: LayerTrack, edits: Edit[]): LayerEdit[] {
  return layers(edits, track);
}

/** The layers covering source time `t`, lower tracks first: the order they paint in. */
export function layersAt(t: number, edits: Edit[]): LayerEdit[] {
  return layers(edits)
    .filter((l) => t >= l.start && t < l.end)
    .sort((a, b) => a.track - b.track);
}

export const FRAMES: readonly Frame[] = [
  'full',
  'pipTopLeft',
  'pipTopRight',
  'pipBottomLeft',
  'pipBottomRight',
];

const ARROW: Record<Exclude<Frame, 'full'>, string> = {
  pipTopLeft: '↖',
  pipTopRight: '↗',
  pipBottomLeft: '↙',
  pipBottomRight: '↘',
};

export function frameLabel(frame: Frame): string {
  return frame === 'full' ? 'Full' : `PiP ${ARROW[frame]}`;
}

/** "V2 · clip.mp4", plus "· PiP ↗" for a picture-in-picture. */
export function layerTag(layer: LayerEdit, name: string): string {
  const parts = [`V${layer.track}`, name];
  if (layer.frame !== 'full') parts.push(frameLabel(layer.frame));
  return parts.join(' · ');
}

/**
 * A layer's preview volume: null when its sound is off (the element stays
 * muted), else its level as a linear factor. `HTMLMediaElement.volume` tops
 * out at 1, so a boost is capped here; the export applies the full level.
 */
export function layerVolume(audio: number | null): number | null {
  return audio === null ? null : Math.min(1, gainToLinear(audio));
}

/** Where a picture-in-picture sits in the frame, as CSS lengths. */
export interface PipPlacement {
  width: string;
  top?: string;
  bottom?: string;
  left?: string;
  right?: string;
}

/**
 * A picture-in-picture is 30% of the frame's width, 4% in from its corner
 * (4% of the width from the side, 4% of the height from the top or bottom).
 * The export uses the same numbers. Null for a full-frame layer.
 */
export function pipPlacement(frame: Frame): PipPlacement | null {
  if (frame === 'full') return null;
  const place: PipPlacement = { width: '30%' };
  if (frame === 'pipTopLeft' || frame === 'pipTopRight') place.top = '4%';
  else place.bottom = '4%';
  if (frame === 'pipTopLeft' || frame === 'pipBottomLeft') place.left = '4%';
  else place.right = '4%';
  return place;
}

/** A layer's or music bed's file name: an uploaded asset, else one of the project's own videos. */
export function mediaName(id: string, assets: Asset[], sources: SourceView[]): string | undefined {
  return assets.find((a) => a.id === id)?.name ?? sources.find((s) => s.mediaId === id)?.filename;
}

export function mediaUrl(id: string, assets: Asset[], sources: SourceView[]): string | undefined {
  return assets.find((a) => a.id === id)?.url ?? sources.find((s) => s.mediaId === id)?.url;
}
```

`AudioEdit` stays imported for `audios`/`audiosAt`.

- [ ] **Step 4: Run it and watch it pass**

Run: `npx vitest run client/src/overlays.test.ts`
Expected: PASS.

- [ ] **Step 5: Failing test for layer selections**

Replace `client/src/selection.test.ts` with:

```ts
import { describe, expect, it } from 'vitest';

import { deleteAction, overlayKey, sameOverlay } from './selection';

const none = { hasWords: false, title: null, clip: null, overlay: null };

describe('deleteAction', () => {
  it('deletes the words when a word selection appears over a selected overlay', () => {
    expect(
      deleteAction({ ...none, hasWords: true, overlay: { kind: 'layer', track: 2, start: 2 } }),
    ).toEqual({ type: 'deleteSelection' });
    expect(deleteAction({ ...none, hasWords: true, overlay: { kind: 'audio', start: 0 } })).toEqual(
      { type: 'deleteSelection' },
    );
  });

  it('removes a selected layer on its own track, or a selected music bar', () => {
    expect(deleteAction({ ...none, overlay: { kind: 'layer', track: 2, start: 2 } })).toEqual({
      type: 'removeLayer',
      track: 2,
      start: 2,
    });
    expect(deleteAction({ ...none, overlay: { kind: 'layer', track: 3, start: 2 } })).toEqual({
      type: 'removeLayer',
      track: 3,
      start: 2,
    });
    expect(deleteAction({ ...none, overlay: { kind: 'audio', start: 0 } })).toEqual({
      type: 'removeAudio',
      start: 0,
    });
  });

  it('removes a selected title card or joins a selected split', () => {
    expect(deleteAction({ ...none, title: 3 })).toEqual({ type: 'removeTitle', at: 3 });
    expect(deleteAction({ ...none, clip: 5 })).toEqual({ type: 'unsplit', at: 5 });
  });

  it('falls back to deleting the (possibly empty) word selection', () => {
    expect(deleteAction(none)).toEqual({ type: 'deleteSelection' });
  });
});

describe('overlay refs', () => {
  it('key a bar by its track, so V2 and V3 bars at one instant stay apart', () => {
    expect(overlayKey({ kind: 'layer', track: 2, start: 12.5 })).toBe('v2:12.5');
    expect(overlayKey({ kind: 'layer', track: 3, start: 12.5 })).toBe('v3:12.5');
    expect(overlayKey({ kind: 'audio', start: 0 })).toBe('audio:0');
  });

  it('match only the same kind, track and start', () => {
    const v2 = { kind: 'layer', track: 2, start: 4 } as const;
    expect(sameOverlay(v2, { kind: 'layer', track: 2, start: 4 })).toBe(true);
    expect(sameOverlay(v2, { kind: 'layer', track: 3, start: 4 })).toBe(false);
    expect(sameOverlay({ kind: 'audio', start: 4 }, v2)).toBe(false);
    expect(sameOverlay(null, v2)).toBe(false);
  });
});
```

- [ ] **Step 6: Run it and watch it fail**

Run: `npx vitest run client/src/selection.test.ts`
Expected: FAIL. `overlayKey` and `sameOverlay` are not exported, and a layer overlay yields `removeAudio`.

- [ ] **Step 7: Implement layer selections**

Replace `client/src/selection.ts` with:

```ts
// What Delete removes. The editor's selections (words, a title card, a split,
// a timeline overlay bar) are meant to be exclusive; if two ever coexist, the
// word selection wins, matching what the selection toolbar shows.

import type { EditorAction } from './editor';
import type { LayerTrack } from './overlays';

/** A selected timeline bar: a layer on V2 or V3, or a music bed. */
export type OverlayRef =
  { kind: 'layer'; track: LayerTrack; start: number } | { kind: 'audio'; start: number };

/** The `data-overlay` value a timeline bar carries; the selection toolbar anchors on it. */
export function overlayKey(ref: OverlayRef): string {
  return ref.kind === 'layer' ? `v${ref.track}:${ref.start}` : `audio:${ref.start}`;
}

/** Same bar: kind, track and start. Two tracks can hold layers starting at one instant. */
export function sameOverlay(a: OverlayRef | null, b: OverlayRef): boolean {
  return a !== null && overlayKey(a) === overlayKey(b);
}

export interface Selections {
  /** A word selection exists. */
  hasWords: boolean;
  /** A selected title card's instant. */
  title: number | null;
  /** A selected split's start (only a split; a cut boundary cannot be joined). */
  clip: number | null;
  overlay: OverlayRef | null;
}

export function deleteAction({ hasWords, title, clip, overlay }: Selections): EditorAction {
  if (hasWords) return { type: 'deleteSelection' };
  if (title !== null) return { type: 'removeTitle', at: title };
  if (clip !== null) return { type: 'unsplit', at: clip };
  if (overlay) {
    return overlay.kind === 'layer'
      ? { type: 'removeLayer', track: overlay.track, start: overlay.start }
      : { type: 'removeAudio', start: overlay.start };
  }
  return { type: 'deleteSelection' };
}
```

In `client/src/components/Timeline.tsx`, delete the local `OverlayRef` interface:

```ts
export interface OverlayRef {
  kind: 'broll' | 'audio';
  start: number;
}
```

In its place, import and re-export the type, so `App.tsx`'s `import { Timeline, type OverlayRef } from './components/Timeline'` keeps working:

```ts
import { overlayKey, sameOverlay, type OverlayRef } from '../selection';

export type { OverlayRef } from '../selection';
```

(Step 15 uses `overlayKey` and `sameOverlay`. Until then the linter may flag them as unused. That is fine, because nothing is linted before Step 22.)

In `client/src/components/SelectionToolbar.tsx`, change `import type { OverlayRef } from './Timeline';` to `import { overlayKey, type OverlayRef } from '../selection';`.

- [ ] **Step 8: Run it and watch it pass**

Run: `npx vitest run client/src/selection.test.ts`
Expected: PASS.

- [ ] **Step 9: Failing test for the Layer dialog**

Create `client/src/components/LayerDialog.test.tsx`:

```tsx
// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { Asset, LayerEdit, SourceView } from '../types';
import { LayerDialog } from './LayerDialog';

const clip: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: 1920,
  height: 1080,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const short: Asset = { ...clip, id: 'a2', name: 'short.mp4', duration: 1 };
const sources: SourceView[] = [
  {
    index: 0,
    mediaId: 'm1',
    url: '/data/m1.mp4',
    filename: 'take1.mp4',
    kind: 'video',
    offset: 0,
    duration: 30,
    transcript: 'ready',
  },
  {
    index: 1,
    mediaId: 'm2',
    url: '/data/m2.mp4',
    filename: 'take2.mp4',
    kind: 'video',
    offset: 30,
    duration: 20,
    transcript: 'ready',
  },
];
const onV3: LayerEdit = {
  kind: 'layer',
  track: 3,
  start: 3,
  end: 5,
  media: 'm2',
  offset: 10,
  frame: 'pipTopRight',
  audio: -6,
};

function setup(initial?: LayerEdit) {
  const onSubmit = vi.fn();
  const onRemove = vi.fn();
  render(
    <LayerDialog
      assets={[clip, short]}
      sources={sources}
      initial={initial}
      original="hello there"
      rangeLength={4}
      onUpload={vi.fn()}
      onSubmit={onSubmit}
      onRemove={initial ? onRemove : undefined}
      onCancel={vi.fn()}
    />,
  );
  return { onSubmit, onRemove };
}

const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const level = () => screen.getByRole('slider', { name: /^Level/ }) as HTMLInputElement;

describe('LayerDialog, adding over the selected words', () => {
  it('waits for a clip, then adds it on the chosen track, frame and sound', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(button('Add layer').disabled).toBe(true);
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(screen.getByRole('radio', { name: 'V3' }));
    await user.click(screen.getByRole('radio', { name: 'Top right' }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'a1',
      offset: 0,
      track: 3,
      frame: 'pipTopRight',
      audio: -6,
    });
  });

  it('defaults to V2, full frame, sound off', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(screen.getByRole('radio', { name: 'V2' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Full frame' }).getAttribute('aria-checked')).toBe(
      'true',
    );
    expect(level().disabled).toBe(true);
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'a1',
      offset: 0,
      track: 2,
      frame: 'full',
      audio: null,
    });
  });

  it('offers the project’s own videos, so a stretch of one can go over another', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    expect(screen.getByRole('radiogroup', { name: 'This project’s videos' })).toBeTruthy();
    await user.click(screen.getByRole('radio', { name: /Video 2 · take2\.mp4/ }));
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ media: 'm2', offset: 0 }));
  });

  it('sets the sound’s level', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup();
    await user.click(screen.getByRole('radio', { name: /a1\.mp4/ }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    fireEvent.change(level(), { target: { value: '-12' } });
    await user.click(button('Add layer'));
    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ audio: -12 }));
  });

  it('refuses a shot shorter than the selected words', async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole('radio', { name: /short\.mp4/ }));
    expect(screen.getByText(/shorter than the selected words/)).toBeTruthy();
    expect(button('Add layer').disabled).toBe(true);
  });
});

describe('LayerDialog, changing a layer', () => {
  it('changes the track, frame and sound, with no picker', async () => {
    const user = userEvent.setup();
    const { onSubmit } = setup(onV3);
    expect(screen.getByText(/Showing take2\.mp4/)).toBeTruthy();
    expect(screen.queryByRole('radio', { name: /a1\.mp4/ })).toBeNull();
    expect(screen.getByRole('radio', { name: 'V3' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Top right' }).getAttribute('aria-checked')).toBe(
      'true',
    );
    expect(
      (screen.getByRole('checkbox', { name: 'Play its sound' }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(level().value).toBe('-6');
    await user.click(screen.getByRole('radio', { name: 'V2' }));
    await user.click(screen.getByRole('radio', { name: 'Full frame' }));
    await user.click(screen.getByRole('checkbox', { name: 'Play its sound' }));
    await user.click(button('Save'));
    expect(onSubmit).toHaveBeenCalledWith({
      media: 'm2',
      offset: 10,
      track: 2,
      frame: 'full',
      audio: null,
    });
  });

  it('removes it', async () => {
    const user = userEvent.setup();
    const { onRemove } = setup(onV3);
    await user.click(button('Remove'));
    expect(onRemove).toHaveBeenCalledOnce();
  });
});
```

- [ ] **Step 10: Run it and watch it fail**

Run: `npx vitest run client/src/components/LayerDialog.test.tsx`
Expected: FAIL, because `./LayerDialog` cannot be resolved.

- [ ] **Step 11: Implement the picker's source group and the Layer dialog**

Replace `client/src/components/AssetPicker.tsx` with:

```tsx
import { useEffect, useRef, useState } from 'react';
import { cx } from '../cx';
import { formatTime } from '../editlist';
import type { Asset, MediaKind, SourceView } from '../types';
import picker from './AssetPicker.module.css';
import ui from '../styles/ui.module.css';

interface Props {
  assets: Asset[];
  /** The project's own videos, offered first so a stretch of one can go over another. */
  sources?: SourceView[] | undefined;
  kind: MediaKind;
  value: Asset | null;
  onChange: (a: Asset) => void;
  onUpload: (f: File) => Promise<Asset>;
  /** Focus the first interactive control (a card, or the upload button) on mount. */
  autoFocus?: boolean;
}

/**
 * A project source as a pickable card. Its id is the source's media id, which
 * is what a layer taken from it names; its offset is seconds into that file.
 */
export function sourceAsset(s: SourceView): Asset {
  return {
    id: s.mediaId,
    kind: s.kind,
    name: `Video ${s.index + 1} · ${s.filename}`,
    ext: s.filename.split('.').pop() ?? '',
    duration: s.duration,
    width: null,
    height: null,
    createdAt: 0,
    url: s.url,
    poster: null,
  };
}

export function AssetPicker({
  assets,
  sources,
  kind,
  value,
  onChange,
  onUpload,
  autoFocus,
}: Props) {
  const input = useRef<HTMLInputElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const own = (sources ?? []).filter((s) => s.kind === kind).map(sourceAsset);
  const list = assets.filter((a) => a.kind === kind);
  useEffect(() => {
    if (autoFocus) first.current?.focus();
  }, [autoFocus]);
  const upload = async (file: File) => {
    setBusy(true);
    setError(null);
    try {
      onChange(await onUpload(file));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const card = (a: Asset, isFirst: boolean) => (
    <button
      key={a.id}
      ref={isFirst ? first : undefined}
      type="button"
      role="radio"
      aria-checked={value?.id === a.id}
      className={cx(picker.card, value?.id === a.id && picker.selected)}
      disabled={busy}
      onClick={() => onChange(a)}
    >
      <span
        className={picker.poster}
        style={a.poster ? { backgroundImage: `url(${a.poster})` } : undefined}
      >
        {!a.poster && (
          <span className={picker.glyph} aria-hidden>
            {a.kind === 'video' ? '▶' : '♪'}
          </span>
        )}
        <span className={picker.length}>{formatTime(a.duration)}</span>
      </span>
      <span className={picker.name}>{a.name}</span>
    </button>
  );
  return (
    <div className={picker.picker}>
      {own.length > 0 && (
        <>
          <p className={picker.heading} aria-hidden>
            This project’s videos
          </p>
          <div className={picker.grid} role="radiogroup" aria-label="This project’s videos">
            {own.map((a, i) => card(a, i === 0))}
          </div>
          <p className={picker.heading} aria-hidden>
            Uploads
          </p>
        </>
      )}
      <div
        className={picker.grid}
        role="radiogroup"
        aria-label={own.length > 0 ? 'Uploads' : undefined}
      >
        {list.map((a, i) => card(a, own.length === 0 && i === 0))}
        <button
          ref={own.length === 0 && list.length === 0 ? first : undefined}
          type="button"
          className={cx(picker.card, picker.add)}
          disabled={busy}
          onClick={() => input.current?.click()}
        >
          {busy ? <span className={ui.spinnerSmall} aria-hidden /> : '+'}{' '}
          {busy ? 'Uploading…' : `Upload ${kind}`}
        </button>
      </div>
      <input
        ref={input}
        type="file"
        hidden
        accept={kind === 'video' ? 'video/*,.mp4,.mov' : 'audio/*,.mp3,.wav,.m4a'}
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) void upload(f);
          e.target.value = '';
        }}
      />
      {error && <p className={ui.error}>{error}</p>}
      {list.length === 0 && own.length === 0 && !busy && (
        <p className={ui.muted}>No {kind} assets yet — upload one.</p>
      )}
    </div>
  );
}
```

Append to `client/src/components/AssetPicker.module.css`:

```css
.heading {
  margin: 0;
  color: var(--muted);
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}
```

Create `client/src/components/LayerDialog.tsx`:

```tsx
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { cx } from '../cx';
import { formatTime } from '../editlist';
import { FRAMES, mediaName, type LayerTrack } from '../overlays';
import ui from '../styles/ui.module.css';
import type { Asset, Frame, LayerEdit, SourceView } from '../types';
import { AssetPicker } from './AssetPicker';
import { DialogFrame } from './DialogFrame';
import frameStyles from './DialogFrame.module.css';
import styles from './LayerDialog.module.css';

/** What the dialog hands back. `media` and `offset` matter only when adding. */
export interface LayerChoice {
  media: string;
  offset: number;
  track: LayerTrack;
  frame: Frame;
  audio: number | null;
}

interface Props {
  assets: Asset[];
  /** The project's own videos; the picker offers them next to the uploads. */
  sources: SourceView[];
  /** The layer being changed (SetLayer). Absent when adding over the selected words. */
  initial?: LayerEdit | undefined;
  /** The words the layer covers, and their length in seconds. */
  original: string;
  rangeLength: number;
  onUpload: (f: File) => Promise<Asset>;
  onSubmit: (choice: LayerChoice) => void;
  onRemove?: (() => void) | undefined;
  onCancel: () => void;
}

/** The level a layer's sound starts at when it is switched on, in dB. */
export const DEFAULT_LEVEL = -6;

const TRACKS: LayerTrack[] = [2, 3];

const FRAME_NAMES: Record<Frame, string> = {
  full: 'Full frame',
  pipTopLeft: 'Top left',
  pipTopRight: 'Top right',
  pipBottomLeft: 'Bottom left',
  pipBottomRight: 'Bottom right',
};

export function LayerDialog({
  assets,
  sources,
  initial,
  original,
  rangeLength,
  onUpload,
  onSubmit,
  onRemove,
  onCancel,
}: Props) {
  const [asset, setAsset] = useState<Asset | null>(null);
  const [offset, setOffset] = useState(0);
  const [track, setTrack] = useState<LayerTrack>(initial?.track ?? 2);
  const [frame, setFrame] = useState<Frame>(initial?.frame ?? 'full');
  const [sound, setSound] = useState(initial !== undefined && initial.audio !== null);
  const [level, setLevel] = useState(initial?.audio ?? DEFAULT_LEVEL);
  const preview = useRef<HTMLVideoElement>(null);
  const max = asset ? Math.max(0, asset.duration - rangeLength) : 0;
  const tooShort = !initial && asset !== null && asset.duration + 1e-6 < rangeLength;
  const ready = initial !== undefined || (asset !== null && !tooShort);
  useEffect(() => {
    setOffset(0);
  }, [asset]);
  useEffect(() => {
    if (preview.current) preview.current.currentTime = offset;
  }, [offset, asset]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!ready) return;
    const audio = sound ? level : null;
    if (initial) onSubmit({ media: initial.media, offset: initial.offset, track, frame, audio });
    else if (asset) onSubmit({ media: asset.id, offset, track, frame, audio });
  };
  const name = initial ? (mediaName(initial.media, assets, sources) ?? 'missing file') : '';
  return (
    <DialogFrame
      title={initial ? 'Layer' : 'Add layer'}
      description={
        initial ? (
          <>
            Showing {name} over “{original}” ({formatTime(rangeLength)}).
          </>
        ) : (
          <>
            Show a clip over “{original}” ({formatTime(rangeLength)}). The voice keeps going.
          </>
        )
      }
      wide
      onClose={onCancel}
    >
      <form className={frameStyles.form} onSubmit={submit}>
        {!initial && (
          <>
            <AssetPicker
              assets={assets}
              sources={sources}
              kind="video"
              value={asset}
              onChange={setAsset}
              onUpload={onUpload}
              autoFocus
            />
            {asset && (
              <>
                <video
                  ref={preview}
                  className={styles.preview}
                  src={asset.url}
                  muted
                  playsInline
                  preload="auto"
                />
                <label className={ui.field}>
                  Start {formatTime(offset)} into the shot
                  <input
                    type="range"
                    min={0}
                    max={max}
                    step={0.1}
                    value={offset}
                    disabled={max === 0}
                    onChange={(e) => setOffset(Number(e.target.value))}
                  />
                </label>
                {tooShort && (
                  <p className={ui.error}>This shot is shorter than the selected words.</p>
                )}
              </>
            )}
          </>
        )}

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Track
          </span>
          <div className={styles.segmented} role="radiogroup" aria-label="Track">
            {TRACKS.map((t) => (
              <button
                key={t}
                type="button"
                role="radio"
                aria-checked={track === t}
                className={cx(styles.option, track === t && styles.on)}
                onClick={() => setTrack(t)}
              >
                V{t}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Frame
          </span>
          <div className={styles.segmented} role="radiogroup" aria-label="Frame">
            {FRAMES.map((f) => (
              <button
                key={f}
                type="button"
                role="radio"
                aria-checked={frame === f}
                className={cx(styles.option, frame === f && styles.on)}
                onClick={() => setFrame(f)}
              >
                <span className={styles.thumb} aria-hidden>
                  <span className={cx(styles.pane, styles[f])} />
                </span>
                {FRAME_NAMES[f]}
              </button>
            ))}
          </div>
        </div>

        <div className={styles.row}>
          <span className={styles.label} aria-hidden>
            Sound
          </span>
          <div className={styles.sound}>
            <label className={ui.toggle}>
              <input type="checkbox" checked={sound} onChange={(e) => setSound(e.target.checked)} />{' '}
              Play its sound
            </label>
            <label className={cx(ui.field, styles.level)}>
              Level {level} dB
              <input
                type="range"
                min={-30}
                max={12}
                step={1}
                value={level}
                disabled={!sound}
                onChange={(e) => setLevel(Number(e.target.value))}
              />
            </label>
          </div>
        </div>
        {sound && level > 0 && (
          <p className={ui.muted}>
            Boost above 0 dB is applied on export; the preview cannot play louder than the source.
          </p>
        )}

        <div className={frameStyles.actions}>
          {initial && onRemove && (
            <button type="button" className={cx(ui.button, ui.danger)} onClick={onRemove}>
              Remove
            </button>
          )}
          <span className={frameStyles.spacer} />
          <button type="button" className={ui.button} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={cx(ui.button, ui.primary)} disabled={!ready}>
            {initial ? 'Save' : 'Add layer'}
          </button>
        </div>
      </form>
    </DialogFrame>
  );
}
```

Create `client/src/components/LayerDialog.module.css`:

```css
.preview {
  width: 100%;
  max-height: 180px;
  margin-top: var(--space-2);
  border-radius: var(--radius);
  background: #000;
}

.row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.label {
  flex: none;
  width: 48px;
  color: var(--muted);
  font-size: var(--text-md);
}

.segmented {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 2px;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--raised);
}

.option {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  font-size: var(--text-sm);
  cursor: pointer;
}

.option:hover {
  background: var(--hover);
}

.option.on {
  background: var(--accent);
  color: var(--on-accent);
}

/* A 16:9 miniature of the frame, with the layer drawn where it will sit. */
.thumb {
  position: relative;
  flex: none;
  width: 24px;
  height: 14px;
  border: 1px solid currentColor;
  border-radius: 2px;
  opacity: 0.85;
}

.pane {
  position: absolute;
  background: currentColor;
}

.full {
  inset: 1px;
}

.pipTopLeft,
.pipTopRight,
.pipBottomLeft,
.pipBottomRight {
  width: 8px;
  height: 5px;
}

.pipTopLeft {
  top: 1px;
  left: 1px;
}

.pipTopRight {
  top: 1px;
  right: 1px;
}

.pipBottomLeft {
  bottom: 1px;
  left: 1px;
}

.pipBottomRight {
  bottom: 1px;
  right: 1px;
}

.sound {
  display: flex;
  flex: 1;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-3);
}

.level {
  flex: 1;
  min-width: 160px;
}

@media (max-width: 600px) {
  .row {
    align-items: flex-start;
    flex-direction: column;
    gap: var(--space-1);
  }
}
```

Delete `client/src/components/BrollDialog.tsx` and `client/src/components/BrollDialog.module.css`. `App.tsx` stops importing `BrollDialog` in Step 21, and nothing else imports it.

- [ ] **Step 12: Run it and watch it pass**

Run: `npx vitest run client/src/components/LayerDialog.test.tsx`
Expected: PASS, 7 tests.

- [ ] **Step 13: Failing tests for the V2/V3 lanes**

In `client/src/components/Timeline.test.tsx`:

1. Task 7 already added `SourceView` to the `../types` import.
2. Task 2 already made the `edits` fixture's first edit a V2 layer (`{ kind: 'layer', track: 2, start: 2, end: 4, media: 'a1', offset: 0, frame: 'full', audio: null }`, followed by the music edit on `a2`). After that fixture, add:

   ```ts
   const take2: SourceView = {
     index: 1,
     mediaId: 'm2',
     url: '/data/m2.mp4',
     filename: 'take2.mp4',
     kind: 'video',
     offset: 30,
     duration: 20,
     transcript: 'ready',
   };
   const v3: Edit = {
     kind: 'layer',
     track: 3,
     start: 6,
     end: 8,
     media: 'm2',
     offset: 1,
     frame: 'pipTopRight',
     audio: -6,
   };
   ```

3. In `setup`'s `props`, add `onOpenLayer: vi.fn()`. (Task 7 already added `sources: []`.)
4. Replace the test `'selects a B-roll bar and opens a music bar on double-click'` with:

   ```ts
   it('selects a layer bar with its track and opens it on double-click; music opens too', () => {
     const props = setup();
     fireEvent.click(screen.getByRole('button', { name: 'V2 a1.mp4' }));
     expect(props.onSelectOverlay).toHaveBeenCalledWith({ kind: 'layer', track: 2, start: 2 });
     fireEvent.doubleClick(screen.getByRole('button', { name: 'V2 a1.mp4' }));
     expect(props.onOpenLayer).toHaveBeenCalledWith(2, 2);
     fireEvent.doubleClick(screen.getByRole('button', { name: 'Music a2.mp3' }));
     expect(props.onOpenAudio).toHaveBeenCalledWith(0);
   });
   ```

5. Replace the test `'tags each overlay bar for the selection toolbar to anchor on'` with:

   ```ts
   it('tags each overlay bar, by track, for the selection toolbar to anchor on', () => {
     setup();
     expect(screen.getByRole('button', { name: 'V2 a1.mp4' }).getAttribute('data-overlay')).toBe(
       'v2:2',
     );
     expect(screen.getByRole('button', { name: 'Music a2.mp3' }).getAttribute('data-overlay')).toBe(
       'audio:0',
     );
   });
   ```

6. Append a new `describe` block:

   ```ts
   describe('Timeline (layers)', () => {
     const laneOrder = () =>
       Array.from(screen.getByTestId('timeline-lanes').querySelectorAll('[data-lane]'), (l) =>
         l.getAttribute('data-lane'),
       );

     it('stacks V3, V2, Clips and Music, with a thin "+ V3" row until V3 holds a clip', () => {
       setup();
       expect(laneOrder()).toEqual(['v3', 'v2', 'clips', 'music']);
       expect(screen.getByText('+ V3')).toBeTruthy();
       expect(
         screen.getByTestId('timeline-lanes').querySelector('[data-lane="v3"] button'),
       ).toBeNull();
     });

     it('draws the V3 lane once it has a clip, named from the project’s own videos', () => {
       setup({ edits: [...edits, v3], sources: [take2] });
       expect(screen.queryByText('+ V3')).toBeNull();
       expect(screen.getByText('V3')).toBeTruthy();
       const bar = screen.getByRole('button', { name: 'V3 take2.mp4' });
       expect(bar.closest('[data-lane]')?.getAttribute('data-lane')).toBe('v3');
       expect(bar.getAttribute('data-overlay')).toBe('v3:6');
       expect(bar.textContent).toBe('PiP ↗ · take2.mp4');
     });

     it('marks only the selected bar, matched on track as well as start', () => {
       setup({
         edits: [...edits, { ...v3, start: 2, end: 4 } as Edit],
         sources: [take2],
         selectedOverlay: { kind: 'layer', track: 3, start: 2 },
       });
       expect(
         screen.getByRole('button', { name: 'V3 take2.mp4' }).getAttribute('aria-pressed'),
       ).toBe('true');
       expect(screen.getByRole('button', { name: 'V2 a1.mp4' }).getAttribute('aria-pressed')).toBe(
         'false',
       );
     });

     it('does not open a layer for a viewer', () => {
       const viewer = setup({ readOnly: true });
       fireEvent.doubleClick(screen.getByRole('button', { name: 'V2 a1.mp4' }));
       expect(viewer.onOpenLayer).not.toHaveBeenCalled();
     });
   });
   ```

   `textContent` is exact because the speaker icon is an SVG with no text nodes.

- [ ] **Step 14: Run them and watch them fail**

Run: `npx vitest run client/src/components/Timeline.test.tsx`
Expected: FAIL. There is no `v3` lane or "+ V3" row, the bar is labelled "B-roll a1.mp4", the `data-overlay` is `broll:2`, and `onOpenLayer` is never called.

- [ ] **Step 15: Implement the lanes**

In `client/src/components/Timeline.tsx`:

1. Imports. Replace the `overlays` import, the `types` import and the local `OverlayRef` (already done in Step 7) so the top reads:

   ```ts
   import { useMemo, useRef, useState, type PointerEvent } from 'react';

   import type { Thumbnails } from '../api';
   import { dropSlot, firstWords, moveFor } from '../clipstrip';
   import { cx } from '../cx';
   import { formatTime, locate } from '../editlist';
   import { audios, frameLabel, layersOn, mediaName, type LayerTrack } from '../overlays';
   import type { Peer } from '../realtime';
   import { overlayKey, sameOverlay, type OverlayRef } from '../selection';
   import {
     clipSpans,
     outputToSource,
     overlaySpans,
     pxToOutput,
     rangeCuts,
     razorAt,
     rulerTicks,
     snappedBand,
     sourceToOutput,
     timelineLength,
     type Segment,
   } from '../timeline';
   import type { Tool } from '../tools';
   import type { Asset, AudioEdit, Edit, LayerEdit, Range, SourceView, Word } from '../types';
   import { ScrubPreview } from './ScrubPreview';
   import { SpeakerIcon } from './SpeakerIcon';
   import styles from './Timeline.module.css';

   export type { OverlayRef } from '../selection';
   ```

   `locate` and `SourceView` are Task 7's, for the source badges and joins.

2. In `TimelineProps`, after `onOpenAudio`, add:

   ```ts
     /** Double-click on a layer bar: change its track, frame or sound. */
     onOpenLayer: (track: LayerTrack, start: number) => void;
   ```

   Task 7's `sources: SourceView[]` prop also names a layer taken from one of the project's own videos.

3. Replace `nameOf`:

   ```ts
   const nameOf = (id: string) => mediaName(id, assets, props.sources) ?? 'missing file';
   ```

4. Replace the whole `bars` function with:

   ```tsx
   interface BarItem {
     ref: OverlayRef;
     edit: LayerEdit | AudioEdit;
     label: string;
     text: string;
     open: () => void;
   }

   const overlayBars = (items: BarItem[]) =>
     items.flatMap(({ ref, edit: e, label, text, open }) => {
       const selected = sameOverlay(props.selectedOverlay, ref);
       const key = overlayKey(ref);
       // Music runs through title holds; a layer, like the picture it covers, does not.
       return overlaySpans(e, segments, ref.kind === 'audio').map((w, i) => (
         <button
           key={`${key}-${i}`}
           type="button"
           className={cx(
             styles.bar,
             ref.kind === 'layer' ? styles.layer : styles.audio,
             selected && styles.selected,
           )}
           style={{ left: pct(w.start), width: pct(w.end - w.start) }}
           data-overlay={key}
           aria-label={label}
           aria-pressed={selected}
           title={readOnly ? label : `${label} · double-click to edit`}
           onPointerDown={(ev) => {
             if (props.tool === 'select') ev.stopPropagation();
           }}
           onClick={() => {
             if (props.tool === 'select') props.onSelectOverlay(ref);
           }}
           onDoubleClick={() => {
             if (props.tool === 'select' && !readOnly) open();
           }}
         >
           {e.kind === 'layer' && e.audio !== null && <SpeakerIcon className={styles.sound} />}
           <span>{text}</span>
         </button>
       ));
     });

   const layerBars = (track: LayerTrack) =>
     overlayBars(
       layersOn(track, edits).map((l): BarItem => {
         const name = nameOf(l.media);
         return {
           ref: { kind: 'layer', track, start: l.start },
           edit: l,
           label: `V${track} ${name}`,
           text: l.frame === 'full' ? name : `${frameLabel(l.frame)} · ${name}`,
           open: () => props.onOpenLayer(track, l.start),
         };
       }),
     );
   const musicBars = overlayBars(
     audios(edits).map((a): BarItem => ({
       ref: { kind: 'audio', start: a.start },
       edit: a,
       label: `Music ${nameOf(a.media)}`,
       text: nameOf(a.media),
       open: () => props.onOpenAudio(a.start),
     })),
   );
   const hasV3 = layersOn(3, edits).length > 0;
   ```

   (`interface BarItem` goes at module level, above `export function Timeline`, not inside the component.)

5. Replace the labels column:

   ```tsx
   <div className={cx(styles.labels, hasV3 && styles.withV3)} aria-hidden>
     <span className={styles.rulerLabel} />
     <span className={cx(styles.overlayLabel, !hasV3 && styles.addLabel)}>
       {hasV3 ? 'V3' : '+ V3'}
     </span>
     <span className={styles.overlayLabel}>V2</span>
     <span>Clips</span>
     <span className={styles.overlayLabel}>Music</span>
   </div>
   ```

6. On the lanes `<div ref={lanes} …>`, change `className={styles.lanes}` to `className={cx(styles.lanes, hasV3 && styles.withV3)}`.
7. Between the ruler `<div className={styles.ruler}>…</div>` and the Clips lane, insert the two layer lanes:

   ```tsx
   {
     hasV3 ? (
       <div className={cx(styles.lane, styles.overlayLane)} data-lane="v3">
         {layerBars(3)}
       </div>
     ) : (
       <div
         className={cx(styles.lane, styles.overlayLane, styles.addLane)}
         data-lane="v3"
         title="Insert ▸ Layer… puts a clip on V3"
       />
     );
   }
   <div className={cx(styles.lane, styles.overlayLane)} data-lane="v2">
     {layerBars(2)}
   </div>;
   ```

8. Replace the old B-roll and Music lanes after the Clips lane with the Music lane alone:

   ```tsx
   <div className={cx(styles.lane, styles.overlayLane)} data-lane="music">
     {musicBars}
   </div>
   ```

   Delete the old `data-lane="broll"` lane (Task 2 left it drawing `layers(edits, 2)`) from below the Clips lane.

Create `client/src/components/SpeakerIcon.tsx`:

```tsx
interface Props {
  className?: string | undefined;
  /** Spoken name. Without one the icon is decorative and hidden from assistive tech. */
  label?: string | undefined;
}

/** A small speaker: this layer plays its own sound. */
export function SpeakerIcon({ className, label }: Props) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="12"
      height="12"
      className={className}
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <path d="M2 6h3l4-3v10l-4-3H2z" fill="currentColor" />
      <path
        d="M11 5.5a3.5 3.5 0 0 1 0 5M12.6 3.6a6 6 0 0 1 0 8.8"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
```

In `client/src/components/Timeline.module.css`:

1. Replace the two row definitions (in `.labels` and `.lanes`). Both currently read `grid-template-rows: 18px 30px 22px 22px;`. Change each to:

   ```css
   grid-template-rows: 18px 10px 22px 30px 22px;
   ```

   and add, after the `.lanes` rule:

   ```css
   /* Ruler, V3 (a thin "+ V3" row until it holds a clip), V2, Clips, Music. */
   .labels.withV3,
   .lanes.withV3 {
     grid-template-rows: 18px 22px 22px 30px 22px;
   }

   .addLane {
     border: 1px dashed var(--border);
     background: transparent;
   }

   .labels .addLabel {
     color: var(--faint);
     font-size: 10px;
   }
   ```

2. Rename `.broll { background: var(--broll); }` to:

   ```css
   .layer {
     background: var(--broll);
   }
   ```

3. Change the `.bar` rule so it lays out an icon and a name. Add these three declarations to it:

   ```css
   display: flex;
   align-items: center;
   gap: 3px;
   ```

   and add:

   ```css
   .bar span {
     min-width: 0;
   }

   .sound {
     flex: none;
   }
   ```

4. Replace the `@media (max-width: 600px)` block with:

   ```css
   @media (max-width: 600px) {
     .labels,
     .lanes,
     .labels.withV3,
     .lanes.withV3 {
       grid-template-rows: 18px 30px;
     }

     .overlayLane,
     .labels .overlayLabel {
       display: none;
     }
   }
   ```

   The V3, V2 and Music lanes all carry `overlayLane`, and their labels carry `overlayLabel`, so below 600 px only the ruler and the Clips lane are laid out.

- [ ] **Step 16: Run them and watch them pass**

Run: `npx vitest run client/src/components/Timeline.test.tsx`
Expected: PASS, including the Razor and Range tests, which do not change.

- [ ] **Step 17: Failing tests for the toolbar, the transcript tags and Insert → Layer…**

Replace `client/src/components/SelectionToolbar.test.tsx` with:

```tsx
// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { shouldIgnoreGlobalKey } from '../keyboardGuard';
import type { OverlayRef } from '../selection';
import { SelectionToolbar } from './SelectionToolbar';

function toolbar(
  overlay: OverlayRef | null,
  onDismiss = vi.fn(),
  anchorIndex: number | null = null,
) {
  const onDelete = vi.fn();
  const onLayer = vi.fn();
  render(
    <SelectionToolbar
      anchorIndex={anchorIndex}
      titleAt={null}
      clipStart={null}
      overlay={overlay}
      open
      onDelete={onDelete}
      onOverdub={vi.fn()}
      onCaption={vi.fn()}
      onLayer={onLayer}
      onDismiss={onDismiss}
    />,
  );
  return { onDelete, onLayer };
}

afterEach(() => vi.restoreAllMocks());

/** jsdom lays nothing out; give every element a visible box unless told otherwise. */
function layout(visible: boolean) {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(
    visible ? new DOMRect(10, 500, 80, 20) : new DOMRect(0, 0, 0, 0),
  );
}

describe('SelectionToolbar over words', () => {
  it('offers Layer where B-roll was', () => {
    layout(true);
    render(<button type="button" data-index="3" />);
    const { onLayer } = toolbar(null, vi.fn(), 3);
    const bar = screen.getByRole('toolbar', { name: 'Selection' });
    expect(Array.from(bar.querySelectorAll('button'), (b) => b.textContent)).toEqual([
      'Delete',
      'Overdub',
      'Caption',
      'Layer',
    ]);
    fireEvent.click(screen.getByRole('button', { name: 'Layer' }));
    expect(onLayer).toHaveBeenCalledOnce();
  });
});

describe('SelectionToolbar over a selected overlay bar', () => {
  it('offers Delete only, below the layer’s bar on its own track', () => {
    layout(true);
    render(
      <>
        <button type="button" data-overlay="v3:2" />
        <button type="button" data-overlay="v2:2" />
      </>,
    );
    const { onDelete } = toolbar({ kind: 'layer', track: 2, start: 2 });
    const bar = screen.getByRole('toolbar', { name: 'Selection' });
    expect(bar.getAttribute('data-side')).toBe('bottom');
    expect(Array.from(bar.querySelectorAll('button'), (b) => b.textContent)).toEqual(['Delete']);
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(onDelete).toHaveBeenCalledOnce();
  });

  it('stays closed while the bar is not in the document', () => {
    layout(true);
    toolbar({ kind: 'audio', start: 0 });
    expect(screen.queryByRole('toolbar')).toBeNull();
  });

  it('stays closed while the bar is hidden (no box, e.g. lanes hidden in a narrow window)', () => {
    layout(false);
    render(<button type="button" data-overlay="audio:0" />);
    toolbar({ kind: 'audio', start: 0 });
    expect(screen.queryByRole('toolbar')).toBeNull();
  });
});

describe('SelectionToolbar and Escape', () => {
  it('Escape dismisses it through onDismiss, the editor’s clear, though its layer takes the key', async () => {
    layout(true);
    const user = userEvent.setup();
    render(<button type="button" data-index="3" />);
    // The editor's own Escape handler, guarded as App's is.
    const appEscape = vi.fn();
    const onKey = (e: KeyboardEvent) => {
      if (!shouldIgnoreGlobalKey(e.target as Element | null, e.defaultPrevented)) appEscape();
    };
    window.addEventListener('keydown', onKey);
    const onDismiss = vi.fn();
    toolbar(null, onDismiss, 3);
    expect(screen.getByRole('toolbar', { name: 'Selection' })).toBeTruthy();
    await user.keyboard('{Escape}');
    window.removeEventListener('keydown', onKey);
    // The popover's layer swallowed the key, so the clear has to come from it.
    expect(appEscape).not.toHaveBeenCalled();
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it('a click elsewhere (another word) does not dismiss it', async () => {
    layout(true);
    const user = userEvent.setup();
    render(
      <>
        <button type="button" data-index="3" />
        <button type="button">elsewhere</button>
      </>,
    );
    const onDismiss = vi.fn();
    toolbar(null, onDismiss, 3);
    await user.click(screen.getByRole('button', { name: 'elsewhere' }));
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
```

Replace `client/src/components/Transcript.test.tsx` with the file below. It keeps Task 7's overridable `setup` and its "several videos" tests, and adds the layer tags:

```tsx
// @vitest-environment jsdom
import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { Asset, Edit, SourceView, Word } from '../types';
import { Transcript } from './Transcript';

const words: Word[] = ['one', 'two', 'three'].map((text, i) => ({
  id: `w${i}`,
  text,
  start: i,
  end: i + 0.5,
}));
const upload: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: null,
  height: null,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const take2: SourceView = {
  index: 1,
  mediaId: 'm2',
  url: '/data/m2.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 30,
  duration: 20,
  transcript: 'ready',
};

function setup(overrides: Partial<ComponentProps<typeof Transcript>> = {}) {
  const onWordDrag = vi.fn();
  const onWordDragEnd = vi.fn();
  const onLayerClick = vi.fn();
  render(
    <Transcript
      words={words}
      edits={[]}
      selected={null}
      activeWord={-1}
      playing={false}
      showCuts
      onWordClick={vi.fn()}
      onWordDrag={onWordDrag}
      onOverdubClick={vi.fn()}
      selectedTitle={null}
      onTitleClick={vi.fn()}
      onTitleOpen={vi.fn()}
      onCaptionClick={vi.fn()}
      onCutTransition={vi.fn()}
      speakers={null}
      speakerNames={[]}
      onRenameSpeaker={vi.fn()}
      readOnly={false}
      peers={[]}
      ordered={[{ start: 0, end: 3 }]}
      splits={[]}
      selectedClip={null}
      onClipClick={vi.fn()}
      assets={[upload]}
      sources={[take2]}
      onLayerClick={onLayerClick}
      onAudioClick={vi.fn()}
      tool="range"
      onWordDragEnd={onWordDragEnd}
      {...overrides}
    />,
  );
  return { onWordDrag, onWordDragEnd, onLayerClick };
}

describe('Transcript word drag (the Range tool cuts on its end)', () => {
  it('a plain click is no drag: nothing ends, so nothing is cut', () => {
    const { onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'two' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).not.toHaveBeenCalled();
  });

  it('a press dragged across words ends the drag on release', () => {
    const { onWordDrag, onWordDragEnd } = setup();
    fireEvent.mouseDown(screen.getByRole('button', { name: 'one' }), { button: 0 });
    fireEvent.mouseEnter(screen.getByRole('button', { name: 'two' }));
    fireEvent.mouseUp(window);
    expect(onWordDrag).toHaveBeenCalledWith(1);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
    // The next plain click starts over.
    fireEvent.mouseDown(screen.getByRole('button', { name: 'three' }), { button: 0 });
    fireEvent.mouseUp(window);
    expect(onWordDragEnd).toHaveBeenCalledOnce();
  });
});

describe('Transcript layer tags', () => {
  const layered: Edit[] = [
    {
      kind: 'layer',
      track: 2,
      start: 0,
      end: 1,
      media: 'a1',
      offset: 0,
      frame: 'full',
      audio: null,
    },
    {
      kind: 'layer',
      track: 3,
      start: 1,
      end: 3,
      media: 'm2',
      offset: 4,
      frame: 'pipTopRight',
      audio: -6,
    },
  ];

  it('reads "V3 · name · PiP ↗", with a speaker when its sound is on', () => {
    setup({ edits: layered });
    const v2 = screen.getByRole('button', { name: 'V2 · a1.mp4' });
    expect(within(v2).queryByRole('img')).toBeNull();
    const v3 = screen.getByRole('button', { name: /^V3 · take2\.mp4 · PiP ↗/ });
    expect(within(v3).getByRole('img', { name: 'Sound on, -6 dB' })).toBeTruthy();
  });

  it('follows the first word each layer covers', () => {
    setup({ edits: layered });
    const tag = screen.getByRole('button', { name: /^V3 · take2\.mp4/ });
    expect(tag.getAttribute('data-layer-tag')).toBe('3:1');
    // The word "two" (index 1) is the tag's nearest button before it.
    expect(tag.previousElementSibling?.textContent).toBe('two');
  });

  it('selects the layer on click, with its track', () => {
    const { onLayerClick } = setup({ edits: layered });
    fireEvent.click(screen.getByRole('button', { name: /^V3 · take2\.mp4/ }));
    expect(onLayerClick).toHaveBeenCalledWith(3, 1);
  });
});

const video = (
  index: number,
  offset: number,
  duration: number,
  transcript: SourceView['transcript'],
): SourceView => ({
  index,
  mediaId: `m${index}`,
  url: `/m${index}`,
  filename: `take${index + 1}.mp4`,
  kind: 'video',
  offset,
  duration,
  transcript,
});

describe('Transcript with several videos', () => {
  // The three words fill video 1, [0, 3); video 2 is [3, 8) with no words yet.
  const twoClips = {
    ordered: [
      { start: 0, end: 3 },
      { start: 3, end: 8 },
    ],
    splits: [3],
  };

  it('greys out a video that is still transcribing, in its own clip', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'running')] });
    const block = screen.getByRole('status');
    expect(block.textContent).toBe('Transcribing video 2…');
    expect(block.closest('section')?.getAttribute('data-clip-start')).toBe('3');
  });

  it('says so when a transcript failed, and shows nothing once it is ready', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'error')] });
    expect(screen.getByRole('status').textContent).toBe('Video 2 could not be transcribed');
  });

  it('labels a join as where a video starts, not as a split Delete could join', () => {
    setup({ ...twoClips, sources: [video(0, 0, 3, 'ready'), video(1, 3, 5, 'ready')] });
    expect(screen.queryByRole('status')).toBeNull();
    const divider = screen.getByRole('button', { name: /^Video 2 · Clip 2/ });
    expect(divider.getAttribute('title')).toMatch(/video 2 starts/i);
  });
});
```

In `client/src/components/TopBar.test.tsx`:

1. In `controls`, replace `onAddBroll: vi.fn(),` with `onAddLayer: vi.fn(),`.
2. `setup` already takes an `overrides` parameter (Task 6).
3. Append:

   ```tsx
   describe('Insert', () => {
     it('adds a layer from Insert → Layer… when words are selected', async () => {
       const user = userEvent.setup();
       const editor = setup({ status: 'idle' }, 0, { hasSelection: true });
       await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
       await user.click(await screen.findByRole('menuitem', { name: 'Layer…' }));
       expect(editor.onAddLayer).toHaveBeenCalledOnce();
     });

     it('offers no B-roll item any more', async () => {
       const user = userEvent.setup();
       setup({ status: 'idle' }, 0, { hasSelection: true });
       await user.click(screen.getByRole('button', { name: 'Insert ▾' }));
       await screen.findByRole('menuitem', { name: 'Layer…' });
       expect(screen.queryByRole('menuitem', { name: /B-roll/ })).toBeNull();
     });
   });
   ```

- [ ] **Step 18: Run them and watch them fail**

Run: `npx vitest run client/src/components/SelectionToolbar.test.tsx client/src/components/Transcript.test.tsx client/src/components/TopBar.test.tsx`
Expected: FAIL. The toolbar still says "B-roll" and anchors on `broll:`/`layer:`, not `v2:`. There are no layer tag buttons, and `onLayerClick`/`sources` are unknown props. There is no "Layer…" menu item.

- [ ] **Step 19: Implement the toolbar, the tags and the menu item**

`client/src/components/SelectionToolbar.tsx`:

1. In `Props`, change the `overlay` doc comment to `/** A selected layer or music bar; Delete-only, anchored to the timeline bar. */`, and replace `onBroll: () => void;` with:

   ```ts
     /** Put a layer over the selected words. */
     onLayer: () => void;
   ```

2. In `target()`, change the overlay selector line to:

   ```ts
         selector: `[data-overlay="${overlayKey(overlay)}"]`,
   ```

3. In the destructured parameters, replace `onBroll` with `onLayer`, and replace the last full-set button with:

   ```tsx
   <button type="button" className={button} onClick={onLayer}>
     Layer
   </button>
   ```

`client/src/components/Transcript.tsx`:

1. Imports: replace Task 2's `import { audios, layers } from '../overlays';` with

   ```ts
   import { audios, layersOn, layerTag, mediaName, type LayerTrack } from '../overlays';
   ```

   (`SourceView` is already in the `../types` import, from Task 7.) Also add:

   ```ts
   import { SpeakerIcon } from './SpeakerIcon';
   ```

2. In `Props`, replace `onBrollClick: (start: number) => void;` with

   ```ts
     /** Click a layer tag: select that layer (as clicking its timeline bar does). */
     onLayerClick: (track: LayerTrack, start: number) => void;
   ```

   and in the destructuring replace `onBrollClick,` with `onLayerClick,`. Task 7's `sources` prop (already destructured) also names a layer taken from one of the project's own videos.

3. Replace `nameOf` and `overlayTags` with:

   ```tsx
   const nameOf = (id: string) => mediaName(id, assets, sources) ?? 'missing asset';
   const overlayTags = (i: number): ReactNode => {
     // One tag per track whose layer starts at this word, V2 before V3.
     const starting = ([2, 3] as const).flatMap((track) => {
       const layer = startingAt(layersOn(track, edits), i);
       return layer ? [layer] : [];
     });
     const a = startingAt(audios(edits), i);
     return (
       <>
         {starting.map((l) => (
           <button
             key={`v${l.track}`}
             type="button"
             className={cx(styles.tag, styles.layerTag)}
             data-layer-tag={`${l.track}:${l.start}`}
             title="Click to select"
             onClick={() => onLayerClick(l.track, l.start)}
           >
             {layerTag(l, nameOf(l.media))}
             {l.audio !== null && (
               <SpeakerIcon className={styles.tagIcon} label={`Sound on, ${l.audio} dB`} />
             )}
           </button>
         ))}
         {a && (
           <span
             className={cx(styles.tag, styles.musicTag)}
             title={readOnly ? 'Music' : 'Click to edit'}
             onClick={readOnly ? undefined : () => onAudioClick(a.start)}
           >
             ♪ {nameOf(a.media)} {a.gain}dB
           </span>
         )}
       </>
     );
   };
   ```

`client/src/components/Transcript.module.css`: replace the `.brollTag` rule with:

```css
.layerTag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 0;
  background: var(--broll);
  color: #0b0e14;
  font-family: inherit;
}

.tagIcon {
  flex: none;
}
```

`client/src/components/TopBar.tsx`:

1. In `EditorControls`, replace `onAddBroll: () => void;` with `onAddLayer: () => void;`.
2. In the Insert menu, replace the B-roll item with:

   ```tsx
   <DropdownMenu.Item
     className={styles.item}
     disabled={!editor.hasSelection}
     onSelect={editor.onAddLayer}
   >
     Layer…
   </DropdownMenu.Item>
   ```

   Task 6 adds "Add video…" to this menu. Keep its item wherever Task 6 put it.

- [ ] **Step 20: Run them and watch them pass**

Run: `npx vitest run client/src/components/SelectionToolbar.test.tsx client/src/components/Transcript.test.tsx client/src/components/TopBar.test.tsx`
Expected: PASS.

- [ ] **Step 21: Wire the dialog and the selection into `App.tsx`**

1. Imports:
   - Replace `import { BrollDialog } from './components/BrollDialog';` with `import { LayerDialog } from './components/LayerDialog';`.
   - Replace Task 2's `import { audios, layers } from './overlays';` with `import { audios, layers, type LayerTrack } from './overlays';`.
   - Add `LayerEdit` to the `./types` import.
2. State. Replace the `brollRange` state and its comment with:

   ```ts
   // The layer dialog: adding over the word range it was opened on, or
   // changing an existing layer's track, frame or sound.
   const [layerDialog, setLayerDialog] = useState<
     { range: [number, number] } | { edit: LayerEdit } | null
   >(null);
   ```

   Change the comment above `selectedOverlay` to `// A selected layer or music bar. Exclusive with every other selection.`

3. Replace every remaining `brollRange` with `layerDialog`. These are in the `keyHandler` guard, its dependency list and the `ToolToolbar` `shortcuts` guard. In `goHome`, replace `setBrollRange(null);` with `setLayerDialog(null);`.
4. The unknown-media refetch effect should look up layers, and should treat the project's own videos (`playable`, Task 7) as known:

   ```ts
   useEffect(() => {
     const have = new Set([...assets.map((a) => a.id), ...playable.map((s) => s.mediaId)]);
     const unknown = [...layers(editor.edits), ...audios(editor.edits)]
       .map((e) => e.media)
       .filter((id) => !have.has(id) && !soughtAssets.current.has(id));
     if (unknown.length === 0) return;
     for (const id of unknown) soughtAssets.current.add(id);
     refreshAssets();
   }, [editor.edits, assets, playable, refreshAssets]);
   ```

5. After `openAudio`, add:

   ```ts
   // A layer bar's double-click opens its dialog to change track, frame or sound.
   const openLayer = useCallback(
     (track: LayerTrack, start: number) => {
       const found = layers(editor.edits).find(
         (l) => l.track === track && Math.abs(l.start - start) < EPS,
       );
       if (found) setLayerDialog({ edit: found });
     },
     [editor.edits],
   );
   ```

6. Replace the stale-overlay effect with:

   ```ts
   // A peer's edit (or an undo) can remove the layer or music we had selected.
   useEffect(() => {
     if (!selectedOverlay) return;
     const still =
       selectedOverlay.kind === 'layer'
         ? layers(editor.edits).some(
             (l) =>
               l.track === selectedOverlay.track && Math.abs(l.start - selectedOverlay.start) < EPS,
           )
         : audios(editor.edits).some((a) => Math.abs(a.start - selectedOverlay.start) < EPS);
     if (!still) setSelectedOverlay(null);
   }, [editor.edits, selectedOverlay]);
   ```

   Change `deleteSelected`'s doc comment to `/** Delete whatever is selected: words, a title card, a split, or a layer or music bar. */`. Its body is unchanged, because `deleteAction` now returns `removeLayer`.

7. After `const captionText = wordsIn(captionRange);`, add:

   ```ts
   // What the layer dialog describes: the words its layer covers, and their length.
   const layerWords = (() => {
     if (!layerDialog) return { text: '', length: 0 };
     if ('edit' in layerDialog) {
       const { start, end } = layerDialog.edit;
       const text = editor.words
         .filter((w) => w.start >= start - EPS && w.start < end - EPS)
         .map((w) => w.text)
         .join(' ');
       return { text, length: end - start };
     }
     const [from, to] = layerDialog.range;
     const r = rangeForWords(editor.words, from, to, editor.duration);
     return { text: wordsIn(layerDialog.range), length: r.end - r.start };
   })();
   ```

8. In `controls`, replace `onAddBroll` with:

   ```ts
   onAddLayer: () => {
     if (selected) setLayerDialog({ range: selected });
   },
   ```

9. `<Transcript>`: replace Task 2's `onBrollClick={(start) => edit({ type: 'removeLayer', track: 2, start })}` with

   ```tsx
   onLayerClick={(track, start) => onSelectOverlay({ kind: 'layer', track, start })}
   ```

   (`sources={playable}` is already there, from Task 7.) A tag click goes through `onSelectOverlay`, which clears the word, title and clip selections. Selections stay exclusive.

10. `<SelectionToolbar>`: replace `onBroll={…}` with

    ```tsx
    onLayer={() => {
      if (selected) setLayerDialog({ range: selected });
    }}
    ```

11. `<Timeline>`: add `onOpenLayer={openLayer}` (`sources={playable}` is already there, from Task 7).
12. Replace the `{brollRange && (<BrollDialog … />)}` block with:

    ```tsx
    {
      layerDialog && (
        <LayerDialog
          assets={assets}
          sources={playable}
          initial={'edit' in layerDialog ? layerDialog.edit : undefined}
          original={layerWords.text}
          rangeLength={layerWords.length}
          onUpload={onUploadAsset}
          onSubmit={(choice) => {
            if ('edit' in layerDialog) {
              const { track, start } = layerDialog.edit;
              edit({
                type: 'setLayer',
                track,
                start,
                toTrack: choice.track,
                frame: choice.frame,
                audio: choice.audio,
              });
              // Keep the bar selected when it moves to the other track.
              if (
                selectedOverlay?.kind === 'layer' &&
                selectedOverlay.track === track &&
                Math.abs(selectedOverlay.start - start) < EPS
              )
                setSelectedOverlay({ kind: 'layer', track: choice.track, start });
            } else {
              edit({
                type: 'addLayer',
                track: choice.track,
                media: choice.media,
                offset: choice.offset,
                frame: choice.frame,
                audio: choice.audio,
                range: layerDialog.range,
              });
            }
            setLayerDialog(null);
          }}
          onRemove={
            'edit' in layerDialog
              ? () => {
                  edit({
                    type: 'removeLayer',
                    track: layerDialog.edit.track,
                    start: layerDialog.edit.start,
                  });
                  setLayerDialog(null);
                }
              : undefined
          }
          onCancel={() => setLayerDialog(null)}
        />
      );
    }
    ```

    Then check that none of the B-roll UI names Task 2 kept remains:

    ```bash
    grep -rn "BrollDialog\|brollRange\|BrollRange\|removeBroll\|addBroll\|onBrollClick\|onBroll\b\|onAddBroll\|'broll'\|styles\.broll\b\|brollTag\|B-roll" client/src --include='*.ts' --include='*.tsx' --include='*.css' --exclude='*.test.ts' --exclude='*.test.tsx' | grep -v "components/Overlays"
    ```

    Expected: no output. (Task 2 removed the `addBroll`/`removeBroll` actions and ops outright; old logs still fold on the server. `Overlays.tsx` and its module keep their B-roll names until Task 9 rewrites them.) The colour token `--broll` stays: it is the lane colour, now V2 and V3's. If a comment, label or hint still says B-roll, rename it to layer (or V2) in this commit.

- [ ] **Step 22: Verify**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npx prettier --write client/src/overlays.ts client/src/overlays.test.ts client/src/selection.ts client/src/selection.test.ts client/src/App.tsx client/src/components/{AssetPicker,LayerDialog,SpeakerIcon,Timeline,SelectionToolbar,Transcript,TopBar}.tsx client/src/components/{LayerDialog,Timeline,SelectionToolbar,Transcript,TopBar}.test.tsx client/src/components/{AssetPicker,LayerDialog,Timeline,Transcript}.module.css
npx vitest run && npx tsc -p client --noEmit && npm run lint
```

Expected: all pass.

- [ ] **Step 23: Commit**

```bash
git add -A client/src
git commit -m "client: V2 and V3 layer lanes, the Layer dialog with track, frame and sound, Layer in the toolbar and the Insert menu, and layer tags in the transcript" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 9: A stack of layer previews in the Player

After this task, the Player shows every layer under the playhead as its own `<video>` above the main picture, with V3 over V2. A full-frame layer covers the picture. A PiP layer is a box 30% of the frame's width, 4% in from its corner. A layer with sound plays at its level (ducked under speech like music); every other layer is muted. Each layer tracks the playhead exactly as B-roll did: it seeks when it drifts more than 0.3 s and plays and pauses with the transport. The timeline's hover preview still shows the main track's frame.

**Files:**

- Create: `client/src/components/Overlays.test.tsx`
- Modify: `client/src/components/Overlays.tsx` (rewrite), `client/src/components/Overlays.module.css` (rewrite), `client/src/components/Player.tsx` (the `<Overlays>` line only), `client/src/components/Timeline.test.tsx`

**Interfaces:**

- Consumes: from Task 8, `layersAt`, `layerVolume`, `pipPlacement`, `mediaUrl`; the existing `assetTime`, `audiosAt`, `DUCK`, `gainToLinear` and `speaking` (`overlays.ts`); `Playback` (`usePlayback.ts`); from Task 2, `SourceView`; from Task 7, the Player's `sources: SourceView[]` and `stitched: StitchedMedia` props (App passes `sources={playable}`).
- Produces:

  ```ts
  // Overlays.tsx
  export function Overlays(props: {
    edits: Edit[];
    assets: Asset[];
    sources: SourceView[];
    words: Word[];
    playback: Playback;
  }): JSX.Element;
  // Player.tsx: Props unchanged from Task 7 (`sources`, `stitched`, `edits`, `assets`,
  // `words`, `playback`, `segments`, `transition`); it passes `sources` on to Overlays.
  ```

  DOM hooks: each layer's wrapper has `data-layer="<track>:<start>"` and `data-frame="<frame>"`. The wrappers are siblings, in paint order.

- [ ] **Step 1: Failing tests for the preview stack**

Create `client/src/components/Overlays.test.tsx`:

```tsx
// @vitest-environment jsdom
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DUCK, gainToLinear } from '../overlays';
import type { Asset, Edit, SourceView, Word } from '../types';
import type { Playback } from '../usePlayback';
import { Overlays } from './Overlays';

const upload: Asset = {
  id: 'a1',
  kind: 'video',
  name: 'a1.mp4',
  ext: 'mp4',
  duration: 30,
  width: null,
  height: null,
  createdAt: 0,
  url: '/data/p/assets/a1',
  poster: null,
};
const take2: SourceView = {
  index: 1,
  mediaId: 'm2',
  url: '/data/m2.mp4',
  filename: 'take2.mp4',
  kind: 'video',
  offset: 30,
  duration: 20,
  transcript: 'ready',
};
// V2 full frame and muted over [2, 6); V3 a top-right PiP of video 2, with sound, over [3, 5).
const edits: Edit[] = [
  {
    kind: 'layer',
    track: 3,
    start: 3,
    end: 5,
    media: 'm2',
    offset: 10,
    frame: 'pipTopRight',
    audio: -6,
  },
  {
    kind: 'layer',
    track: 2,
    start: 2,
    end: 6,
    media: 'a1',
    offset: 0,
    frame: 'full',
    audio: null,
  },
];

function playbackAt(currentTime: number, playing = false): Playback {
  return {
    playing,
    currentTime,
    outputTime: currentTime,
    atEnd: false,
    activeWord: -1,
    overdubbing: null,
    titling: null,
    toggle: vi.fn(),
    seek: vi.fn(),
    seekOutput: vi.fn(),
  } as Playback;
}

function show(t: number, opts: { playing?: boolean; words?: Word[]; edits?: Edit[] } = {}) {
  return render(
    <Overlays
      edits={opts.edits ?? edits}
      assets={[upload]}
      sources={[take2]}
      words={opts.words ?? []}
      playback={playbackAt(t, opts.playing)}
    />,
  );
}

const stack = (root: HTMLElement) => Array.from(root.querySelectorAll<HTMLElement>('[data-layer]'));
const videoOf = (el: HTMLElement) => el.querySelector('video') as HTMLVideoElement;

let play: ReturnType<typeof vi.fn>;
beforeEach(() => {
  // jsdom implements neither; play must return a promise, as browsers do.
  play = vi.fn(() => Promise.resolve());
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(play);
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

describe('Overlays: the layer stack', () => {
  it('stacks one video per visible layer, upper tracks painted last', () => {
    const { container } = show(4);
    const layers = stack(container);
    expect(layers.map((l) => l.getAttribute('data-layer'))).toEqual(['2:2', '3:3']);
    expect(layers.map((l) => videoOf(l).getAttribute('src'))).toEqual([
      '/data/p/assets/a1',
      '/data/m2.mp4',
    ]);
  });

  it('shows only the layers under the playhead', () => {
    expect(stack(show(5.5).container).map((l) => l.getAttribute('data-layer'))).toEqual(['2:2']);
  });

  it('shows nothing where no layer is', () => {
    expect(stack(show(7).container)).toHaveLength(0);
  });

  it('insets a picture-in-picture 30% wide from its corner, and fills the frame otherwise', () => {
    const [full, pip] = stack(show(4).container) as [HTMLElement, HTMLElement];
    expect(full.getAttribute('data-frame')).toBe('full');
    expect(full.style.width).toBe('');
    expect(pip.getAttribute('data-frame')).toBe('pipTopRight');
    expect(pip.style.width).toBe('30%');
    expect(pip.style.top).toBe('4%');
    expect(pip.style.right).toBe('4%');
    expect(pip.style.left).toBe('');
    expect(pip.style.bottom).toBe('');
  });

  it('mutes a layer without sound, and plays one with sound at its level', () => {
    const [full, pip] = stack(show(4).container) as [HTMLElement, HTMLElement];
    expect(videoOf(full).muted).toBe(true);
    expect(videoOf(pip).muted).toBe(false);
    expect(videoOf(pip).volume).toBeCloseTo(gainToLinear(-6), 5);
  });

  it('ducks a layer’s sound under speech, as music ducks', () => {
    const words: Word[] = [{ id: 'w', text: 'hi', start: 3.8, end: 4.4 }];
    const [, pip] = stack(show(4, { words }).container) as [HTMLElement, HTMLElement];
    expect(videoOf(pip).volume).toBeCloseTo(gainToLinear(-6) * DUCK, 5);
  });

  it('keeps each layer on the playhead, the way B-roll did', () => {
    const [full, pip] = stack(show(4, { playing: true }).container) as [HTMLElement, HTMLElement];
    // Seconds into each file: offset + (t - start).
    expect(videoOf(full).currentTime).toBe(2);
    expect(videoOf(pip).currentTime).toBe(11);
    expect(play).toHaveBeenCalledTimes(2);
  });

  it('leaves a layer alone while it is within 0.3 s of the playhead', () => {
    const { container, rerender } = show(4);
    const pip = stack(container)[1] as HTMLElement;
    videoOf(pip).currentTime = 11.2;
    rerender(
      <Overlays
        edits={edits}
        assets={[upload]}
        sources={[take2]}
        words={[]}
        playback={playbackAt(4.1)}
      />,
    );
    expect(videoOf(pip).currentTime).toBe(11.2);
  });

  it('draws no layer, picture or sound, over a title card, as the export does', () => {
    const titling = { ...playbackAt(4), titling: { text: 'T' } } as unknown as Playback;
    const { container } = render(
      <Overlays edits={edits} assets={[upload]} sources={[take2]} words={[]} playback={titling} />,
    );
    expect(stack(container)).toHaveLength(0);
  });

  it('marks a layer whose file is missing', () => {
    const lost: Edit[] = [
      {
        kind: 'layer',
        track: 2,
        start: 2,
        end: 6,
        media: 'gone',
        offset: 0,
        frame: 'full',
        audio: null,
      },
    ];
    show(4, { edits: lost });
    expect(screen.getByText('Layer file missing')).toBeTruthy();
  });
});
```

Append a guard test to the `'Timeline (layers)'` block in `client/src/components/Timeline.test.tsx`. It passes as soon as it is written, because the hover preview is already the main track's. It is there to keep it that way:

```ts
it('keeps the hover preview on the main track’s frame over a layer', () => {
  const thumbs = {
    url: '/thumbs.jpg',
    count: 10,
    columns: 10,
    rows: 1,
    interval: 1,
    width: 64,
    height: 36,
  };
  setup({ thumbs });
  // Output 7.5 is source 2.5 (clip 2 is source [0, 5) at output [5, 10)),
  // under the V2 layer over source [2, 4): the preview is still source frame 2.
  fireEvent.pointerMove(screen.getByTestId('timeline-lanes'), { clientX: 750 });
  const frame = document.querySelector<HTMLElement>('[style*="thumbs.jpg"]');
  expect(frame?.style.backgroundImage).toContain('/thumbs.jpg');
  expect(frame?.style.backgroundPosition).toMatch(/^-128px/);
});
```

- [ ] **Step 2: Run them and watch them fail**

Run: `npx vitest run client/src/components/Overlays.test.tsx client/src/components/Timeline.test.tsx`
Expected: `Overlays.test.tsx` FAILS. `sources` is not a prop, no element carries `data-layer`, at most one full-frame muted video renders, and the PiP has no placement. `Timeline.test.tsx` passes, including the new guard.

- [ ] **Step 3: Implement the stack**

Replace `client/src/components/Overlays.tsx` with:

```tsx
import { useEffect, useRef, useState } from 'react';

import { cx } from '../cx';
import {
  assetTime,
  audiosAt,
  DUCK,
  gainToLinear,
  layersAt,
  layerVolume,
  mediaUrl,
  pipPlacement,
  speaking,
} from '../overlays';
import type { Asset, AudioEdit, Edit, LayerEdit, SourceView, Word } from '../types';
import type { Playback } from '../usePlayback';
import styles from './Overlays.module.css';

interface Props {
  edits: Edit[];
  assets: Asset[];
  /** The project's own videos: a layer can show a stretch of one of them. */
  sources: SourceView[];
  words: Word[];
  playback: Playback;
}

/**
 * The layer stack and the music under the playhead. One video per visible
 * layer, lower tracks first: siblings paint in DOM order, so V3 covers V2
 * covers the main picture, and captions and title cards (after this in the
 * Player) stay on top. One audio element per music edit.
 */
export function Overlays({ edits, assets, sources, words, playback }: Props) {
  const t = playback.currentTime;
  const url = (id: string) => mediaUrl(id, assets, sources);
  const talking = speaking(t, words);
  // Like the export, a layer's picture and sound are both absent over a title card.
  const shown = playback.titling ? [] : layersAt(t, edits);
  return (
    <>
      {shown.map((l) => (
        <LayerVideo
          key={`${l.track}:${l.start}`}
          src={url(l.media)}
          edit={l}
          t={t}
          playing={playback.playing}
          ducked={talking}
        />
      ))}
      {audiosAt(t, edits).map((a) => (
        <AudioBed
          key={a.start}
          src={url(a.media)}
          edit={a}
          t={t}
          playing={playback.playing}
          ducked={a.duck && talking}
        />
      ))}
    </>
  );
}

function LayerVideo({
  src,
  edit,
  t,
  playing,
  ducked,
}: {
  src: string | undefined;
  edit: LayerEdit;
  t: number;
  playing: boolean;
  ducked: boolean;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  const [missing, setMissing] = useState(src === undefined);
  useEffect(() => {
    // Clears the placeholder once `listAssets` resolves and the file shows up.
    setMissing(src === undefined);
  }, [src]);
  const volume = layerVolume(edit.audio);
  useEffect(() => {
    const v = ref.current;
    if (!v) return;
    const want = assetTime(edit, t);
    if (Math.abs(v.currentTime - want) > 0.3) v.currentTime = want;
    // Muted unless the layer's sound is on; then its level, ducked under
    // speech like music, as the export mixes it.
    v.muted = volume === null;
    if (volume !== null) v.volume = volume * (ducked ? DUCK : 1);
    if (playing && v.paused)
      void v.play().catch(() => {
        // Autoplay can be blocked; the layer just stays paused.
      });
    if (!playing && !v.paused) v.pause();
  }, [edit, t, playing, volume, ducked]);
  // Explicit stop when this layer unmounts (its range ended, or the whole
  // player did), rather than relying on removal from the document to pause
  // it. A cleanup on the sync effect above would fire every tick (it depends
  // on `t`), so this is its own mount-only effect.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  const place = pipPlacement(edit.frame);
  return (
    <div
      className={cx(styles.layer, place && styles.pip)}
      style={place ?? undefined}
      data-layer={`${edit.track}:${edit.start}`}
      data-frame={edit.frame}
    >
      {missing ? (
        <span className={styles.missing}>Layer file missing</span>
      ) : (
        <video
          ref={ref}
          className={styles.video}
          src={src}
          muted={volume === null}
          playsInline
          preload="auto"
          onError={() => setMissing(true)}
        />
      )}
    </div>
  );
}

function AudioBed({
  src,
  edit,
  t,
  playing,
  ducked,
}: {
  src: string | undefined;
  edit: AudioEdit;
  t: number;
  playing: boolean;
  ducked: boolean;
}) {
  const ref = useRef<HTMLAudioElement>(null);
  useEffect(() => {
    const a = ref.current;
    if (!a) return;
    const want = assetTime(edit, t);
    if (Math.abs(a.currentTime - want) > 0.3) a.currentTime = want;
    // `HTMLMediaElement.volume` tops out at 1, so a positive gain cannot be
    // previewed: the clamp keeps the value legal. The export honours the full
    // −30..+12 dB range, and `AudioDialog` says so when the gain is a boost.
    a.volume = Math.min(1, gainToLinear(edit.gain) * (ducked ? DUCK : 1));
    if (playing && a.paused)
      void a.play().catch(() => {
        // Autoplay can be blocked; the bed just stays paused.
      });
    if (!playing && !a.paused) a.pause();
  }, [edit, t, playing, ducked]);
  // Explicit stop when this overlay unmounts, rather than relying on removal
  // from the document to pause it; see the matching note in `LayerVideo`.
  useEffect(
    () => () => {
      ref.current?.pause();
    },
    [],
  );
  if (!src) return null;
  return <audio ref={ref} src={src} preload="auto" />;
}
```

Replace `client/src/components/Overlays.module.css` with:

```css
/* A layer fills the frame; a picture-in-picture is placed by inline style
   (width 30%, 4% in from its corner) and takes its height from the video. */
.layer {
  position: absolute;
  inset: 0;
  pointer-events: none;
}

.pip {
  inset: auto;
}

/* Beats the Player's `.frame video` rule (0,1,1) with (0,2,0). */
.layer .video {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  background: #000;
  cursor: default;
}

.pip .video {
  height: auto;
}

.missing {
  position: absolute;
  top: 8px;
  left: 8px;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  background: var(--danger);
  color: var(--on-accent);
  font-size: var(--text-sm);
}
```

Task 2 already removed `brolls` and `brollAt`; `layerAt` stays (Task 2's tests use it). Confirm that no B-roll name is left anywhere in the client:

```bash
grep -rn "brollAt\|brolls\b\|BrollVideo\|styles.broll\b" client/src
grep -rni "broll" client/src | grep -v -- "--broll"
```

Expected: no output from either. Only the `--broll` colour token remains (in `styles/tokens.css` and its `var(--broll)` uses), as the V2/V3 lane colour.

- [ ] **Step 4: Pass the project's videos from the Player to the stack**

`Player` already takes `sources: SourceView[]` (Task 7: the fold's sources with their files, which App passes as `sources={playable}`) and `stitched: StitchedMedia`. Nothing in its props or in `App.tsx` changes. In `client/src/components/Player.tsx`, replace the `<Overlays edits={edits} assets={assets} words={words} playback={playback} />` line with:

```tsx
<Overlays edits={edits} assets={assets} sources={sources} words={words} playback={playback} />
```

Keep it where it is: right after the main `<video ref={stitched.attach} …>`, before the badges, captions and title card. The layers need to paint above the main picture and below those. The frame's box has the canvas's aspect ratio (Task 7), so the 30 % / 4 % placement follows the canvas, as the export's does.

- [ ] **Step 5: Run the tests and watch them pass**

Run: `npx vitest run client/src/components/Overlays.test.tsx client/src/components/Timeline.test.tsx client/src/overlays.test.ts`
Expected: PASS.

- [ ] **Step 6: Verify**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
npx prettier --write client/src/components/Overlays.tsx client/src/components/Overlays.test.tsx client/src/components/Overlays.module.css client/src/components/Player.tsx client/src/components/Timeline.test.tsx client/src/overlays.ts client/src/overlays.test.ts client/src/App.tsx
npx vitest run && npx tsc -p client --noEmit && npm run lint && npm run build -w client
```

Expected: all pass.

- [ ] **Step 7: Live check against Vite at http://localhost:5174**

The dev server may already be running: Vite on `5174`, the Rust server on `5175`. A Rust server started before the server tasks landed runs old code (`cargo run` does not reload), so restart it if in doubt. If it is not running, run `export PATH="$HOME/.cargo/bin:$PATH"; npm run dev &` and wait for `listening on`. Use the browser tools (`mcp__claude-in-chrome__*`). If they are unavailable, say so in the report; never claim a visual check you did not make. The demo account is signed in in the user's Chrome profile. Check both themes.

1. Open a project with at least two sources. If none exists, import two clips at once on the home screen (Task 6).
2. Select a sentence in video 1, then choose Insert ▾ → **Layer…**. The picker shows "This project's videos" above "Uploads". Pick video 2, choose **V3** and **Top right**, tick **Play its sound**, leave it at −6 dB, and click **Add layer**.
   - The V3 lane replaces the thin "+ V3" row.
   - The bar reads "PiP ↗ · <file>" and shows a speaker.
   - The transcript tag after the first word reads "V3 · <file> · PiP ↗" with a speaker.
3. Play across the sentence.
   - A box a third of the frame's width sits in the top-right corner, about 4% in from each edge.
   - Its picture follows the playhead: pause, seek into the middle of the sentence, and it shows the matching moment of video 2.
   - You hear its sound under the main voice, quieter while words are spoken.
4. Select a different, overlapping stretch and add an upload on **V2**, **Full frame**, sound off. While both are visible, the PiP sits above the full-frame layer, and captions or title cards still draw on top.
5. Hover the timeline over the V2 layer's span. The hover preview shows the main track's frame, not the layer's.
6. Double-click the V3 bar. The dialog opens as **Layer** with V3 / Top right / sound on at −6 dB. Change it to **V2**, **Bottom left**, sound off, and click **Save**.
   - The bar moves to the V2 lane and stays selected.
   - The PiP moves to the bottom left and is silent.
   - The V3 lane returns to "+ V3".
7. Click the layer's transcript tag. The V2 bar is outlined and the floating toolbar shows Delete only, below the bar. Press Delete: the layer disappears from the lane, the transcript and the preview. ⌘Z brings it back.
8. Narrow the window below 600 px. Only the Clips lane shows.
9. Take a screenshot of step 4 (both layers visible) for the report.

- [ ] **Step 8: Commit**

```bash
git add -A client/src
git commit -m "client: the Player stacks one video per visible layer, with picture-in-picture corners and layer sound at its level" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 10: Layer export — overlay, picture-in-picture, layer audio

Task 1 left layers rendering as full-frame, silent B-roll. This task stacks V2 under V3 and draws picture-in-picture corners. It also mixes a layer's sound at its level, under the main audio and ducked like music. On the server, a layer can show another source of the project as well as an asset.

**Files:**

- Modify: `engine/src/editlist.rs` (`layer_windows` replaces `broll_windows`)
- Modify: `engine/src/ffmpeg.rs` (layer inputs, overlay and mix; `layer_placement`, `bed`)
- Modify: `server/src/assets.rs` (a test only: it pins, for the export, the layer media resolution Task 3 put in `sources::resolve_layer_media`)

**Interfaces:**

- Consumes:
  - (Task 1) `Edit::Layer { track: u8, start, end, media: String, offset, frame: Frame, audio: Option<f64> }` and `Frame::{Full, PipTopLeft, PipTopRight, PipBottomLeft, PipBottomRight}` (`Copy + PartialEq`)
  - (Task 3) the table `project_sources(project_id, position, media_id, start_at, duration)`, and `assets::asset_files`, which resolves a layer's media through `sources::resolve_layer_media` (a project asset, or any registry media) and music through assets only
  - (Task 5) `SourceInput`, `canvas`, `args_from`/`file`/`single` test helpers
  - (existing) `overlay_windows`, `ranges_of`, `duck_runs`, `duck_expr`, `routes::media_dir`, `routes::read_meta`
- Produces:

  ```rust
  // engine/src/editlist.rs
  /// Per segment, every layer that intersects it: V2 before V3, then by start.
  pub fn layer_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<Window>>;
  // engine/src/ffmpeg.rs
  /// Picture-in-picture width as a fraction of the canvas width.
  pub const PIP_WIDTH: f64 = 0.3;
  /// Picture-in-picture inset from its corner, as a fraction of each canvas side.
  pub const PIP_MARGIN: f64 = 0.04;
  ```

- [ ] **Step 1: Write the failing engine tests**

In `engine/src/editlist.rs` tests, add `Frame` to the `use crate::types::{…}` line. Replace the two `broll_windows(` calls (in the tests near the old lines 964 and 993) with `layer_windows(`. Then add:

```rust
    #[test]
    fn layer_windows_put_v2_under_v3_whatever_the_edit_order() {
        let edits = [
            Edit::Layer {
                track: 3,
                start: 2.0,
                end: 5.0,
                media: "p".into(),
                offset: 0.0,
                frame: Frame::PipTopRight,
                audio: None,
            },
            Edit::Layer {
                track: 2,
                start: 1.0,
                end: 6.0,
                media: "b".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
        ];
        let tl = timeline(10.0, &edits);
        let order: Vec<usize> = layer_windows(&tl, &edits)[0].iter().map(|w| w.index).collect();
        assert_eq!(order, vec![1, 0]);
    }
```

In `engine/src/ffmpeg.rs` tests, add `Frame` to `use crate::types::{CaptionPos, TitleStyle};`, then append:

```rust
    fn layer(
        track: u8,
        start: f64,
        end: f64,
        media: &str,
        offset: f64,
        frame: Frame,
        audio: Option<f64>,
    ) -> Edit {
        Edit::Layer {
            track,
            start,
            end,
            media: media.into(),
            offset,
            frame,
            audio,
        }
    }

    /// The files given to `-i`, in argv order.
    fn inputs(args: &[String]) -> Vec<&str> {
        args.iter()
            .enumerate()
            .filter(|(i, a)| *a == "-i" && *i + 1 < args.len())
            .map(|(i, _)| args[i + 1].as_str())
            .collect()
    }

    #[test]
    fn v2_full_frame_then_v3_pip_with_sound_are_stacked_and_mixed() {
        // V3 is listed first on purpose: stacking follows the track, not the log.
        let edits = [
            layer(3, 2.0, 5.0, "p1", 1.0, Frame::PipTopRight, Some(-6.0)),
            layer(2, 1.0, 6.0, "b1", 0.0, Frame::Full, None),
        ];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4"), ("p1", "/assets/p1.mp4")]);
            o.words = leak([word(2.5, 3.0)]);
        })
        .unwrap();
        assert_eq!(inputs(&args), ["in.mp4", "/assets/b1.mp4", "/assets/p1.mp4"]);
        assert_eq!(
            filter_complex(&args),
            format!(
                "[1:v]trim=start=0:end=5,setpts=PTS-STARTPTS+1/TB,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1[v0b0];\
                 [0:v]trim=start=0:end=10,setpts=PTS-STARTPTS,setsar=1[v0bo0];\
                 [2:v]trim=start=1:end=4,setpts=PTS-STARTPTS+2/TB,scale=384:-2,setsar=1[v0b1];\
                 [v0bo0][v0b0]overlay=x=0:y=0:eof_action=pass:enable='between(t,1,6)'[v0bo1];\
                 [v0bo1][v0b1]overlay=x=main_w-overlay_w-51:y=29:eof_action=pass:enable='between(t,2,5)'[v0];\
                 [0:a]atrim=start=0:end=10,asetpts=PTS-STARTPTS,{AN}[a0m];\
                 [2:a]atrim=start=1:end=4,asetpts=PTS-STARTPTS,{AN},adelay=2000:all=1,volume=-6dB,volume=volume='1-0.749*(max(0,min(1,min((t-2.38)/0.12,(3.12-t)/0.12))))':eval=frame[a0l1];\
                 [a0m][a0l1]amix=inputs=2:normalize=0:duration=first[a0];\
                 [v0][a0]concat=n=1:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn a_sounding_layer_mixes_beside_music() {
        let edits = [
            Edit::Audio {
                start: 0.0,
                end: 10.0,
                media: "m1".into(),
                offset: 0.0,
                gain: -12.0,
                duck: false,
            },
            layer(2, 4.0, 6.0, "b1", 0.0, Frame::Full, Some(3.0)),
        ];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("m1", "/assets/m1.mp3"), ("b1", "/assets/b1.mp4")]);
        });
        // Inputs: source 0, layer 1, music 2.
        assert!(g.contains(&format!("[1:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN},adelay=4000:all=1,volume=3dB[a0l0];")), "{g}");
        assert!(g.contains("[a0m][a0x0][a0l0]amix=inputs=3:normalize=0:duration=first[a0];"), "{g}");
    }

    #[test]
    fn audio_only_export_mixes_a_sounding_layer_and_skips_muted_ones() {
        let edits = [
            layer(2, 1.0, 3.0, "b1", 0.0, Frame::Full, None),
            layer(3, 4.0, 6.0, "p1", 0.0, Frame::PipBottomLeft, Some(0.0)),
        ];
        let args = args_with(&edits, MediaKind::Audio, OutputFormat::Mp3, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4"), ("p1", "/assets/p1.mp4")]);
        })
        .unwrap();
        assert_eq!(inputs(&args), ["in.mp3", "/assets/p1.mp4"]);
        let g = filter_complex(&args);
        assert!(!g.contains("overlay"), "{g}");
        assert!(g.contains(&format!("[1:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN},adelay=4000:all=1,volume=0dB[a0l1];")), "{g}");
        assert!(g.contains("[a0m][a0l1]amix=inputs=2:normalize=0:duration=first[a0];"), "{g}");
    }

    #[test]
    fn pip_sits_four_percent_in_from_its_corner_at_thirty_percent_width() {
        let c = VideoInfo {
            width: 1920,
            height: 1080,
            fps: 30.0,
        };
        let s = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
        assert_eq!(
            layer_placement(Frame::Full, c),
            s("scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2", "0", "0")
        );
        assert_eq!(layer_placement(Frame::PipTopLeft, c), s("scale=576:-2", "77", "43"));
        assert_eq!(
            layer_placement(Frame::PipTopRight, c),
            s("scale=576:-2", "main_w-overlay_w-77", "43")
        );
        assert_eq!(
            layer_placement(Frame::PipBottomLeft, c),
            s("scale=576:-2", "77", "main_h-overlay_h-43")
        );
        assert_eq!(
            layer_placement(Frame::PipBottomRight, c),
            s("scale=576:-2", "main_w-overlay_w-77", "main_h-overlay_h-43")
        );
    }

    #[test]
    fn a_layer_on_a_later_piece_is_numbered_after_the_sources() {
        // Two files: layer inputs start at 2, and the window is relative to its piece.
        let sources = [
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(HD)),
            file("m1", "b.mp4", 4.0, 6.0, MediaKind::Video, Some(HD)),
        ];
        let edits = [layer(3, 5.0, 7.0, "m0", 1.0, Frame::PipBottomRight, None)];
        let args = args_from(&sources, &edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.splits = &[4.0];
            o.assets = assets(&[("m0", "/data/m0/source.mp4")]);
        })
        .unwrap();
        assert_eq!(inputs(&args), ["a.mp4", "b.mp4", "/data/m0/source.mp4"]);
        let g = filter_complex(&args);
        assert!(g.contains("[2:v]trim=start=1:end=3,setpts=PTS-STARTPTS+1/TB,scale=384:-2,setsar=1[v1b0];"), "{g}");
        assert!(g.contains("[v1bo0][v1b0]overlay=x=main_w-overlay_w-51:y=main_h-overlay_h-29:eof_action=pass:enable='between(t,1,3)'[v1];"), "{g}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p engine layer
```

Expected: FAIL to compile:

```
error[E0425]: cannot find function `layer_windows` in this scope
error[E0425]: cannot find function `layer_placement` in this scope
```

- [ ] **Step 3: Implement layer windows, stacking, picture-in-picture and layer audio**

In `engine/src/editlist.rs`, replace `broll_windows` (after Task 1 it selects `Edit::Layer`) with:

```rust
/// Per segment, every layer that intersects it, lower tracks first and,
/// within a track, in window order, so overlaying in this order stacks V3
/// over V2. Like captions, a layer is not drawn on a title card.
pub fn layer_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<Window>> {
    let track = |w: &Window| match &edits[w.index] {
        Edit::Layer { track, .. } => *track,
        _ => 0,
    };
    let mut windows = overlay_windows(
        segments,
        &ranges_of(edits, |e| matches!(e, Edit::Layer { .. })),
    );
    for ws in &mut windows {
        ws.sort_by(|a, b| track(a).cmp(&track(b)).then(a.start.total_cmp(&b.start)));
    }
    windows
}
```

Check with `grep -rn broll_windows engine server`, which must print nothing.

In `engine/src/ffmpeg.rs`, replace `broll_windows` with `layer_windows` in the `crate::editlist` import, and add `Frame` to the `crate::types` import. Below `AUDIO_NORMALIZE`, add:

```rust
/// Picture-in-picture width as a fraction of the canvas width.
pub const PIP_WIDTH: f64 = 0.3;
/// Picture-in-picture inset from its corner, as a fraction of each canvas side.
pub const PIP_MARGIN: f64 = 0.04;
```

Replace the B-roll input block, which runs from `let brolls = broll_windows(&segments, edits);` through the `if render_video { for (i, ws) in brolls… }` loop that fills `broll_input`. Keep `let audios = audio_windows(…)` and the `asset_path` closure between them as they are:

```rust
    let layers = layer_windows(&segments, edits);
    let audios = audio_windows(&segments, edits);
    let asset_path = |id: &str| {
        opts.assets
            .get(id)
            .ok_or_else(|| ExportError::MissingAsset(id.to_owned()))
    };
    // A layer opens its file when its picture is drawn or its sound is mixed;
    // a muted layer in an audio-only render needs nothing.
    let mut layer_input: Vec<Vec<Option<usize>>> = vec![Vec::new(); segments.len()];
    for (i, ws) in layers.iter().enumerate() {
        for w in ws {
            let Edit::Layer { media, audio, .. } = &edits[w.index] else {
                continue;
            };
            let opened = if render_video || audio.is_some() {
                args.push("-i".into());
                args.push(asset_path(media)?.to_string_lossy().into_owned());
                next_input += 1;
                Some(next_input - 1)
            } else {
                None
            };
            layer_input[i].push(opened);
        }
    }
```

Inside `if let Some(base) = v { … }`, replace the `for (n, w) in brolls[i].iter().enumerate() { … }` loop with:

```rust
            for (n, w) in layers[i].iter().enumerate() {
                let (Edit::Layer { start, offset, frame, .. }, Some(input)) =
                    (&edits[w.index], layer_input[i][n])
                else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let (scale, x, y) = layer_placement(*frame, video_info);
                let _ = write!(
                    graph,
                    "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS+{}/TB,{scale},setsar=1[v{i}b{n}];",
                    fmt(from),
                    fmt(from + (w.end - w.start)),
                    fmt(w.start)
                );
                let _ = write!(graph, "{chain}[v{i}bo{n}];");
                chain = format!(
                    "[v{i}bo{n}][v{i}b{n}]overlay=x={x}:y={y}:eof_action=pass:enable='between(t,{},{})'",
                    fmt(w.start),
                    fmt(w.end)
                );
            }
```

Replace the `let mut mixed = if audios[i].is_empty() { a } else { … };` statement with:

```rust
        // Music beds and layers with sound mix under the piece's own audio.
        let sounding: Vec<(usize, &Window, f64, usize)> = layers[i]
            .iter()
            .enumerate()
            .filter_map(|(n, w)| match &edits[w.index] {
                Edit::Layer {
                    audio: Some(db), ..
                } => Some((n, w, *db, layer_input[i][n]?)),
                _ => None,
            })
            .collect();
        let mut mixed = if audios[i].is_empty() && sounding.is_empty() {
            a
        } else {
            let _ = write!(graph, "{a}[a{i}m];");
            let mut labels = format!("[a{i}m]");
            for (n, w) in audios[i].iter().enumerate() {
                let Edit::Audio {
                    start,
                    offset,
                    gain,
                    duck,
                    ..
                } = &edits[w.index]
                else {
                    continue;
                };
                let runs = if *duck {
                    duck_runs(seg, w, opts.words)
                } else {
                    Vec::new()
                };
                let from = offset + (w.source_start - start);
                let _ = write!(
                    graph,
                    "{}[a{i}x{n}];",
                    bed(audio_input[i][n], from, w, *gain, &runs)
                );
                let _ = write!(labels, "[a{i}x{n}]");
            }
            // A layer has no duck switch: its sound always dips under speech.
            for (n, w, db, input) in &sounding {
                let Edit::Layer { start, offset, .. } = &edits[w.index] else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let runs = duck_runs(seg, w, opts.words);
                let _ = write!(graph, "{}[a{i}l{n}];", bed(*input, from, w, *db, &runs));
                let _ = write!(labels, "[a{i}l{n}]");
            }
            format!(
                "{labels}amix=inputs={}:normalize=0:duration=first",
                audios[i].len() + sounding.len() + 1
            )
        };
```

Add these helpers next to `overdub_audio`:

```rust
/// An inserted sound: `w`'s length of `input` from `from`, placed at
/// `w.start` in the piece, at `gain` dB, dipped under `runs` of speech.
fn bed(input: usize, from: f64, w: &Window, gain: f64, runs: &[Range]) -> String {
    let mut m = format!(
        "[{input}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE},adelay={}:all=1,volume={}dB",
        fmt(from),
        fmt(from + (w.end - w.start)),
        (w.start * 1000.0).round() as i64,
        fmt(gain)
    );
    if !runs.is_empty() {
        let _ = write!(m, ",volume=volume='{}':eval=frame", duck_expr(runs));
    }
    m
}

/// A layer's scale filter and its `overlay` x and y on the canvas. Full frame
/// is fitted and letterboxed; picture-in-picture is `PIP_WIDTH` of the canvas
/// wide and `PIP_MARGIN` of each side in from its corner.
fn layer_placement(frame: Frame, canvas: VideoInfo) -> (String, String, String) {
    let (w, h) = (canvas.width, canvas.height);
    let pip = || format!("scale={}:-2", even(f64::from(w) * PIP_WIDTH));
    let mx = (f64::from(w) * PIP_MARGIN).round() as u32;
    let my = (f64::from(h) * PIP_MARGIN).round() as u32;
    let right = format!("main_w-overlay_w-{mx}");
    let bottom = format!("main_h-overlay_h-{my}");
    match frame {
        Frame::Full => (
            format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"),
            "0".into(),
            "0".into(),
        ),
        Frame::PipTopLeft => (pip(), mx.to_string(), my.to_string()),
        Frame::PipTopRight => (pip(), right, my.to_string()),
        Frame::PipBottomLeft => (pip(), mx.to_string(), bottom),
        Frame::PipBottomRight => (pip(), right, bottom),
    }
}

/// The nearest even whole number; yuv420p needs even sizes.
fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32) * 2
}
```

`broll_input` and `brolls` no longer exist. Remove anything left that refers to them, and let `cargo clippy` confirm.

- [ ] **Step 4: Run the engine tests to verify they pass**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p engine
```

Expected: PASS, including:

```
test editlist::tests::layer_windows_put_v2_under_v3_whatever_the_edit_order ... ok
test ffmpeg::tests::v2_full_frame_then_v3_pip_with_sound_are_stacked_and_mixed ... ok
test ffmpeg::tests::a_sounding_layer_mixes_beside_music ... ok
test ffmpeg::tests::audio_only_export_mixes_a_sounding_layer_and_skips_muted_ones ... ok
test ffmpeg::tests::pip_sits_four_percent_in_from_its_corner_at_thirty_percent_width ... ok
test ffmpeg::tests::a_layer_on_a_later_piece_is_numbered_after_the_sources ... ok
```

The B-roll tests Task 1 moved onto `Edit::Layer { track: 2, frame: Frame::Full, audio: None }` still pass unchanged: the full-frame chain and the `v{i}b{n}`/`v{i}bo{n}` labels are identical. The music tests still pass unchanged too, because `bed()` writes the same string the old inline code did.

- [ ] **Step 5: Write the server test that pins layer media resolution for export**

Task 3 already resolves a layer's media in `asset_files` through `sources::resolve_layer_media`, the one resolver op validation also uses. This test holds the export to it: a layer over another source renders, an asset still resolves, a stranger's media is refused, and music stays asset-only. In `server/src/assets.rs`, inside `mod tests`, add:

```rust
    #[tokio::test]
    async fn a_layer_may_show_another_source_of_the_project() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let second = crate::projects::test_support::seed_media(&state, 5.0).await;
        let dir = state.config.data_dir.join(&second);
        tokio::fs::write(dir.join("source.mp4"), b"").await.unwrap();
        sqlx::query(
            "INSERT INTO project_sources (project_id, position, media_id, start_at, duration) VALUES (?, 1, ?, 10.0, 5.0)",
        )
        .bind(&project.id)
        .bind(&second)
        .execute(&state.db)
        .await
        .unwrap();
        let layer = |media: &str| Edit::Layer {
            track: 3,
            start: 1.0,
            end: 2.0,
            media: media.into(),
            offset: 0.0,
            frame: engine::Frame::PipTopRight,
            audio: Some(-6.0),
        };

        let files = asset_files(&state, &project, &[layer(&second)]).await.unwrap();
        assert_eq!(files[&second], dir.join("source.mp4"));

        // An asset still resolves to its own file.
        let clip = seed_asset(&state, &project, MediaKind::Video, 4.0).await;
        let files = asset_files(&state, &project, &[layer(&clip.id)]).await.unwrap();
        assert!(files[&clip.id].ends_with(format!("{}.mp4", clip.id)));

        // Media that is not in this project is refused.
        let stranger = crate::projects::test_support::seed_media(&state, 5.0).await;
        let err = asset_files(&state, &project, &[layer(&stranger)])
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("does not belong"), "{err:?}");

        // Music may not play a source: it stays asset-only.
        let music = Edit::Audio {
            start: 0.0,
            end: 1.0,
            media: second.clone(),
            offset: 0.0,
            gain: 0.0,
            duck: true,
        };
        assert!(asset_files(&state, &project, &[music]).await.is_err());
    }
```

- [ ] **Step 6: Run it to verify it passes**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p server a_layer_may_show_another_source_of_the_project
```

Expected: PASS on arrival. Task 3's `asset_files` already resolves layers through `sources::resolve_layer_media`. If it fails, fix that resolver. Do not add a second lookup to `asset_files`: validation and export must keep sharing one rule.

- [ ] **Step 7: No second resolver**

Leave `asset_files` as Task 3 wrote it. Check that the registry is read in one place only:

```bash
grep -n "project_sources" server/src/assets.rs server/src/routes.rs
```

Expected: no output outside tests. Every registry lookup goes through `sources::in_registry` / `sources::resolve_layer_media`.

- [ ] **Step 8: Run the server tests and the whole suite**

Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p server asset
npm test && npm run lint
```

Expected: PASS, with `a_layer_may_show_another_source_of_the_project ... ok`, every existing asset test still ok, and lint clean.

- [ ] **Step 9: Commit**

```bash
git add engine/src/editlist.rs engine/src/ffmpeg.rs server/src/assets.rs
git commit -m "export: stack V2 under V3, draw picture-in-picture corners and mix layer sound ducked under speech" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 11: MCP tools and README

Agents can build a multi-source project and stack layers.

- `add_source` appends a video.
- `add_layer` and `set_layer` put pictures on V2/V3.
- `add_broll` becomes an alias for a muted, full-frame V2 layer.
- `list_clips` names each clip's source.
- The agent's transcript refreshes when sources change.

**Files:**

- Modify: `server/src/mcp.rs`, `server/src/sources.rs` (`source_media`, which promotes an asset to a media item),
  `README.md`

**Interfaces:**

- Consumes (Task 1): `engine::{all_sources, stitched_duration, locate, Frame, Edit::Layer,
Op::AddLayer, Op::SetLayer}`.
- Consumes (Tasks 3–4): `sources::{add_source, timeline, views, transcript_status,
project_transcript, in_registry, TranscriptStatus}`, `assets::{find, list_for, Asset}`.
- Produces (`server/src/sources.rs`):
  `pub async fn source_media(state: &AppState, project: &Project, media: &str) -> AppResult<Meta>`.
- Produces (MCP tools):
  - `add_source(media)`
  - `add_layer(from, to, media, track?, frame?, audio?, offset?)`
  - `set_layer(track, start, to_track?, frame?, audio?)`
  - `add_broll` becomes an alias for `add_layer`
  - `list_clips` items gain `"source": <index>`
  - `open_project` gains `"sources": SourceView[]` and `"layers": Edit::Layer[]`

- [ ] **Step 1: Write the failing tests**

In `server/src/mcp.rs`, `mod tests`, replace the tool list in `every_read_only_tool_is_advertised`:

```rust
            [
                "add_audio",
                "add_broll",
                "add_caption",
                "add_layer",
                "add_source",
                "add_title",
                "cut",
                "export",
                "find",
                "get_transcript",
                "list_assets",
                "list_clips",
                "list_projects",
                "look_at",
                "move_clip",
                "open_project",
                "overdub",
                "redo",
                "remove_fillers",
                "set_layer",
                "set_transition",
                "split",
                "tighten_pauses",
                "undo"
            ]
```

In `add_broll_and_add_audio_take_project_assets`, replace the B-roll assertion with:

```rust
        assert!(matches!(
            &doc.edits[0],
            Edit::Layer { track: 2, frame: engine::Frame::Full, audio: None, offset, .. } if *offset == 1.0
        ));
```

Add:

```rust
    #[tokio::test]
    async fn add_source_appends_an_asset_or_a_source_and_list_clips_names_the_source() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let clip = crate::assets::test_support::seed_asset(
            &state,
            &project,
            engine::MediaKind::Video,
            5.0,
        )
        .await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();

        let out = session.tool_add_source(&identity, &clip.id).await.unwrap();
        assert_eq!(out["source"]["index"], 1);
        assert_eq!(out["source"]["offset"], 10.0);
        assert_eq!(out["duration"], 15.0);
        let clips = session.tool_list_clips(&identity).await.unwrap();
        let clips = clips.as_array().unwrap();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0]["source"], 0);
        assert_eq!(clips[1]["source"], 1);
        assert_eq!(clips[1]["start"], 10.0);

        // The same asset again shares its media directory.
        let again = session.tool_add_source(&identity, &clip.id).await.unwrap();
        assert_eq!(again["source"]["mediaId"], out["source"]["mediaId"]);
        assert_eq!(again["source"]["offset"], 15.0);

        assert!(session.tool_add_source(&identity, "nope").await.is_err());
    }

    #[tokio::test]
    async fn add_layer_and_set_layer_stack_a_source_over_the_first() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let second = crate::sources::test_support::seed_source(&state, 4.0, &["d", "e"]).await;
        crate::sources::add_source(&state, &project, &owner, &second)
            .await
            .unwrap();
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        let opened = session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        assert_eq!(opened["sources"].as_array().unwrap().len(), 2);

        let out = session
            .tool_add_layer(
                &identity,
                AddLayerArgs {
                    from: 0,
                    to: 1,
                    media: second.id.clone(),
                    track: Some(3),
                    frame: Some("pipTopRight".into()),
                    audio: Some(-6.0),
                    offset: Some(0.5),
                },
            )
            .await
            .unwrap();
        assert_eq!(out["touched"].as_array().unwrap().len(), 2);
        assert_eq!(out["layers"][0]["track"], 3);
        let (_, doc) = ops::load_doc(&state, &project.id).await.unwrap();
        assert!(matches!(
            &doc.edits[0],
            Edit::Layer { track: 3, start, end, frame: engine::Frame::PipTopRight, audio: Some(a), offset, .. }
                if *start == 0.0 && *end == 1.0 && *a == -6.0 && *offset == 0.5
        ));

        session
            .tool_set_layer(
                &identity,
                SetLayerArgs {
                    track: 3,
                    start: 0.0,
                    to_track: Some(2),
                    frame: None,
                    audio: None,
                },
            )
            .await
            .unwrap();
        let (_, doc) = ops::load_doc(&state, &project.id).await.unwrap();
        assert!(matches!(
            &doc.edits[0],
            Edit::Layer { track: 2, frame: engine::Frame::PipTopRight, audio: None, .. }
        ));

        let bad_track = AddLayerArgs {
            from: 0,
            to: 0,
            media: second.id.clone(),
            track: Some(4),
            frame: None,
            audio: None,
            offset: None,
        };
        assert!(session.tool_add_layer(&identity, bad_track).await.is_err());
        let bad_frame = AddLayerArgs {
            from: 0,
            to: 0,
            media: second.id.clone(),
            track: None,
            frame: Some("middle".into()),
            audio: None,
            offset: None,
        };
        let err = session
            .tool_add_layer(&identity, bad_frame)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pipTopLeft"), "{err}");
        let missing = SetLayerArgs {
            track: 3,
            start: 0.0,
            to_track: None,
            frame: None,
            audio: None,
        };
        assert!(session.tool_set_layer(&identity, missing).await.is_err());
    }

    #[tokio::test]
    async fn get_transcript_picks_up_a_source_a_peer_added() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let owner = me(&state, &ada).await;
        let identity = identity_for(&state, &ada).await;
        let session = McpSession::new(state.clone());
        session
            .tool_open_project(&identity, &project.id)
            .await
            .unwrap();
        let second = crate::sources::test_support::seed_source(&state, 4.0, &["d"]).await;
        crate::sources::add_source(&state, &project, &owner, &second)
            .await
            .unwrap();
        let transcript = session.tool_get_transcript(&identity).await.unwrap();
        assert_eq!(transcript.as_array().unwrap().len(), 4);
        assert_eq!(transcript[3]["start"], 10.0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server mcp::`

Expected: FAIL to compile, with `no method named `tool_add_source`` and
`cannot find struct `AddLayerArgs``.

- [ ] **Step 3: Implement source resolution and the tools**

In `server/src/sources.rs`, append (the imports from Tasks 3–4 already cover it):

```rust
/// The media an agent means by `media`: a media item already in the
/// project's registry, or a project asset promoted to a media item of its
/// own. A source needs its own directory (transcript cache, `meta.json`,
/// `/data` URL), and assets live inside the first media's, so an asset is
/// hard-linked (copied across filesystems) into one. Its id is derived from
/// the asset's, so adding the same asset twice shares one transcript.
pub async fn source_media(state: &AppState, project: &Project, media: &str) -> AppResult<Meta> {
    if in_registry(&state.db, &project.id, media).await? {
        return read_meta(&state.config.data_dir.join(media)).await;
    }
    let asset = crate::assets::list_for(&state.db, project)
        .await?
        .into_iter()
        .find(|a| a.id == media)
        .ok_or_else(|| {
            AppError::bad_request(format!(
                "{media} is neither an asset from list_assets nor one of this project's videos"
            ))
        })?;
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("type-n-stitch:asset:{}", asset.id).as_bytes(),
    )
    .to_string();
    let dir = state.config.data_dir.join(&id);
    if dir.join("meta.json").is_file() {
        return read_meta(&dir).await;
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .context("creating media dir")?;
    let source_name = format!("source.{}", asset.ext);
    let from = state
        .config
        .data_dir
        .join(&project.media_id)
        .join("assets")
        .join(format!("{}.{}", asset.id, asset.ext));
    if tokio::fs::hard_link(&from, dir.join(&source_name))
        .await
        .is_err()
    {
        tokio::fs::copy(&from, dir.join(&source_name))
            .await
            .context("copying the asset")?;
    }
    let meta = Meta {
        url: format!("/data/{id}/{source_name}"),
        id,
        filename: asset.name,
        ext: asset.ext,
        duration: asset.duration,
        kind: asset.kind,
        // The asset row keeps no frame rate; the export re-probes, as it
        // does for any media stored before dimensions were recorded.
        video: None,
    };
    tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)
        .await
        .context("writing meta.json")?;
    Ok(meta)
}
```

In `server/src/mcp.rs`:

1. Imports: `use engine::types::{CaptionPos, Frame, Range, TitleStyle, Transition, Word};` and
   `use engine::{Edit, Op, Source, EPS};` and
   `use crate::sources::{SourceView, TranscriptStatus};`.

2. `Open` gains two fields after `speakers`:

```rust
    /// The main track as of the last transcript read, for `refresh`.
    sources: Vec<Source>,
    /// How many of those sources had a ready transcript then.
    ready: usize,
```

3. Argument structs, next to `AddBrollArgs`:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddSourceArgs {
    /// An asset id from `list_assets`, or a `mediaId` from `open_project`'s
    /// `sources` to use the same video again.
    pub media: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddLayerArgs {
    /// First word the layer covers, from the transcript's `i`.
    pub from: usize,
    /// Last word it covers, inclusive.
    pub to: usize,
    /// A video asset id from `list_assets`, or a `mediaId` from
    /// `open_project`'s `sources` to show a stretch of another source.
    pub media: String,
    /// 2 or 3; a higher track covers a lower one. 2 when omitted.
    pub track: Option<u8>,
    /// `full`, `pipTopLeft`, `pipTopRight`, `pipBottomLeft` or
    /// `pipBottomRight`; `full` when omitted.
    pub frame: Option<String>,
    /// Mix the layer's own sound in at this level in dB (-30..12); muted
    /// when omitted.
    pub audio: Option<f64>,
    /// Seconds into the media to start from; 0 when omitted.
    pub offset: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetLayerArgs {
    /// The layer's track now, 2 or 3.
    pub track: u8,
    /// The layer's start in seconds, from `layers` in `open_project` or
    /// `add_layer`'s result.
    pub start: f64,
    /// Move it to this track; it stays where it is when omitted.
    pub to_track: Option<u8>,
    /// New framing (see `add_layer`); unchanged when omitted.
    pub frame: Option<String>,
    /// Sound level in dB; muted when omitted.
    pub audio: Option<f64>,
}
```

4. Tool wrappers, inside `#[tool_router] impl McpSession`, after `add_audio`:

```rust
    #[tool(description = "Append a video to the end of the main track: an asset \
                       from `list_assets`, or a source's `mediaId` to use it again. \
                       Its words join the transcript once it is transcribed; call \
                       `get_transcript` for new indices.")]
    async fn add_source(
        &self,
        Parameters(args): Parameters<AddSourceArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_add_source(&identity_of(&ctx)?, &args.media).await)
    }

    #[tool(description = "Show a video on track 2 or 3 over words `from`..`to`: \
                       full frame or a picture-in-picture corner, muted unless \
                       `audio` gives a level in dB. Higher tracks cover lower ones.")]
    async fn add_layer(
        &self,
        Parameters(args): Parameters<AddLayerArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_add_layer(&identity_of(&ctx)?, args).await)
    }

    #[tool(description = "Change the layer that starts at `start` on `track`: move \
                       it to `to_track`, reframe it, or set its sound (omit \
                       `audio` to mute).")]
    async fn set_layer(
        &self,
        Parameters(args): Parameters<SetLayerArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        rendered(self.tool_set_layer(&identity_of(&ctx)?, args).await)
    }
```

Update the `add_broll` and `list_clips` descriptions:

```rust
    #[tool(description = "Show video asset `asset` full frame on track 2 while \
                       words `from`..`to` play, starting `offset` seconds into it, \
                       muted. Same as `add_layer` with its defaults.")]
```

```rust
    #[tool(
        description = "The clips in output order, each with its index, the source \
                       it comes from, start/end, duration and first words. Clips \
                       come from splits, cuts and the joins between sources."
    )]
```

5. Implementations, inside the plain `impl McpSession`:

```rust
    /// Re-read the open project's transcript if its sources changed since the
    /// agent last looked: a video added or undone, or one that finished
    /// transcribing. Otherwise only the fold and a few file checks.
    async fn refresh(&self, open: &mut Open, project: &Project) -> AppResult<()> {
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        let sources = crate::sources::timeline(&self.state, project, &doc).await?;
        let ready = sources
            .iter()
            .filter(|s| {
                crate::sources::transcript_status(&self.state, &s.media)
                    == TranscriptStatus::Ready
            })
            .count();
        if sources == open.sources && ready == open.ready {
            return Ok(());
        }
        let (sources, words, speakers) =
            crate::sources::project_transcript(&self.state, project).await?;
        open.duration = engine::stitched_duration(&sources);
        open.words = words;
        open.speakers = speakers.map(|s| s.words);
        open.sources = sources;
        open.ready = ready;
        Ok(())
    }

    pub(crate) async fn tool_add_source(
        &self,
        identity: &McpIdentity,
        media: &str,
    ) -> AppResult<Value> {
        let agent = self.agent(identity);
        let mut open = agent.lock().await;
        let project = require_open(&open, identity)?.clone();
        require_edit(&open)?;
        let meta = crate::sources::source_media(&self.state, &project, media).await?;
        let source: SourceView =
            crate::sources::add_source(&self.state, &project, &identity.bot, &meta).await?;
        self.refresh(&mut open, &project).await?;
        Ok(json!({
            "source": source,
            "sources": open.sources.len(),
            "duration": open.duration,
            "words": open.words.len(),
        }))
    }

    pub(crate) async fn tool_add_layer(
        &self,
        identity: &McpIdentity,
        args: AddLayerArgs,
    ) -> AppResult<Value> {
        let frame = frame_kind(args.frame.as_deref())?;
        let agent = self.agent(identity);
        let mut out = self
            .edit(&agent, identity, |open| {
                let range = range_of(open, args.from, args.to)?;
                Ok((
                    Some((args.from, args.to)),
                    Op::AddLayer {
                        track: args.track.unwrap_or(2),
                        start: range.start,
                        end: range.end,
                        media: args.media.clone(),
                        offset: args.offset.unwrap_or(0.0),
                        frame,
                        audio: args.audio,
                    },
                ))
            })
            .await?;
        out["layers"] = self.layers(identity).await?;
        Ok(out)
    }

    pub(crate) async fn tool_set_layer(
        &self,
        identity: &McpIdentity,
        args: SetLayerArgs,
    ) -> AppResult<Value> {
        let agent = self.agent(identity);
        let project_id = {
            let open = agent.lock().await;
            require_open(&open, identity)?.id.clone()
        };
        let (_, doc) = ops::load_doc(&self.state, &project_id).await?;
        let current = doc
            .edits
            .iter()
            .find_map(|e| match e {
                Edit::Layer {
                    track, start, frame, ..
                } if *track == args.track && (start - args.start).abs() < EPS => Some(*frame),
                _ => None,
            })
            .ok_or_else(|| {
                AppError::bad_request(format!(
                    "no layer starts at {} on V{}",
                    args.start, args.track
                ))
            })?;
        let frame = match args.frame.as_deref() {
            Some(name) => frame_kind(Some(name))?,
            None => current,
        };
        let op = Op::SetLayer {
            track: args.track,
            start: args.start,
            to_track: args.to_track.unwrap_or(args.track),
            frame,
            audio: args.audio,
        };
        let mut out = self.edit(&agent, identity, |_| Ok((None, op))).await?;
        out["layers"] = self.layers(identity).await?;
        Ok(out)
    }

    /// Every layer in the open project, as the fold has them.
    async fn layers(&self, identity: &McpIdentity) -> AppResult<Value> {
        let agent = self.agent(identity);
        let project_id = {
            let open = agent.lock().await;
            require_open(&open, identity)?.id.clone()
        };
        let (_, doc) = ops::load_doc(&self.state, &project_id).await?;
        Ok(layers_json(&doc))
    }
```

Replace `tool_add_broll`'s body:

```rust
        // The old name for a muted, full-frame layer on V2.
        self.tool_add_layer(
            identity,
            AddLayerArgs {
                from,
                to,
                media: asset.to_owned(),
                track: Some(2),
                frame: None,
                audio: None,
                offset,
            },
        )
        .await
```

6. Replace `tool_open_project`. The transcript is now read through `refresh`, under the guard:

```rust
    pub(crate) async fn tool_open_project(
        &self,
        identity: &McpIdentity,
        id: &str,
    ) -> AppResult<Value> {
        // The same three rejections, in the same words, as `ProjectAccess`.
        let project = find_project(&self.state.db, id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("no project with id {id}")))?;
        let owner_role = member_role(&self.state.db, &project.id, &identity.owner.id)
            .await?
            .ok_or_else(|| AppError::forbidden("you are not a member of this project"))?;
        let role =
            ensure_bot_member(&self.state.db, &project.id, &identity.bot.id, owner_role).await?;

        let agent = self.agent(identity);
        let mut open = agent.lock().await;
        // Leave whatever was open before, so the agent is never two peers.
        open.subscription = None;
        let subscription =
            self.state
                .bus
                .subscribe(&project.id, &open.conn_id, PeerInfo::from(&identity.bot));
        open.subscription = Some(subscription);
        open.project = Some(project.clone());
        open.role = Some(role);
        open.bot = Some(identity.bot.clone());
        // Forget the last project's sources so `refresh` reads this one's.
        open.sources.clear();
        self.refresh(&mut open, &project).await?;
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        let sources = crate::sources::views(&self.state, &open.sources).await?;

        Ok(json!({
            "id": project.id,
            "title": project.title,
            "duration": open.duration,
            "outputDuration": engine::output_duration(&engine::timeline_with(
                open.duration,
                &doc.edits,
                &doc.splits,
                &doc.order,
            )),
            "cuts": doc.edits.iter().filter(|e| matches!(e, engine::Edit::Cut { .. })).count(),
            "overdubs": doc.edits.iter().filter(|e| matches!(e, engine::Edit::Overdub { .. })).count(),
            "speakerNames": doc.speaker_names,
            "transition": doc.transition,
            "role": role,
            "sources": sources,
            "layers": layers_json(&doc),
            "transcript": transcript(&open.words, open.speakers.as_deref(), &doc.edits),
            "clips": Self::clips_json(&open, &doc),
        }))
    }
```

A failed transcript read now leaves the agent subscribed to the new project with no words.
That is the same end state as a later failed tool call, and the next `open_project` or
`get_transcript` retries it.

7. `tool_get_transcript` refreshes first:

```rust
    pub(crate) async fn tool_get_transcript(&self, identity: &McpIdentity) -> AppResult<Value> {
        let agent = self.agent(identity);
        let mut open = agent.lock().await;
        let project = require_open(&open, identity)?.clone();
        self.refresh(&mut open, &project).await?;
        let (_, doc) = ops::load_doc(&self.state, &project.id).await?;
        Ok(serde_json::to_value(transcript(
            &open.words,
            open.speakers.as_deref(),
            &doc.edits,
        ))?)
    }
```

8. `clips_json` names each clip's source and uses the fold's current sources:

```rust
    fn clips_json(open: &Open, doc: &engine::ProjectDoc) -> Vec<Value> {
        let sources = open
            .sources
            .first()
            .map(|first| engine::all_sources(first.clone(), &doc.sources))
            .unwrap_or_default();
        let duration = engine::stitched_duration(&sources);
        engine::ordered_pieces(duration, &doc.edits, &doc.splits, &doc.order)
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let words: Vec<&str> = open
                    .words
                    .iter()
                    .filter(|w| w.start >= p.start - engine::EPS && w.start < p.end)
                    .map(|w| w.text.as_str())
                    .take(6)
                    .collect();
                let source = engine::locate(&sources, p.start).map(|(k, _)| k);
                json!({
                    "i": i,
                    "source": source,
                    "start": p.start,
                    "end": p.end,
                    "duration": p.len(),
                    "words": words.join(" "),
                })
            })
            .collect()
    }
```

In `tool_move_clip`, take the first source under the guard instead of `open.duration`, and
measure the pieces against the current fold's stitched duration:

```rust
        let (project_id, first) = {
            let open = agent.lock().await;
            let project = require_open(&open, identity)?;
            // `refresh` always leaves the first source once a project is open.
            (project.id.clone(), open.sources.first().cloned())
        };
        let (_, doc) = ops::load_doc(&self.state, &project_id).await?;
        let sources = first
            .map(|f| engine::all_sources(f, &doc.sources))
            .unwrap_or_default();
        let duration = engine::stitched_duration(&sources);
        let pieces = engine::ordered_pieces(duration, &doc.edits, &doc.splits, &doc.order);
```

9. Free functions next to `transition_kind`:

```rust
fn frame_kind(name: Option<&str>) -> AppResult<Frame> {
    match name.unwrap_or("full") {
        "full" => Ok(Frame::Full),
        "pipTopLeft" => Ok(Frame::PipTopLeft),
        "pipTopRight" => Ok(Frame::PipTopRight),
        "pipBottomLeft" => Ok(Frame::PipBottomLeft),
        "pipBottomRight" => Ok(Frame::PipBottomRight),
        other => Err(AppError::bad_request(format!(
            "unknown frame {other}: use full, pipTopLeft, pipTopRight, pipBottomLeft or pipBottomRight"
        ))),
    }
}

/// The fold's layers, each as its `Edit::Layer` JSON.
fn layers_json(doc: &engine::ProjectDoc) -> Value {
    Value::Array(
        doc.edits
            .iter()
            .filter(|e| matches!(e, Edit::Layer { .. }))
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect(),
    )
}
```

10. `get_info` instructions: replace the sentence starting "`list_clips`, `split` and
    `move_clip` reorder the edit;" with:

```
             `list_clips`, `split` and `move_clip` reorder the edit; each clip \
             names the source it comes from. `add_source` appends another video \
             to the end of the main track. `list_assets`, `add_layer`, `set_layer` \
             and `add_audio` lay a picture or music over a range of words \
             (`add_broll` is `add_layer` on track 2, full frame, muted; uploading \
             assets happens in the browser).
```

    Add `add_source`, `add_layer` and `set_layer` to the list of editing tools in the sentence
    before it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p server mcp::`

Expected: PASS, including

- `every_read_only_tool_is_advertised`
- `add_broll_and_add_audio_take_project_assets` (now a V2 layer)
- `add_source_appends_an_asset_or_a_source_and_list_clips_names_the_source`
- `add_layer_and_set_layer_stack_a_source_over_the_first`
- `get_transcript_picks_up_a_source_a_peer_added`
- `split_after_a_word_then_move_reorders_the_clips` (unchanged)

- [ ] **Step 5: Update the README's Agents table**

In `README.md`, under "## Agents", change the `open_project`, `list_clips`, `list_assets` and
`add_broll` rows, and add three rows after `add_audio`:

```markdown
| `open_project(project_id)` | Opens a project and joins it as a visible peer; returns duration, edit counts, sources, layers and the transcript. |
| `list_clips()` | The clips in output order, each with its index, the source it comes from, start/end, duration and first words. |
| `list_assets()` | Videos and music files uploaded to this project, with ids for `add_layer`, `add_source` and `add_audio`. |
| `add_broll(from, to, asset, offset?)` | Same as `add_layer` on track 2, full frame, muted: shows video asset `asset` while words `from`..`to` play. |
| `add_source(media)` | Appends a video to the end of the main track: an asset from `list_assets`, or a source used again. Its words join the transcript once it is transcribed. |
| `add_layer(from, to, media, track?, frame?, audio?, offset?)` | Shows a video on track 2 or 3 over words `from`..`to`, full frame or as a picture-in-picture corner, muted unless `audio` gives a level in dB. |
| `set_layer(track, start, to_track?, frame?, audio?)` | Moves the layer starting at `start` on `track` to another track, reframes it, or sets its sound (omit `audio` to mute). |
```

Then realign the table: `npx prettier --write README.md`.

Add one paragraph after the paragraph that ends "calling `export` again picks up the same
job.":

```markdown
A project can hold up to 20 videos laid end to end on its main track; the transcript reads
straight through them, and word indices count across every video. A video an agent adds with
`add_source` joins the transcript once it has been transcribed, so call `get_transcript` again
before using indices past the old end. Layers on tracks 2 and 3 cover the tracks below them;
a picture-in-picture layer sits in a corner at 30% of the width.
```

- [ ] **Step 6: Verify and commit**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test -p server && npx prettier --check README.md
git add server/src/mcp.rs server/src/sources.rs README.md
git commit -m "mcp: add_source, add_layer and set_layer; add_broll becomes a V2 layer; clips name their source" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

---

### Task 12: End-to-end check

Run the spec's five live steps against the real app, on a fresh project made from three generated clips, one of them a different resolution and frame rate. Then run the full checks, and commit the README screenshot and the README's remaining B-roll wording, because the timeline and the dialogs changed materially.

**Files:**

- Create (scratchpad only, not committed): three test clips and the downloaded export
- Modify: `docs/screenshot.png`, `README.md` (screenshot alt text and the editing notes that still say B-roll)

**Interfaces:**

- Consumes: everything from Tasks 1–11. In particular: the `/sources` route and polling (Tasks 3, 4 and 7), the stitched clock (Task 7), the Range tool across a join (Tasks 2 and 7), the Layer dialog (Task 8), the layer preview (Task 9), and the export (Tasks 5 and 10).
- Produces: nothing new in code. It produces evidence: ffprobe output and screenshots.

Do not open, edit or export any existing project, including the user's TEDx one. Everything below happens in one new project whose name starts with `e2e-`. Use the browser tools (`mcp__claude-in-chrome__*`). If they are unavailable, say so plainly in the report and do not claim any visual check.

- [ ] **Step 1: Make three test clips in the scratchpad**

Each clip gets a synthetic test picture (so the export shows which file is which) and a stretch of `samples/sample.mp4`'s synthetic speech (so whisper produces words and the Range tool has word starts to snap to). Clip 3 is 640×480 at 25 fps; the other two are 1280×720 at 30 fps. `S` is the working directory for the clips and the export: export `SCRATCHPAD` as your session's scratchpad directory first if you have one, and set `S` the same way again in every later shell.

```bash
S="${SCRATCHPAD:-${TMPDIR:-/tmp}}/tns-e2e"
mkdir -p "$S/e2e"
cd /Users/jonyen/Projects/type-n-stitch
test -f samples/sample.mp4 || samples/make-sample.sh
ffmpeg -y -loglevel error \
  -f lavfi -i "testsrc2=size=1280x720:rate=30:duration=10" \
  -ss 0 -t 10 -i samples/sample.mp4 \
  -map 0:v -map 1:a -c:v libx264 -pix_fmt yuv420p -c:a aac -ar 48000 -ac 2 -shortest \
  "$S/e2e/e2e-1.mp4"
ffmpeg -y -loglevel error \
  -f lavfi -i "smptehdbars=size=1280x720:rate=30:duration=8" \
  -ss 10 -t 8 -i samples/sample.mp4 \
  -map 0:v -map 1:a -c:v libx264 -pix_fmt yuv420p -c:a aac -ar 48000 -ac 2 -shortest \
  "$S/e2e/e2e-2.mp4"
ffmpeg -y -loglevel error \
  -f lavfi -i "testsrc=size=640x480:rate=25:duration=6" \
  -ss 14 -t 6 -i samples/sample.mp4 \
  -map 0:v -map 1:a -c:v libx264 -pix_fmt yuv420p -c:a aac -ar 44100 -ac 1 -shortest \
  "$S/e2e/e2e-3.mp4"
for f in "$S"/e2e/e2e-*.mp4; do
  printf '%s ' "$(basename "$f")"
  ffprobe -v error -select_streams v:0 -show_entries stream=width,height,r_frame_rate:format=duration -of csv=p=0 "$f" | tr '\n' ' '
  echo
done
```

Expected: three lines, `e2e-1.mp4 1280,720,30/1 10.0…`, `e2e-2.mp4 1280,720,30/1 8.0…` and `e2e-3.mp4 640,480,25/1 6.0…`. Write down the three durations exactly as printed: they are D1, D2 and D3 below. Clip 3's mono 44.1 kHz audio is deliberate, so the export's 48 kHz stereo normalisation is exercised too.

If `samples/sample.mp4` cannot be made (it needs macOS `say`), take the audio from any clip in `samples/library/` that has speech instead, for example `-ss 5 -t 10 -i samples/library/conversation-interview.mp4`. Keep the same pictures, sizes and rates.

- [ ] **Step 2: Start the dev servers on the new code**

The repo's dev script is `npm run dev` (`concurrently "cargo run -p server" "npm:dev -w client"`): Vite on `http://localhost:5174`, proxying to the Rust server on `5175`.

```bash
lsof -nP -iTCP:5174 -iTCP:5175 -sTCP:LISTEN
```

A Rust server that was started before Tasks 1–11 landed runs old code and an old schema: `cargo run` does not reload. If anything is listening, stop that dev process (the `concurrently` job, or the PIDs `lsof` printed) and start it again:

```bash
cd /Users/jonyen/Projects/type-n-stitch
export PATH="$HOME/.cargo/bin:$PATH"
npm run dev > "$S/e2e/dev.log" 2>&1 &
until grep -q "listening on" "$S/e2e/dev.log"; do sleep 1; done
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:5174/
```

Expected: `listening on` in the log (the `0005_sources` migration runs on this start) and `200` from Vite. The demo account is already signed in in the user's Chrome profile.

- [ ] **Step 3: Live step 1 — import all three at once**

Open `http://localhost:5174` on the home screen.

1. Upload `e2e-3.mp4`, `e2e-1.mp4` and `e2e-2.mp4` together with `mcp__claude-in-chrome__file_upload` on `[data-testid="media-input"]`.
2. The staged list reads `e2e-1.mp4`, `e2e-2.mp4`, `e2e-3.mp4`, in natural name order. Click **Import 3 files**. Each row shows a progress bar, then **Added**, one file at a time.
3. The editor opens on a project titled after `e2e-1`. Check that:
   - video 1's words show at once, and "Transcribing video 2…" and "Transcribing video 3…" blocks are greyed and pulsing. Each block is replaced by words without a reload, within about 2 s of its transcript finishing (the poll).
   - the Clips lane shows badges `1`, `2` and `3` with a divider at each join.
   - the frame has video 1's 16:9 shape, and when playback reaches video 3, its 4:3 picture sits pillarboxed inside that frame.
   - Space at 0:00 plays straight through video 1 into video 2, with the transport staying on Pause.

- [ ] **Step 4: Live step 2 — reorder across sources**

With the Select tool (V), drag the clip badged `3` on the Clips lane to before the clip badged `1`. The badges read `3`, `1`, `2`, the transcript reorders to match, and playing from 0:00 shows clip 3's `testsrc` picture first, then video 1's `testsrc2`, then video 2's colour bars. Leave the new order in place.

- [ ] **Step 5: Live step 3 — a Range cut across a join**

Press X (Range). In the transcript, drag from the second-to-last word of video 1 to the second word of video 2, then release.

- The struck-through words span both videos, and the Clips lane shows clip `1` ending earlier and clip `2` starting later. The divider between them stays: a join is never removed.
- Play from a second before the cut. The cut is skipped, and playback carries on in video 2 without stopping.
- Undo (⌘Z, or the top bar's Undo) restores the words in one step, and the top bar's Redo cuts them again. Leave the cut in place.

- [ ] **Step 6: Live step 4 — a PiP layer with sound from video 3 on V3 over video 1**

1. Press V. In video 1's transcript, select a run of words near its start, well before the Range cut, at most 5 seconds long. Video 3 is only D3 ≈ 6 s long, and the dialog refuses a shot shorter than the words.
2. Choose Insert ▾ → **Layer…**. The picker shows "This project's videos" above "Uploads". Pick **Video 3 · e2e-3.mp4**, choose **V3** and **Top right**, tick **Play its sound**, leave it at −6 dB, and click **Add layer**.
3. The thin "+ V3" row becomes a V3 lane with a bar reading "PiP ↗ · e2e-3.mp4" and a speaker icon. The transcript shows a tag reading "V3 · e2e-3.mp4 · PiP ↗" with a speaker.
4. Play across those words. A box about 30 % of the frame's width sits in the top-right corner, 4 % in from each edge, showing video 3's `testsrc` picture. You hear video 3's sound under video 1's voice, quieter while words are spoken.
5. Take a screenshot of the editor with the PiP visible, the V3 bar, and the badges. This is the README screenshot candidate (Step 9).

- [ ] **Step 7: Live step 5 — export and check it with ffprobe**

1. Read the numbers the export must match, with `mcp__claude-in-chrome__javascript_tool` on the editor tab. The App keeps the open project's id in `history.state.project`.

   ```js
   const id = history.state.project;
   const { project, doc } = await (await fetch(`/api/projects/${id}`)).json();
   const stitched = project.sources.reduce((end, s) => Math.max(end, s.offset + s.duration), 0);
   const cut = doc.edits
     .filter((e) => e.kind === 'cut')
     .reduce((sum, e) => sum + (e.end - e.start), 0);
   const layer = doc.edits.find((e) => e.kind === 'layer' && e.track === 3);
   // Output order is video 3, then video 1: the layer's middle, in output seconds.
   const pipAt = project.sources[2].duration + (layer.start + layer.end) / 2;
   ({
     sources: project.sources.map((s) => [s.filename, s.offset, s.duration]),
     stitched,
     cut,
     expected: stitched - cut,
     pipAt,
   });
   ```

   Expected: three sources at offsets 0, D1 and D1 + D2, with `stitched` equal to D1 + D2 + D3 (about 24 s). `cut` is the Range cut's length. `expected` is the export's duration. It must also match the output length the transport shows after `/`, to the second. There are no title cards or overdubs in this project, so nothing else changes the length. `pipAt` is an output second inside the layer, because the layer sits before the cut in video 1.

2. Click **Export**. Leave the format at mp4, wait for the download link, and copy its path, which has the form `/data/{first media id}/export-{n}.mp4`. Set `EXPORT_PATH` to it in the shell.
3. Download it and probe it:

   ```bash
   curl -s -o "$S/e2e/export.mp4" "http://localhost:5175$EXPORT_PATH"
   ffprobe -v error \
     -show_entries format=duration:stream=index,codec_type,width,height,r_frame_rate,sample_rate,channels \
     -of json "$S/e2e/export.mp4"
   ```

   Expected:
   - **Duration:** `format.duration` equals `expected` from item 1, within 0.1 s. Encoder padding accounts for the small difference.
   - **Canvas:** exactly one video stream at `width` 1280, `height` 720 and `r_frame_rate` `30/1`. That is video 1's size and rate: the first source with a picture, even though video 3 plays first in the reordered edit. Video 3's 640×480@25 has been fitted, pillarboxed and resampled.
   - **Audio:** exactly one audio stream, `sample_rate` `48000` and `channels` 2. The layer's sound is mixed into it rather than added as a second stream, and video 3's mono 44.1 kHz audio has been normalised.
   - Two streams in total.

4. Spot-check the picture. Set `PIP_AT` to the `pipAt` value from item 1, then extract a frame there and one at 1 s:

   ```bash
   ffmpeg -y -loglevel error -ss "$PIP_AT" -i "$S/e2e/export.mp4" -frames:v 1 "$S/e2e/pip.png"
   ffmpeg -y -loglevel error -ss 1 -i "$S/e2e/export.mp4" -frames:v 1 "$S/e2e/first.png"
   ```

   Read both images. `pip.png` shows video 1's `testsrc2` with video 3's `testsrc` in a box at the top right, about 384 px wide and inset 51 px from the right and 29 px from the top. `first.png` shows video 3's 4:3 `testsrc` pillarboxed with black bars on a 16:9 frame.

- [ ] **Step 8: The full checks**

```bash
cd /Users/jonyen/Projects/type-n-stitch
export PATH="$HOME/.cargo/bin:$PATH"
npm test && npm run lint && npm run build -w client
git status --short
```

Expected: all pass, and `git status` lists no stray files. The clips and the export live in the scratchpad; the project's media lives under `server/data/`, which git ignores.

- [ ] **Step 9: README screenshot and wording, then commit**

The UI changed materially: the B-roll lane became V2 with a V3 lane above it, clips carry source badges, and B-roll became Layer everywhere. So the screenshot is out of date. Retake it at a 1280 × 800 window in the dark theme, on the `e2e-` project from Step 6: a PiP visible in the viewer, the V3 bar and the three badges on the timeline, and a few words selected so the floating toolbar shows. Save it over `docs/screenshot.png`. Do not use the TEDx project; opening it to stage a screenshot would mean editing it.

In `README.md`:

1. Change the screenshot's alt text to: `type-n-stitch editing three videos stitched into one project: viewer with a picture-in-picture layer on the left, transcript on the right, and the V3, V2, clips and music lanes of the timeline along the bottom`.
2. In "How it works", replace `B-roll shows an
uploaded shot over a range of words (the voice continues);` with `A layer shows an
uploaded shot, or a stretch of one of the project's own videos, over a range of words on track
V2 or V3, full frame or as a picture-in-picture corner, muted or at a set level (the voice
continues);`.
3. In the same section, replace `picks B-roll or
music bars` with `picks layer or
music bars`.
4. Under **Export**, replace `` `overlay` per B-roll window`` with `` `overlay` per layer window (V2, then V3; picture-in-picture layers scaled to 30 % of the width first), every source fitted onto the canvas of the first video with a picture,``.
5. Under **Assets**, replace `B-roll and music files you upload` with `Layer videos and music files you upload`, and `that a B-roll or music edit still` with `that a layer or music edit still`.

Then realign and commit:

```bash
npx prettier --write README.md
npm run lint
git add README.md docs/screenshot.png
git commit -m "README: screenshot and editing notes for several videos per project and stacked layers" -m "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01HJD5eNACC6HhmevGdb9BP2"
```

If the browser tools were unavailable, there is no screenshot to take. Commit only the wording, and say in the report that the screenshot still shows the old B-roll lane.

- [ ] **Step 10: Report**

Report the ffprobe JSON, `expected` from Step 7, the two spot-check frames, and which of Steps 3–6 were checked in the browser. The `e2e-` project stays in place for the user to look at or delete; delete nothing else.

---

## Cross-task notes from the planners

Kept from the five parts for the implementer, with the rulings (R1–R8) applied and task numbers updated. "Part A–E" and "planner A–E" name the planners (see Decisions made while planning).

### Part A (engine, Task 1)

#### Contract notes

- **`stitch_speakers` shape.** Diarization labels are `Vec<Option<u32>>` (see `assign_speakers` and the server's `routes::Speakers { count: u32, words: Vec<Option<u32>>, .. }`), not `usize`. The engine exposes `pub fn stitch_speakers(parts: &[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>)` in `engine/src/speakers.rs`. Each part is one source's `(count, labels)` in stitched order; source _k_'s labels are shifted by the speakers of the sources before it. It returns the stitched `count` and labels. The base grows by `max(count, highest label + 1)`, so a cache whose `count` is too small cannot make two people share an index.
- **Where things live.** `Source` and `Frame` are in `types.rs`. `all_sources`, `stitched_duration`, `locate` and `stitch_words` are in `editlist.rs`. `stitch_speakers` is in `speakers.rs`. `lib.rs` already glob re-exports every module, so all of them are `engine::<name>`. `lib.rs` does not change.
- **`locate` returns `(usize, f64)`,** as the contract says, not the spec's `(MediaId, f64)`. The index is into `all_sources(..)`. A negative `t`, or one more than `EPS` past the stitched end, returns `None`. An instant within `EPS` of a join belongs to the later source.
- **A join is permanent in the fold.** `Op::Unsplit { at }` is a no-op when `at` is a source offset in `doc.sources`, so no log can merge two files into one piece. This is an addition that the contract implies ("the permanent join").
- **Layer replacement rules.** An `AddLayer` replaces the layers it overlaps **on its own track**. That is the old `AddBroll` rule, applied per track. `SetLayer` edits the layer **in place**, so it keeps its index in `edits`, and drops any other layer it now overlaps on `to_track`. Layers are identified by `(track, start)`.
- **Layer JSON.** `audio` serialises as `null` when muted (it is never omitted) and reads `None` when missing. `offset` defaults to `0` when missing, like `Broll` and `Audio`. `Frame` has no `Default`, so `frame` is required.
- **Export stays unchanged.** `editlist::broll_windows` keeps its name and now selects **every** `Edit::Layer`, whatever its track, frame or audio. The ffmpeg planner draws each one full frame and muted, in edit order, exactly as it drew B-roll. A regression test pins this until Task 10 replaces it.
- **No new cap constant.** `MAX_BROLL` stays. Server validation of `AddBroll` now counts track-2 layers against it. The contract defines no `MAX_LAYERS`, so none is added here.
- **Server compile shims.** The server matches exhaustively on `Op` (`server/src/ops.rs::validate`) and names `Edit::Broll` in four places, so Task 1 has to touch the server to keep the workspace building. It switches those references to `Edit::Layer` and adds one arm that rejects the four new ops with "not supported yet". Task 3 replaces that arm.

#### Cross-part notes

- **Serde tags and fields (all parts).** Op tags: `"addsource"`, `"addlayer"`, `"setlayer"`, `"removelayer"`, all lowercase like the existing ops. `SetLayer`'s field is `"toTrack"`. The Edit tag is `"layer"`. Frame values are `"full"`, `"pipTopLeft"`, `"pipTopRight"`, `"pipBottomLeft"` and `"pipBottomRight"`. Layer `audio` is `null` when muted. `ProjectDoc` gains `"sources": Source[]` (default `[]`), holding only appended sources. `Edit::Broll` / `kind: "broll"` no longer exists anywhere.
- **Server validation, Task 3 (B).** Replace the `"not supported yet"` arm in `server/src/ops.rs::validate` with real checks:
  - `AddSource`: `offset` equals `stitched_duration(&all_sources(first, &current.sources))` within `EPS`; `duration > 0`; the 20-source cap.
  - `AddLayer`: `track` is 2 or 3; `end > start`; the range lies inside the stitched duration; `media` is a video asset or a project source; `offset + (end - start)` fits the media; `audio` is `None` or within `MIN_GAIN_DB..=MAX_GAIN_DB`.
  - `SetLayer`: `to_track` is 2 or 3; the layer exists; same audio rule.
  - `RemoveLayer`: range check.

  More for B:
  - `Unsplit` at a join: decided not to reject it on the server. The fold ignores it and the client never sends one (see Decisions).
  - `check_range` and `Split`'s edge check use the first media's `duration`; Task 3 switches them to the stitched duration.
  - Joins live in `doc.splits` and count toward `MAX_SPLITS` (64): decided to leave them counted (see Decisions).
  - `DocState` (the ops response) has no `sources`. Task 3 adds it, holding exactly `doc.sources` (appended sources only), and the WebSocket `hello`/`doc` frames carry the same.

- **Assets (B/C).** After Task 1, `assets::references` and `asset_files` read `Edit::Layer.media` as an asset id. A layer whose `media` is a project **source**, not an asset, makes `asset_files` fail with "asset … does not belong to this project". Task 3 fixes that with one resolver, `sources::resolve_layer_media`, which both its op validation and `asset_files` (and so Task 10's export) call.
- **Stitched words and speakers (B, Task 4).** Call `stitch_words` and `stitch_speakers` with **every** source in `all_sources` order, and pass sources that are still transcribing with empty slices (for speakers: every source after the first one not yet ready passes count 0 and all-`None` labels), so that part _k_ is source _k_. `stitch_speakers` is the only implementation of the base rule; the server's `sources::stitch_speaker_labels` calls it. Word ids of source _k_ > 0 are `"{k}:{id}"`. Stitched speaker indices can exceed `MAX_SPEAKERS` (64) with many speakers, and `RenameSpeaker` rejects those.
- **Export (C, Tasks 5 and 10).** `broll_windows` now returns **every** `Edit::Layer` (any track, frame or audio). The planner draws all of them full frame and muted, in edit order. Task 10 replaces this with per-track overlay, PiP and audio, and must rewrite `a_layer_on_any_track_exports_full_frame_and_muted_for_now`. `locate` returns an index into `all_sources(..)`. A join instant belongs to the later file, and the stitched end is the last file at its duration.
- **Client mirror (D, Task 2).** The reducer must copy the fold exactly:
  - `addbroll` → layer `{track: 2, frame: "full", audio: null}`, replacing overlapping **track-2** layers only.
  - `removebroll` → `removelayer` on track 2.
  - `addlayer` replaces the layers it overlaps on its own track, then pushes.
  - `setlayer` edits in place (keeping its index) and drops the other layers it overlaps on `toTrack`. A missing layer is a no-op.
  - `addsource` pushes the source and adds a split at `offset`.
  - `unsplit` at a source offset is a no-op.

  Between Task 1 and Task 2 the server sends `kind: "layer"` edits that the current client does not draw. The B-roll lane goes empty, but the edits are intact.

- **Layers UI (E).** A layer is addressed by `(track, start)` in `SetLayer` and `RemoveLayer`. Two layers on the same track never overlap after a fold.
- **MCP (B, Task 11).** The existing `add_broll` tool still sends `Op::AddBroll`, which folds to a track-2 layer. No change is needed for the alias.

### Part B (server, Tasks 3, 4 and 11)

#### Contract notes

- **Media ids are not content hashes.** The planning contract said "store media by content
  hash", but `routes::store_upload` has always minted `Uuid::new_v4` ids. Only library clips
  use a derived v5 id. `/sources` stores uploads exactly as `POST /api/projects` does. Nothing
  here depends on hashing.
- **`stitch_speakers`.** The server's diarization returns
  `Speakers { count: u32, words: Vec<Option<u32>>, turns }`. Task 1's
  `engine::stitch_speakers(&[(u32, &[Option<u32>])]) -> (u32, Vec<Option<u32>>)` namespaces the
  labels and the count. Task 4's `sources::stitch_speaker_labels` calls it, then pads labels to
  each source's word count and shifts the turns in time by the same base. There is no second
  implementation of the base rule.
- **Op tags.** The `kind` tags follow the existing `#[serde(tag = "kind", rename_all = "lowercase")]`:
  `addsource`, `addlayer`, `setlayer`, `removelayer`. Fields are camelCase (`toTrack`).
- **`Edit::Broll` is removed by Task 1.** To keep the workspace compiling, Task 1 switches
  `assets::references`/`asset_files` and the `AddBroll` count in `validate` to `Edit::Layer`, and adds
  one `validate` arm that rejects `AddSource`, `AddLayer`, `SetLayer` and `RemoveLayer` with
  "not supported yet". Task 3 deletes that arm and replaces it with real validation.
- **Undo of `AddSource` needs Task 1's fold to support it** (the fold simply doesn't replay it).
  Task 3 adds server-side rules that keep the track contiguous (see Decisions).

#### Cross-part notes

For the client planners (D, E) and the export planner (C). Every shape below is JSON as the
server sends it.

**`SourceView`**

```json
{
  "index": 1,
  "mediaId": "3f2c9a1e-5b7d-4c2f-9e11-0a6b8d4c7e21",
  "url": "/data/3f2c9a1e-5b7d-4c2f-9e11-0a6b8d4c7e21/source.mp4",
  "filename": "take2.mp4",
  "kind": "video",
  "offset": 10.0,
  "duration": 4.0,
  "transcript": "pending"
}
```

- `kind` is `"video"` or `"audio"`.
- `transcript` is one of `"pending"`, `"running"`, `"ready"`, `"error"`:
  - `pending`: not started yet. The next `/transcribe` starts it.
  - `running`: a background job is working on it.
  - `ready`: its words are cached and included in `words`.
  - `error`: failed. It stays that way until the server restarts. Show the block as
    "Couldn't transcribe video N" and stop polling for it.

**Endpoints**

- `POST /api/projects/{id}/sources`
  - Multipart, one field named `file`, the same as `POST /api/projects`.
  - Editors and owners → `200 SourceView`.
  - Errors are `{ "error": string }`:
    - 400 `unsupported file type .txt; …`
    - 400 `could not read media: …`: the file is unreadable, and nothing is registered or
      appended.
    - 400 `a project holds at most 20 videos, and this one is full`: sent before the body is read.
    - 403 for a viewer, a commenter or a non-member.
    - 413 over the upload size limit.
  - Upload files one at a time, in order. Each one lands at the stitched end when it arrives.
  - A concurrent add from someone else can make one fail with 400
    `a video can only be added at the end, at N s`. Retry it.
- `GET /api/projects/{id}` → `{ project, doc }`.
  - `project` is the same `ProjectSummary` as before, including `media` (the first source,
    kept for one release), plus `sources: SourceView[]` in stitched order, first included.
  - `doc` is `DocState` plus `sources: Source[]`, where `Source` is
    `{ media, offset, duration }`. It holds the **appended** sources only, exactly the engine's
    `ProjectDoc.sources`; the project's own media is not in it. The same holds for the `POST /ops`
    reply and the WebSocket frames.
- WebSocket `hello` and `doc` frames carry the same appended `sources: Source[]`. When a media id
  appears that the client has not listed (a peer added a video), it refetches `GET
/api/projects/{id}` for the new `SourceView`s; new words follow through the polling below.
- `POST /api/projects/{id}/transcribe` → `{ words: Word[], sources: SourceView[] }`.
  - `words` are stitched: every ready source, shifted by its offset, in source order.
  - Word ids: source 0 keeps `"w12"`; source k > 0 is `"k:w12"`.
  - The response waits for source 0 only, and starts any `pending` source in the background.
  - The client polls `GET /api/projects/{id}` about every 2 s while any `sources[i].transcript`
    is `pending` or `running`, and posts `/transcribe` again when the ready count grows or a
    source reads `pending`.
  - A source with no words yet is the "Transcribing video N…" block. Its stitched range is
    `[offset, offset + duration)`.
- `POST /api/projects/{id}/speakers` → `{ count, words: (number|null)[], turns }`.
  - Stitched and namespaced. `words` is parallel to the stitched `words`.
  - Labels are `null` for every source after the first one that is not yet ready.
  - `turns` are `{ start, end, speaker }` in stitched seconds.
  - Re-fetch when a source becomes ready.
  - `speakerNames` in the doc index the stitched speaker numbers. Rename is unchanged.
- `POST /api/projects/{id}/suggest` → `{ fillers, pauses }`: `Edit::Cut`s in stitched seconds,
  covering every ready source. 404 while none is ready.
- `POST /api/projects/{id}/thumbnails`, with the optional body `{ "media": SourceView.mediaId }`.
  - Returns the sprite sheet for that source; the first when there is no body.
  - Sheet times are the source's own seconds (subtract its `offset`).
  - 400 for a media that is not one of the project's.

**Ops (POST /ops)**

Tags are lowercase, fields camelCase.

- `{ "kind": "addsource", "media", "offset", "duration" }`
  - Normally submitted by the server's `/sources` route, not the client.
  - `offset` must equal the stitched end. `media` must be in the registry.
- `{ "kind": "addlayer", "track": 2|3, "start", "end", "media", "offset", "frame", "audio": number|null }`
  - `media` is an asset id or any `SourceView.mediaId`. It must be a video.
  - `offset + (end - start)` must not pass the media's end.
  - `audio` is `null` (muted) or −30..12 dB.
  - At most 32 layers across both tracks.
- `{ "kind": "setlayer", "track", "start", "toTrack", "frame", "audio" }`
  - 400 if no layer starts at `start` on `track`.
  - 400 if `toTrack` already has a layer starting there.
- `{ "kind": "removelayer", "track", "start" }`
- `frame` is one of `"full"`, `"pipTopLeft"`, `"pipTopRight"`, `"pipBottomLeft"`,
  `"pipBottomRight"`.
- Undo of an `addsource` is refused while a later `addsource` is live. Redo of one is refused
  unless it would land at the stitched end. The client should grey out Undo accordingly or
  surface the 400 message.

**For C (export, Tasks 5 and 10)**

- `assets::asset_files(state, project, edits)` returns `HashMap<media_id, PathBuf>` for every
  `Edit::Layer` and `Edit::Audio`. A layer resolves through `sources::resolve_layer_media` (an
  asset id or any registry media id); music resolves asset ids only.
- `sources::timeline(state, project, &doc)` gives the full `Vec<Source>`.
- `sources::source_file(state, media)` gives `(PathBuf, Meta)`. `Meta.video` may be `None` for
  promoted assets and old media, so probe as `start_export` already does.
- Media promoted from an asset by MCP `add_source` has `video: None` in `meta.json`.

### Part C (export, Tasks 5 and 10)

#### Contract notes

- Task 1 leaves `engine/src/ffmpeg.rs` with (it only swaps `Edit::Broll` for `Edit::Layer` in the planner):
  - the B-roll block reading `Edit::Layer` through `broll_windows`
  - the variables `brolls` and `broll_input`
  - the labels `v{i}b{n}` / `v{i}bo{n}`
- Task 10 replaces that block wholesale with the code below.
- `locate`, `stitched_duration`, `all_sources` and `stitch_words` are used through the crate root (`crate::…` / `engine::…`). `lib.rs` re-exports every module with `*`, so they resolve wherever Task 1 puts them.

#### Cross-part notes

- **Probe data the server passes.** Nothing new is probed.
  - Each source's existing `Meta` (`meta.json` in its media dir) supplies `ext` (the path `source.<ext>`), `kind`, `duration` and `video: Option<VideoInfo>`. That `video` is already oriented for rotation by `media::probe`.
  - `source_meta` re-probes any video source whose `video` is missing and writes it back. This now covers every source, not only the first.
  - **Task 3's `/sources` route must write the full `Meta`, including `video` from `probe.video`**, exactly as `store_upload` does for the first media. Otherwise every export re-probes that source.
  - No audio parameters are needed: every chain already resamples to 48 kHz stereo.
- **New and changed public API (engine):**
  - New: `SourceInput`, `canvas`, `sources_kind`, `layer_windows`, `PIP_WIDTH`, `PIP_MARGIN`.
  - Changed: `build_ffmpeg_args(&[SourceInput], …)`.
  - Removed: `ExportOptions.duration`, `ExportOptions.video` and `broll_windows`.
  - Server: `routes::export_sources` (`pub(crate)`) and the private `source_meta`.
  - `start_export(state, project, format)` keeps its signature, so the MCP export tool needs no change.
- **Task 1 (A):**
  - `locate`, `stitched_duration` and `all_sources` must be `pub` and reachable through the crate root. `stitch_words` too.
  - `stitched_duration(&[])` should return `0.0`. The planner guards against an empty list itself.
  - `Source` must be `Debug + Clone + PartialEq` and `Frame` must be `Copy + PartialEq`.
  - Task 1 switches `assets::asset_files` and `assets::references` from `Edit::Broll` to `Edit::Layer` so the server compiles. Task 3 then makes `asset_files` resolve layer media through `sources::resolve_layer_media`; Task 10 only adds a test for it.
  - Task 10 assumes Task 1's planner block keeps the names `brolls`/`broll_input` and the labels `v{i}b{n}`/`v{i}bo{n}`.
  - The stitched word ids `"{k}:{id}"` are asserted by Task 5's server test.
- **Task 3 (B):**
  - Task 10's `asset_files` test inserts a `project_sources` row directly, so the migration's column names must match the contract.
  - `AddLayer` validation and `asset_files` share one resolver, `sources::resolve_layer_media`: a project asset, or any media in the registry. So a layer cannot pass validation and then fail at export, or the other way round.
  - Music (`AddAudio`) stays asset-only.
- **Task 4 (B):** export reads each source's own `words-v2.json` (`WORDS_CACHE`) from its media dir and stitches them with `stitch_words`. A source whose transcript is still pending contributes no words, which only means no ducking there. If Task 4 moves the per-source word cache, `export_sources` must follow it.
- **Task 7 (D), preview canvas:**
  - The export canvas is the first source _with a picture_ (see `canvas`). The preview follows the same rule: the first playable source whose video has `videoWidth > 0`, else 16:9 (Task 7's `StitchedMedia.aspect`).
  - `SourceView` carries no dimensions, so the preview reads them from each file's metadata.
  - Letterboxing is `object-fit: contain` centred, which matches `force_original_aspect_ratio=decrease` plus a centred `pad`.
  - An audio-only source shows black.
- **Task 9 (E), layer preview geometry, which must match export:**
  - V3 above V2.
  - Full frame is `object-fit: contain` over the whole canvas box.
  - A PiP layer is `width: 30%` of the canvas box with auto height, inset `4%` of the box width from the left or right edge and `4%` of the box height from the top or bottom edge. In CSS that is `left|right: 4%; top|bottom: 4%` on an absolutely positioned child of the canvas-sized box.
  - A layer with `audio: dB` plays at that level. In export it is also ducked under speech. The preview need not duck.
  - A layer's picture and sound are both absent over title cards.
- **Task 12 (end-to-end):** ffprobe on an export should show:
  - the canvas resolution: the first video source's, after rotation
  - one audio stream at 48 kHz stereo
  - duration equal to `planned`

### Part D (client sources, Tasks 2, 6 and 7)

#### Contract notes

- `locate` and `stitchedDuration` take `readonly Pick<Source, 'offset' | 'duration'>[]`, so `Source[]`, `SourceView[]` and the clock's `{ offset, duration, url }[]` all fit. The name and return shape are the contract's. The body mirrors `engine/src/editlist.rs::locate` in Part A exactly, including the `EPS` snapping at joins and at the end.
- `ProjectSummary.sources` is optional: the list endpoint and an older server may omit it. `sourceViewsOf(project)` falls back to one view built from `project.media`.
- `DocState.sources`, `RemoteDoc.sources` and the socket's `hello`/`doc`/`resync` messages gain `sources?: Source[]` (appended sources only), read as `?? []`.

#### Cross-part notes

- **Part B (Tasks 3–4): `sources` on every fold the client receives.**
  - The client reads `sources` (the fold's appended sources, not the first) from `POST /ops` replies (`DocState`) and from the socket's `hello`, `doc` and `resync` messages.
  - Task 3 adds `sources` (the fold's `doc.sources`, appended only) to `DocState` and to the `hello` and `doc` frames, which are assembled field by field.
  - Without it, a peer's Add video still shows after its refetch, but undo and redo of an `AddSource` do not reach other tabs.
- **Part B: `/transcribe` must not block on appended sources.** (Task 4 does exactly this and answers `{ words, sources }`.)
  - The client assumes `POST /api/projects/:id/transcribe` behaves as follows:
    - It still transcribes source 0 synchronously, as today.
    - It returns stitched words for every source whose transcript is `ready`, passing untranscribed sources with no words.
    - Sources appended by `/sources` are transcribed in the background, and `SourceView.transcript` reports `pending → running → ready | error`.
  - The client polls `GET /api/projects/:id` every 2 s while any source is pending or running. It calls `/transcribe` again each time the ready count grows.
  - If `/transcribe` instead waits for every source, nothing breaks. The project just opens later and the "Transcribing video N…" blocks never show.
- **Part B: `/speakers` has to be stitched and parallel to the stitched words.** The client refetches speakers whenever the words change, and indexes them by word position.
- **Part B: `SourceView.offset` is the op's offset.** The `/sources` reply's `offset` must equal the `offset` in the `AddSource` op it appended. The client echoes it optimistically and deduplicates by offset.
- **Part B: `SourceView` has no size.** It carries no width or height, so the client learns the canvas aspect from each file's metadata: the first playable source whose video has `videoWidth > 0`, else 16:9 (the export's `canvas` rule). If Part B adds `width`/`height`, the Player can use them directly, and Part E's picture-in-picture maths can too.
- **Parts B, C and E: thumbnails.** `/thumbnails` stays source 0's sprite. The timeline hides the hover frame past source 0 rather than showing a wrong one. A stitched sprite, or one sprite per source, is a follow-up if wanted.
- **Part A:**
  - `locate` in `client/src/editlist.ts` is a line-for-line mirror of Part A's `engine/src/editlist.rs::locate` (the EPS snaps at joins and at the end included), and its tests are the engine's cases.
  - The reducer mirrors Part A's layer rules: `AddLayer` replaces the layers it overlaps on its own track, and `SetLayer` edits in place, dropping the layers it now overlaps on `to_track`.
  - The reducer and `opForAction` also treat an `unsplit` at a join as a no-op, as the fold does.
  - Please keep the joins in `doc.splits`. The client's piece layout relies on it: no piece may span two files.
- **Part E (Tasks 8–9): the B-roll names are yours to rename.**
  - Task 2 keeps the B-roll UI names so the app is unchanged: `OverlayRef.kind: 'broll'`, the "B-roll" lane and label, `BrollDialog`, and `Transcript`'s `onBrollClick`. Underneath, they already go through `layers(edits, 2)`, `layerAt(t, edits, 2)`, `addLayer { track: 2, frame: 'full', audio: null }` and `removeLayer { track: 2 }`.
  - `setLayer` is ready in the editor and ops for the Layer dialog.
  - Task 7 changes the Clips lane:
    - `.clip` becomes a flex row with a `.text` span and a `.sourceBadge`.
    - `.sourceJoin` dividers are added.
    - `Timeline`, `Transcript` and `Player` gain the `sources: SourceView[]` prop.
  - Rebase the V2/V3 lane work on those changes.
  - For the layer picker's "the project's own sources", use App's `playable` (`SourceView[]`, stitched order, fold-authoritative).
  - The Player's layer stack (Task 9) goes inside `<Overlays>`, which the Player already renders. The frame's box (`stitched.aspect`, else 16:9) is the canvas for the 30 %/4 % picture-in-picture geometry.
  - `playback.currentTime` stays stitched, main-track time.
- **Part C: no client dependency.** The export needs nothing from the client beyond the ops above.
- **Controller (Task 12).** Task 7's live check covers spec live steps 1–3 (import three clips, one of a different resolution; reorder across sources; a Range cut across a join). Steps 4–5 (a V3 picture-in-picture layer with sound, then export and ffprobe) need Parts C and E.

### Part E (layers UI, Tasks 8 and 9)

#### Contract notes

This part builds on Tasks 2, 6 and 7 as written (planner D). The names below are theirs.

- `client/src/types.ts` exports `Frame`, `SourceView` (contract shapes) and **`LayerEdit`**:
  `{ kind: 'layer'; track: 2 | 3; start: number; end: number; media: string; offset: number; frame: Frame; audio: number | null }`.
  The kind tag is `'layer'`, because the Rust `Edit` enum is `#[serde(tag = "kind", rename_all = "lowercase")]`. `BrollEdit` is gone from the `Edit` union.
- `EditorAction` (client `editor.ts`) has:
  - `{ type: 'addLayer'; track: 2 | 3; media: string; offset: number; frame: Frame; audio: number | null; range?: [number, number] }`
  - `{ type: 'setLayer'; track: 2 | 3; start: number; toTrack: 2 | 3; frame: Frame; audio: number | null }`
  - `{ type: 'removeLayer'; track: 2 | 3; start: number }`

  `opForAction` maps these to the contract's `AddLayer` / `SetLayer` / `RemoveLayer`.

- `types.ts` also exports `LayerTrack = 2 | 3` (Task 2). `overlays.ts` re-exports it, so this part imports it from `../overlays`.
- App has **`playable: SourceView[]`** in scope (Task 7): the fold's sources joined with their file details, in stitched order. This part reads it wherever it needs the project's own videos, and never writes it.
- Task 7's `Player` takes `sources: SourceView[]` and `stitched: StitchedMedia` in place of `media` and `mediaRef`, and already passes nothing but `edits`, `assets`, `words` and `playback` to `<Overlays>`. `Timeline` and `Transcript` already take a `sources: SourceView[]` prop (Task 7), and App passes `sources={playable}` to all three.
- After Task 2, `overlays.ts` has `layers(edits, track?)` and `layerAt(t, edits, track)` in place of `brolls`/`brollAt`. Task 2 kept the B-roll UI names, which this part renames: `OverlayRef.kind: 'broll'`, the "B-roll" lane and its label, `BrollDialog` and `brollRange`, `Transcript`'s `onBrollClick`, `SelectionToolbar`'s `onBroll`, `TopBar`'s `onAddBroll` and its "B-roll…" item, and the `.broll`/`.brollTag` styles.

#### Cross-part notes

- **To planner D (Task 2).** This part dispatches exactly these actions:
  - `addLayer { track, media, offset, frame, audio, range }`
  - `setLayer { track, start, toTrack, frame, audio }`
  - `removeLayer { track, start }`

  It reads `LayerEdit` with `kind: 'layer'` and `track: 2 | 3`. The client `addLayer` reducer drops overlapping layers **on the same track only** (Task 2), so a V3 PiP can sit over a V2 layer. Task 2 replaced `brolls`/`brollAt` with `layers`/`layerAt`; Task 8 adds `layersOn`/`layersAt` beside them.

- **To planner D (Tasks 6 and 7).** Several edits in this part land on files D also changes.
  - `TimelineProps`, `Transcript` Props and `Player` Props gain `sources: SourceView[]` in Task 7; Tasks 8 and 9 reuse it, and App passes `playable`.
  - In `TopBar`, Task 8 renames `onAddBroll` to `onAddLayer` and the "B-roll…" item to "Layer…". Task 6's "Add video…" item stays wherever Task 6 put it.
  - Task 8 edits only the Timeline's label column, the lanes above and below Clips, and the bar renderer. It does not touch the Clips lane, where Task 7's source badges and dividers live.
  - In `Player.tsx`, Task 9 changes only the `<Overlays>` line. Layers are absolutely positioned inside `.frame`, which Task 7 gives the canvas aspect ratio, so PiP placement follows the canvas, as the export's does.
- **To planner B (Tasks 3 and 11).**
  - `AddLayer.media` can be a project asset id **or** a project source's `media_id`. The dialog lists sources by `SourceView.mediaId`. Validation should accept both, check `kind == video`, and check `offset + (end − start) ≤ that file's duration`.
  - `audio` must be null or within −30..12 dB. That is the dialog's slider range, the same as `AddAudio`.
  - `add_layer` for MCP can reuse the same checks.
- **To planner C (Tasks 5 and 10).**
  - PiP geometry must match the preview: `w = 0.3·W`, height from the layer's own aspect ratio, `x = 0.04·W` (left) or `W − w − 0.04·W` (right), `y = 0.04·H` (top) or `H − h − 0.04·H` (bottom), where W and H are the canvas size.
  - A full-frame layer is scaled to fit and letterboxed like the sources.
  - The preview always ducks layer sound under speech by `DUCK = 0.25`, the same as the spec's "ducked like music". The export ducks layer audio too (Task 10).
  - Layer files can be project sources (`media_id`), not only assets; `asset_files` resolves both through `sources::resolve_layer_media` (Task 3).
- **To the controller (Task 12).** The live checks in Task 9, Step 7 cover the spec's live step 4 (a PiP layer of video 3 with sound on V3 over video 1), up to export. Task 12 exports it and checks it with ffprobe.

---

## Spec coverage

| Spec section                                                                                                                                                                                | Where it is implemented                                                                                            |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Problem; Decisions (stacked layers, sources in sequence, muted by default, full frame or PiP, one stitched timeline)                                                                        | Tasks 1–11 as a whole; the rejected alternatives are not built                                                     |
| Data model and engine → Sources (`project_sources`, random media ids, backfill)                                                                                                             | Task 3 (migration, registry, `create_project`)                                                                     |
| Data model and engine → Stitched time (fixed offsets)                                                                                                                                       | Task 1 (`Source`, `all_sources`, `stitched_duration`); Task 2 (client mirror)                                      |
| Data model and engine → Operations: `AddSource` with its permanent split and undo, offset at the stitched end                                                                               | Task 1 (fold); Task 3 (end, registry and duration checks, undo/redo rules); Task 2 (client echo)                   |
| Data model and engine → Operations: removing a video is an ordinary cut                                                                                                                     | Existing cut ops, validated against the stitched duration in Task 3                                                |
| Data model and engine → Operations: `AddLayer`, `SetLayer`, `RemoveLayer`; `AddBroll`/`RemoveBroll` folded to a track-2 layer                                                               | Task 1 (fold); Task 3 (validation); Task 2 (client mirror)                                                         |
| Data model and engine → Fold and timeline: `locate`, a join belongs to the later source                                                                                                     | Task 1 (engine); Task 2 (client mirror); Task 5 (export); Task 7 (player)                                          |
| Data model and engine → Words and speakers: per-media cache, stitched words, namespaced speakers                                                                                            | Task 1 (`stitch_words`, `stitch_speakers`); Task 4 (server stitching, speakers, suggestions)                       |
| Import and transcription → Home: several files, sorted by name, reorderable, first creates the project                                                                                      | Task 6                                                                                                             |
| Import and transcription → In a project: Insert ▾ → Add video…                                                                                                                              | Task 6                                                                                                             |
| Import and transcription → Route: `POST /api/projects/{id}/sources`, editors and owners only                                                                                                | Task 3                                                                                                             |
| Import and transcription → Transcription per source, progress per source, "Transcribing video 2…" block, suggestions across sources                                                         | Task 4 (server); Task 7 (polling and blocks)                                                                       |
| Import and transcription → Mixed formats: canvas, fit and letterbox, frame rate, 48 kHz stereo, `object-fit: contain` preview                                                               | Task 5 (export); Task 7 (preview canvas)                                                                           |
| Import and transcription → Audio-only sources show black                                                                                                                                    | Task 5 (export); Task 7 (preview)                                                                                  |
| Import and transcription → Limits: 20 sources, size limit, unreadable file rejected with nothing appended                                                                                   | Task 3 (server); Task 6 (per-file errors)                                                                          |
| Tracks in the UI → Timeline: V3, V2, Clips, Music lanes, "+ V3" row, narrow layout                                                                                                          | Task 8                                                                                                             |
| Tracks in the UI → Timeline: source badges and dividers                                                                                                                                     | Task 7                                                                                                             |
| Tracks in the UI → Adding and editing a layer: Insert ▾ → Layer…, dialog with track, frame, sound, own sources in the picker, toolbar Layer button, select, Delete, double-click `SetLayer` | Task 8                                                                                                             |
| Tracks in the UI → Preview: one `<video>` per visible layer, 30 % / 4 % PiP, layer sound at its level, hover preview stays the main track                                                   | Task 9                                                                                                             |
| Tracks in the UI → Transcript: layer tags with speaker icon, click selects                                                                                                                  | Task 8                                                                                                             |
| Tracks in the UI → Export: overlay V2 then V3, PiP scale and corner, `amix` of layer sound, ducked                                                                                          | Task 10                                                                                                            |
| Agents (MCP): `add_source`, `add_layer`, `set_layer`, `add_broll` alias, `list_clips` source                                                                                                | Task 11                                                                                                            |
| Migration and compatibility: SQL migration and backfill, old logs fold unchanged, `sources` on GET with `media` kept one release                                                            | Task 1 (log compatibility); Task 3 (migration, response)                                                           |
| Testing → Engine (`AddSource` and undo, `locate` boundaries, `AddBroll` fold, two-resolution export plan, V2/V3/PiP/sound plan)                                                             | Tasks 1, 5 and 10                                                                                                  |
| Testing → Client mirror (stitched words, namespaced speakers, badges, `timelineSegments` across sources, layer spans per track)                                                             | Tasks 2, 7 and 8                                                                                                   |
| Testing → Server (`/sources` and its errors, per-source transcription, 20-source cap, MCP tools, `add_broll` alias)                                                                         | Tasks 3, 4 and 11                                                                                                  |
| Testing → Live steps 1–5                                                                                                                                                                    | Task 12 (with earlier partial checks in Tasks 7 and 9)                                                             |
| Order of work                                                                                                                                                                               | Tasks 1–12 in order; the client model (Task 2) runs right after the engine so the client never drops `layer` edits |
| Out of scope (dragging or resizing layer bars, free transform, more than three tracks, multicam, deleting a source's files)                                                                 | Not built                                                                                                          |
