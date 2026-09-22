# Facelift: a dark, pro-editor interface with cut tools

Date: 2026-09-22
Status: approved design, ready for an implementation plan

## Problem

The editor looks like a form, not an editing tool. Fourteen controls sit in
stacked rows of grey buttons under the player, the clip strip and scrubber
marks compete for the same space, and the whole app is styled by one global
1,800-line stylesheet in which every rule applies everywhere. There is no
direct way to cut with the mouse: splitting needs a selection plus a button,
and removing time needs a selection plus Delete.

The facelift makes the app look and work like a video editor — dark,
viewer-first, with a lane timeline and mouse tools for splitting and cutting —
across every screen, and moves styling onto a structure that stays fast to
work in as the app grows.

## Decisions

- **Layout:** viewer first. Large viewer in the middle, transcript as a side
  panel on the right, a full-width lane timeline docked at the bottom.
- **Tool picker:** a clickable toolbar across the top of the timeline with
  three tools — Select, Razor, Range — each with a name, an icon, a shortcut
  and a hint for the active tool.
- **Theme:** dark first, designed and shipped dark by default; a light theme
  remains and follows the system until the viewer picks one.
- **Palette:** "midnight and blue" — blue-tinted near-black surfaces with a
  bright blue accent (values below).
- **Scope:** everything — editor, home and projects, login, every dialog.
- **Styling:** CSS Modules with a shared tokens file, and Radix primitives for
  menus, dialogs, popovers, tooltips and the tool toggle group.

Rejected: script-first and reskin-only layouts (the user chose viewer-first);
Tailwind (fastest to write, but a full markup rewrite and noisy markup for a
UI with heavy custom geometry); plain CSS reorganised by area (keeps the
global-cascade problem and leaves every interactive widget hand-built — this
repo already shipped two dialogs that forgot to take focus).

## Layout

### Top bar

One row. Left to right: logo, project name and source filename; undo and
redo; **Clean up ▾** (remove fillers, tighten pauses, the "you know / I mean"
toggle); **Insert ▾** (title card, caption, B-roll, music, and "Split here" — the razor is
pointer-only, so splitting also needs a keyboard path); the project
transition setting; then, right-aligned, collaborator avatars with the
connection dot, **Agent**, and **Export** — the only filled accent button. Export progress and the finished download appear in a popover
anchored to Export, not as text in the page.

### Main area

The viewer fills the left, with a slim transport underneath: play/pause,
current time, and `output / source` lengths in tabular figures. The
transcript panel on the right keeps its clip dividers, speaker labels, and
caption, B-roll and music tags. Selecting words shows a **floating selection
toolbar** above the selection with Delete, Overdub, Caption and B-roll.

### Timeline dock

Full width at the bottom:

1. **Tool toolbar** — Select (V), Razor (C), Range (X), and a one-line hint
   for the active tool.
2. **Ruler** — time labels in output time.
3. **Lanes** — Clips, B-roll, Music. A playhead line crosses all three.

The dock replaces the clip strip above the transcript and the coloured marks
on today's scrubber.

### Narrow windows

Below 1,000 px the transcript moves under the viewer; the timeline stays
docked. Below 600 px the timeline shows the clips lane only.

### Home, login, dialogs

Same tokens, type and components. Home keeps its structure (dropzone, your
projects, library) on dark surfaces; project cards use a poster frame from the
thumbnail sprite when one exists, the current glyph otherwise. Every modal
uses one shared Radix dialog frame, so focus-on-open, focus trapping, Escape
and focus return are handled once.

### Theme toggle

Dark, light, or system, in the account menu. The choice is stored in
`localStorage` (wrapped in try/catch; the app renders correctly without it).
With no stored choice the app is dark. "System" follows the operating
system's light or dark setting.

## Tools and the timeline

### Output time

The timeline lays out the edit as it will export: clips in their current
order, cut gaps closed, overdub and title holds at their rendered length. The
client gains a mirror of the engine's `timeline_with` that returns segments
with both source and output ranges, plus `outputToSource` and
`sourceToOutput`:

```ts
export interface Segment {
  source: Range;
  output: Range;
  kind: 'source' | 'overdub' | 'title';
  /** Index into `edits` for an overdub or title segment. */
  index?: number;
}
export function timelineSegments(
  duration: number,
  edits: Edit[],
  splits: number[],
  order: number[],
): Segment[];
export function outputToSource(t: number, segments: Segment[]): number;
export function sourceToOutput(t: number, segments: Segment[]): number;
```

`timelineSegments` is built on the existing `pieces()` (which already mirrors
the engine's piece layout) by adding output positions, so the preview has one
layout, not two. These functions are tested with the engine's
own `editlist.rs` cases (reorder, a title at a clip boundary, overdub holds,
cut then title, holds owned by no piece) so the timeline cannot disagree with
the export.

The timeline always fits the whole edit to its width. There is no zoom.

### Select (V) — default

- Click empty lane space or the ruler: seek. Drag the playhead: scrub.
- Drag a clip in the Clips lane: reorder (the existing `move` operation; the
  drop logic of today's clip strip moves here).
- Click a B-roll or music bar: select it; Delete removes it
  (`removebroll` / `removeaudio`). Double-click a music bar: open its level and
  ducking dialog.
- In the transcript: click, shift-click and drag-select words, as today.

### Razor (C) — split

- Hovering the timeline shows a vertical red line **snapped to the nearest
  word start** in output time. Click splits there: `split { at: word.start }`.
- In the transcript, clicking a word splits just before it.
- Illegal spots show a not-allowed cursor and do nothing: inside a cut or an
  overdub, on a title card hold, at time 0, at the end of the media, or on an
  existing split — the same rules the server enforces.

### Range (X) — remove time

- Dragging across the timeline paints a red band; both ends snap to word
  starts in output time. On release the band is converted to source ranges by
  intersecting it with each source and overdub segment; title holds contribute
  nothing. The ranges are sent as **one** `applycuts` operation, so a band that
  crosses a reordered seam is still undone in one step.
- Dragging across words in the transcript cuts them on release, exactly as
  selecting them and pressing Delete.

### Switching tools

Click the toolbar or press V, C or X. The active tool persists until changed;
Escape returns to Select. Shortcuts are ignored while focus is in a text
field, select or contenteditable.

### Server

No server, engine or MCP changes. Every tool emits operations that already
exist — `split`, `move`, `cut`, `applycuts`, `removebroll`, `removeaudio` —
so collaboration, undo and the agent tools keep working unchanged.

## Styling architecture

### Tokens

`client/src/styles/tokens.css` defines every colour, space, radius, shadow
and type size as a custom property. Dark values are the default on `:root`;
light values apply under `:root[data-theme="light"]`, and under
`@media (prefers-color-scheme: light)` when `data-theme="system"`.

| Token                            | Dark                   | Light                 |
| -------------------------------- | ---------------------- | --------------------- |
| `--bg` (page)                    | `#0b0e14`              | `#f5f7fb`             |
| `--surface` (panel)              | `#111622`              | `#ffffff`             |
| `--raised` (buttons, inputs)     | `#1a2130`              | `#eef1f6`             |
| `--timeline`                     | `#0e121b`              | `#eef1f6`             |
| `--track`                        | `#151b27`              | `#e3e8f0`             |
| `--clip` / `--clip-border`       | `#243049` / `#364767`  | `#d6deeb` / `#b8c4d8` |
| `--border`                       | `#1d2432`              | `#dde3ec`             |
| `--text`                         | `#d3d9e4`              | `#151a23`             |
| `--text-strong`                  | `#eef2f8`              | `#0b0f16`             |
| `--muted`                        | `#6f7a90`              | `#5d6879`             |
| `--faint` (struck-through words) | `#58627a`              | `#9aa4b5`             |
| `--accent`                       | `#3b82f6`              | `#2563eb`             |
| `--accent-soft` (selection)      | `rgba(59,130,246,.35)` | `rgba(37,99,235,.18)` |
| `--playhead`                     | `#facc15`              | `#ca8a04`             |
| `--cut`                          | `#f87171`              | `#dc2626`             |
| `--overdub`                      | `#c4b5fd`              | `#7c3aed`             |
| `--broll`                        | `#14b8a6`              | `#0d9488`             |
| `--music`                        | `#f59e0b`              | `#d97706`             |
| `--danger`                       | `#f87171`              | `#dc2626`             |

Lane colours are reserved: cut, overdub, B-roll, music and the playhead are
never reused for anything else, and the accent is never used for a lane.

Spacing uses a 4 px scale (`--space-1` … `--space-8`), radii `--radius-sm`
4 px, `--radius` 6 px, `--radius-lg` 10 px.

### Type

Inter throughout, loaded from the Regular and SemiBold files the server
already serves at `/fonts/` for title cards — nothing new to download. The
existing `'Inter Title'` faces are kept for title-card rendering. Timecodes
use `font-variant-numeric: tabular-nums`.

### CSS Modules

Every component gets a `Name.module.css` beside it. A small
`client/src/styles/global.css` keeps only the reset, `body`, the `@font-face`
rules and the tokens import. `client/src/styles.css` is emptied component by
component and deleted at the end.

### Radix

Radix primitives, unstyled and themed with the tokens:

- **Dialog** for every modal (overdub, title, caption, B-roll, music, agent).
- **Dropdown Menu** for Clean up, Insert and the account menu.
- **Popover** for export status.
- **Tooltip** for tools and icon-only buttons.
- **Toggle Group** for the tool toolbar (pressed state, arrow-key movement).

Package versions are the current releases at install time, pinned in
`client/package.json`.

### New and replaced components

New: `TopBar`, `ToolToolbar`, `Timeline` (ruler, lanes, playhead, tool
interactions), `SelectionToolbar`, `DialogFrame`, `ThemeToggle`.
Replaced: `ClipStrip` (by the Timeline's clips lane), the scrubber in
`Player` (by the timeline ruler and playhead), `Toolbar` (by `TopBar`, the
selection toolbar and the tool toolbar).

## Testing

- **Unit (vitest):** `timelineSegments`, `outputToSource`, `sourceToOutput`
  against the engine's cases; razor snapping and every illegal-spot rule;
  range-band → source cuts, including a band across a reordered seam and one
  spanning an overdub hold and a title; the shortcut guard for text fields;
  theme resolution (stored choice, system fallback, storage unavailable).
- **Component (new):** add `jsdom` and `@testing-library/react` as dev
  dependencies. Cover the tool toolbar (click and V/C/X select tools, Escape
  returns to Select), the timeline emitting the right operation for a razor
  click and a range drag, and every dialog taking focus on open and returning
  it on close.
- **Live:** after each step, drive the real app in the browser in both
  themes and screenshot it. At the end, one full pass: razor split, range cut,
  reorder, add B-roll and music, export, and confirm the output length. Refresh
  `docs/screenshot.png`.

## Order of work

Each step leaves the app usable.

1. Tokens, fonts, global CSS, theme toggle, CSS Modules conventions. The app
   turns dark.
2. Top bar with Radix menus and the export popover, plus the floating
   selection toolbar, so the actions the old toolbar held never disappear.
3. Timeline: segments mirror, ruler, lanes, playhead; remove the clip strip
   and scrubber marks.
4. Tool toolbar and the three tools, on the timeline and in the transcript.
5. Transcript panel restyle.
6. Every dialog on the shared Radix frame.
7. Home and login.
8. Delete `styles.css`; full verification pass; README screenshot.

## Out of scope

- Timeline zoom.
- Dragging or resizing B-roll and music bars (select and delete only).
- Keyboard reordering of clips.
- A Share button. The server has a members API but the client has no sharing
  UI; building one is a feature, not a restyle.
- Compiling the engine to WebAssembly for the preview (a separate project;
  the new timeline mirror is kept honest by the engine's test cases instead).
