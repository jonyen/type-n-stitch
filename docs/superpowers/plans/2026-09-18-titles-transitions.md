# Titles, Captions and Transitions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Title cards between words, lower-third captions over word ranges, and dip-to-black transitions at cuts — all as operations in the log, previewed live in the browser and rendered by the engine.

**Architecture:** `engine::Edit` gains `Title` and `Caption` variants and `Cut` gains an optional per-cut `transition`; `ProjectDoc` gains a project-wide `transition`. The timeline inserts title segments, computes caption windows per segment and the effective transition at each join; the ffmpeg planner renders titles from a `color` source + `drawtext`, captions as `drawtext` with `enable=between()`, and dips as `fade`/`afade` pairs. The browser mirrors the rules in `editlist.ts`, pauses the video for a title the way it does for an overdub, and overlays captions and a fade class on the frame. Every change is an op the existing realtime path already fans out.

**Tech Stack:** Rust 1.98 (engine + axum server), ffmpeg `color`/`drawtext`/`fade`/`afade`/`anullsrc`, React 19 + TypeScript, vitest.

**Spec:** `docs/superpowers/specs/2026-09-18-titles-transitions-design.md`

## Global Constraints

- Serde defaults keep every existing log and client payload parsing: `Cut.transition` defaults to `None` and is omitted when `None`; `ProjectDoc.transition` defaults to `None`.
- Validation: `at`, `start`, `end` within `[0, duration]`; title `duration` within `[0.5, 30]`; `text`/`subtitle` ≤ 200 chars; ≤ 32 titles and ≤ 32 captions per project; `Crossfade` rejected with 400 "crossfade is not supported yet".
- Both sides of a title are always `Dip`; a non-cut, non-title boundary (source → overdub) has no transition.
- Dip length is 0.25 s (`FADE = 0.25`); a piece shorter than 0.5 s gets no fade.
- `drawtext` uses `expansion=none`; text is single-quoted with `'` → `’` and `\` → `\\`.
- Missing `TITLE_FONT` file → export 400 naming the path. Video without probed dimensions → titles render at 1280×720, 30 fps.
- Audio-only exports keep a title's silence and ignore captions.
- `npm test` (`cargo test && vitest run`) and `npm run lint` pass after every task; commit messages are plain prose ending with the session attribution trailer.

## Conventions

- Cargo: `export PATH="$HOME/.cargo/bin:$PATH"`. Engine tests: `cargo test -p engine <module>::`. Server tests reuse `server/src/test_util.rs` and the `ops::tests` fixtures (`setup(role)`, `post_ops`, `cut`). Client tests: `npx vitest run`.
- Times are seconds in source media; ranges half-open; `EPS = 1e-6`.

## File structure

Modified (engine): `engine/src/types.rs`, `engine/src/ops.rs`, `engine/src/editlist.rs`, `engine/src/ffmpeg.rs`, `engine/src/lib.rs` (no change needed — modules already re-exported).
Modified (server): `server/src/media.rs` (probe dims), `server/src/routes.rs` (`Meta`, export wiring, lazy re-probe), `server/src/config.rs` (`title_font`), `server/src/ops.rs` (validation), `server/src/error.rs` (none), `README.md`.
Modified (client): `client/src/types.ts`, `client/src/ops.ts`, `client/src/editor.ts`, `client/src/editlist.ts`, `client/src/tokens.ts`, `client/src/usePlayback.ts`, `client/src/components/Player.tsx`, `client/src/components/Transcript.tsx`, `client/src/components/Toolbar.tsx`, `client/src/App.tsx`, `client/src/styles.css`, plus their tests.
Created (client): `client/src/components/TitleDialog.tsx`, `client/src/components/CaptionDialog.tsx`, `client/src/components/TitleCard.tsx` (also renders `Caption`).

---

### Task 1: Engine types and fold — Title, Caption, transitions

**Files:**

- Modify: `engine/src/types.rs`, `engine/src/ops.rs`, `engine/src/editlist.rs` (exhaustive matches only), `engine/src/ffmpeg.rs` (exhaustive matches only)

**Interfaces:**

- Produces:

```rust
// types.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub enum Transition { #[default] None, Dip, Crossfade }
#[derive(… Default …)] #[serde(rename_all = "lowercase")]
pub enum TitleStyle { #[default] Dark, Light, Accent }
#[derive(… Default …)] #[serde(rename_all = "camelCase")]
pub enum CaptionPos { #[default] BottomLeft, BottomCenter, TopLeft }

pub enum Edit {
    Cut { start, end, #[serde(default, skip_serializing_if = "Option::is_none")] transition: Option<Transition> },
    Overdub { … unchanged … },
    #[serde(rename_all = "camelCase")]
    Title { at: f64, duration: f64, text: String, subtitle: Option<String>, style: TitleStyle },
    Caption { start: f64, end: f64, text: String, position: CaptionPos },
}
impl Edit { pub fn range(&self) -> Range /* Title => [at, at) */ ; pub fn is_cut(&self) -> bool }
pub const MAX_TITLES: usize = 32; pub const MAX_CAPTIONS: usize = 32;

// ops.rs
pub enum Op { …existing…,
    #[serde(rename_all = "camelCase")] AddTitle { at, duration, text, subtitle: Option<String>, style: TitleStyle },
    #[serde(rename_all = "camelCase")] EditTitle { at, duration, text, subtitle: Option<String>, style: TitleStyle },
    RemoveTitle { at: f64 },
    AddCaption { start, end, text, position: CaptionPos },
    RemoveCaption { start: f64 },
    SetTransition { transition: Transition },
    SetCutTransition { start: f64, transition: Option<Transition> },
}
pub struct ProjectDoc { pub edits, pub speaker_names, #[serde(default)] pub transition: Transition }
```

Variant tags (lowercase): `addtitle`, `edittitle`, `removetitle`, `addcaption`, `removecaption`, `settransition`, `setcuttransition`.

- [ ] **Step 1: Write the failing tests**

Append to `engine/src/ops.rs` `mod tests` (uses the existing `op`, `undone`, `cut` helpers):

```rust
    fn title(at: f64) -> Op {
        Op::AddTitle {
            at,
            duration: 3.0,
            text: "Chapter".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }
    }

    #[test]
    fn add_edit_remove_title() {
        let doc = fold(&[op(1, title(5.0))]);
        assert!(matches!(doc.edits[0], Edit::Title { at, duration, .. } if at == 5.0 && duration == 3.0));

        let doc = fold(&[
            op(1, title(5.0)),
            op(2, Op::EditTitle { at: 5.0, duration: 2.0, text: "Part two".into(), subtitle: Some("sub".into()), style: TitleStyle::Accent }),
        ]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(&doc.edits[0], Edit::Title { duration, text, subtitle: Some(s), style: TitleStyle::Accent, .. }
            if *duration == 2.0 && text == "Part two" && s == "sub"));

        let doc = fold(&[op(1, title(5.0)), op(2, title(5.0)), op(3, Op::RemoveTitle { at: 5.0 })]);
        assert!(doc.edits.is_empty(), "remove drops every title at that instant");
        let doc = fold(&[op(1, title(5.0)), op(2, Op::EditTitle { at: 9.0, duration: 1.0, text: "x".into(), subtitle: None, style: TitleStyle::Dark })]);
        assert_eq!(doc.edits.len(), 1, "editing a missing title is a no-op");
    }

    #[test]
    fn captions_replace_overlapping_ones_and_remove_by_start() {
        let cap = |start: f64, end: f64| Op::AddCaption { start, end, text: "name".into(), position: CaptionPos::BottomLeft };
        let doc = fold(&[op(1, cap(1.0, 3.0)), op(2, cap(2.0, 4.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Caption { start, .. } if start == 2.0));
        let doc = fold(&[op(1, cap(1.0, 3.0)), op(2, cap(5.0, 6.0)), op(3, Op::RemoveCaption { start: 1.0 })]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Caption { start, .. } if start == 5.0));
    }

    #[test]
    fn cuts_keep_titles_and_captions_inside_them() {
        let cap = Op::AddCaption { start: 2.0, end: 3.0, text: "n".into(), position: CaptionPos::TopLeft };
        let doc = fold(&[op(1, title(2.5)), op(2, cap), op(3, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 3);
    }

    #[test]
    fn transitions_project_wide_and_per_cut() {
        let doc = fold(&[op(1, cut(1.0, 2.0)), op(2, Op::SetTransition { transition: Transition::Dip })]);
        assert_eq!(doc.transition, Transition::Dip);
        assert!(matches!(doc.edits[0], Edit::Cut { transition: None, .. }));
        let doc = fold(&[
            op(1, cut(1.0, 2.0)),
            op(2, Op::SetCutTransition { start: 1.0, transition: Some(Transition::None) }),
        ]);
        assert!(matches!(doc.edits[0], Edit::Cut { transition: Some(Transition::None), .. }));
        let doc = fold(&[op(1, Op::SetCutTransition { start: 9.0, transition: Some(Transition::Dip) })]);
        assert!(doc.edits.is_empty(), "no cut at that start: no-op");
    }

    #[test]
    fn new_ops_and_edits_serialise_with_expected_tags() {
        let json = serde_json::to_value(Op::RemoveTitle { at: 2.0 }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "removetitle", "at": 2.0 }));
        let json = serde_json::to_value(Op::SetCutTransition { start: 1.0, transition: Some(Transition::Dip) }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "setcuttransition", "start": 1.0, "transition": "dip" }));
        let json = serde_json::to_value(Edit::Cut { start: 1.0, end: 2.0, transition: None }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "cut", "start": 1.0, "end": 2.0 }), "None is omitted");
        let old: Edit = serde_json::from_str(r#"{"kind":"cut","start":1,"end":2}"#).unwrap();
        assert!(matches!(old, Edit::Cut { transition: None, .. }));
        let t: Edit = serde_json::from_str(r#"{"kind":"title","at":1,"duration":2,"text":"T","subtitle":null,"style":"light"}"#).unwrap();
        assert!(matches!(t, Edit::Title { style: TitleStyle::Light, .. }));
        let c: Edit = serde_json::from_str(r#"{"kind":"caption","start":1,"end":2,"text":"T","position":"bottomCenter"}"#).unwrap();
        assert!(matches!(c, Edit::Caption { position: CaptionPos::BottomCenter, .. }));
        let doc: ProjectDoc = serde_json::from_str(r#"{"edits":[],"speakerNames":[]}"#).unwrap();
        assert_eq!(doc.transition, Transition::None);
    }
```

Add `use crate::types::{CaptionPos, TitleStyle, Transition};` to the test module's imports (and to the file's imports for the implementation).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p engine ops::` — Expected: compile errors (missing variants/types).

- [ ] **Step 3: Implement**

`engine/src/types.rs` — add after `Range`:

```rust
/// How two output pieces meet. `Crossfade` is reserved: the server rejects
/// it until the planner supports `xfade`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transition {
    #[default]
    None,
    Dip,
    Crossfade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleStyle {
    #[default]
    Dark,
    Light,
    Accent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptionPos {
    #[default]
    BottomLeft,
    BottomCenter,
    TopLeft,
}

/// Upper bounds on inserted text so a stored op cannot bloat every fold.
pub const MAX_TITLES: usize = 32;
pub const MAX_CAPTIONS: usize = 32;
```

Change `Edit`:

```rust
pub enum Edit {
    /// Remove `[start, end)` from the output entirely. `transition` overrides
    /// the project setting where this cut joins its neighbours.
    Cut {
        start: f64,
        end: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<Transition>,
    },
    // Overdub unchanged
    /// A card inserted at source instant `at`; the output grows by `duration`.
    #[serde(rename_all = "camelCase")]
    Title {
        at: f64,
        duration: f64,
        text: String,
        subtitle: Option<String>,
        style: TitleStyle,
    },
    /// Text drawn over the picture for `[start, end)`; the output length is unchanged.
    Caption {
        start: f64,
        end: f64,
        text: String,
        position: CaptionPos,
    },
}

impl Edit {
    pub fn range(&self) -> Range {
        match *self {
            Edit::Cut { start, end, .. }
            | Edit::Overdub { start, end, .. }
            | Edit::Caption { start, end, .. } => Range::new(start, end),
            Edit::Title { at, .. } => Range::new(at, at),
        }
    }
}
```

Every `Edit::Cut { start, end }` struct literal in the engine (`editlist.rs` tests, `ffmpeg.rs` tests, `ops.rs` `apply`) becomes `Edit::Cut { start, end, transition: None }`; every exhaustive `match` over `Edit` in `editlist.rs` (`cut_ranges`, `timeline`'s overdub filter, the `unreachable!` arm) gains a `_ => None` / `_ => unreachable!()` arm. Do not change behaviour in `editlist.rs` here — Task 2 does.

`engine/src/ops.rs`:

- `ProjectDoc` gains `#[serde(default)] pub transition: Transition,`.
- `Op` gains the seven variants (signatures above; `RemoveTitle`, `RemoveCaption`, `SetTransition`, `SetCutTransition`, `AddCaption` need no `rename_all`; `AddTitle`/`EditTitle` need `camelCase` for nothing today — keep it for future fields).
- `apply` arms:

```rust
        Op::Cut { start, end } => {
            let cut = Range::new(*start, *end);
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if inside(e.range(), cut)));
            doc.edits.push(Edit::Cut { start: *start, end: *end, transition: None });
        }
        Op::AddTitle { at, duration, text, subtitle, style } => doc.edits.push(Edit::Title {
            at: *at, duration: *duration, text: text.clone(), subtitle: subtitle.clone(), style: *style,
        }),
        Op::EditTitle { at, duration, text, subtitle, style } => {
            if let Some(Edit::Title { duration: d, text: t, subtitle: s, style: st, .. }) = doc
                .edits
                .iter_mut()
                .find(|e| matches!(e, Edit::Title { at: a, .. } if (a - at).abs() < EPS))
            {
                *d = *duration; *t = text.clone(); *s = subtitle.clone(); *st = *style;
            }
        }
        Op::RemoveTitle { at } => doc
            .edits
            .retain(|e| !matches!(e, Edit::Title { at: a, .. } if (a - at).abs() < EPS)),
        Op::AddCaption { start, end, text, position } => {
            let span = Range::new(*start, *end);
            // A new caption replaces any it overlaps, like an overdub.
            doc.edits
                .retain(|e| !matches!(e, Edit::Caption { .. } if overlaps(e.range(), span)));
            doc.edits.push(Edit::Caption { start: *start, end: *end, text: text.clone(), position: *position });
        }
        Op::RemoveCaption { start } => doc
            .edits
            .retain(|e| !matches!(e, Edit::Caption { start: s, .. } if (s - start).abs() < EPS)),
        Op::SetTransition { transition } => doc.transition = *transition,
        Op::SetCutTransition { start, transition } => {
            for e in &mut doc.edits {
                if let Edit::Cut { start: s, transition: t, .. } = e {
                    if (*s - start).abs() < EPS { *t = *transition; }
                }
            }
        }
```

with `const EPS: f64 = 1e-6;` at module level (or import from `editlist` by making its `EPS` `pub(crate)`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p engine` — Expected: all pass (existing tests updated for the new `transition: None` field compile and pass).

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt --check` — Expected: clean.

```bash
git add engine/src
git commit -m "engine: add title, caption and transition edits to the model and fold"
```

---

### Task 2: Engine timeline — title segments, caption windows, joins, remaps

**Files:**

- Modify: `engine/src/editlist.rs`

**Interfaces:**

- Produces:

```rust
pub enum SegmentKind { Source, Overdub { index: usize }, Title { index: usize } }
pub struct CaptionWindow { pub index: usize, pub start: f64, pub end: f64 } // segment-relative output seconds
pub fn caption_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<CaptionWindow>>; // one Vec per segment
pub struct Join { pub after: usize, pub transition: Transition } // boundary between segments[after] and [after+1]
pub fn joins(segments: &[Segment], edits: &[Edit], project: Transition) -> Vec<Join>;
```

`timeline` signature unchanged; `source_to_output_time`/`output_to_source_time` handle titles.

- [ ] **Step 1: Write the failing tests**

Append to `editlist.rs` `mod tests` (extend the `overdub` helper's `Edit::Cut` literals with `transition: None`; add helpers):

```rust
    fn title(at: f64, duration: f64) -> Edit {
        Edit::Title { at, duration, text: "T".into(), subtitle: None, style: TitleStyle::Dark }
    }

    fn caption(start: f64, end: f64) -> Edit {
        Edit::Caption { start, end, text: "c".into(), position: CaptionPos::BottomLeft }
    }

    fn kinds(tl: &[Segment]) -> Vec<&SegmentKind> { tl.iter().map(|s| &s.kind).collect() }

    #[test]
    fn title_splits_a_kept_piece_and_stretches_the_output() {
        let tl = timeline(10.0, &[title(4.0, 2.0)]);
        assert_eq!(tl.len(), 3);
        assert_eq!(tl[0].source, r(0.0, 4.0));
        assert_eq!(tl[1].kind, SegmentKind::Title { index: 0 });
        assert_eq!(tl[1].source, r(4.0, 4.0));
        assert_eq!(tl[1].output, r(4.0, 6.0));
        assert_eq!(tl[2].source, r(4.0, 10.0));
        assert_eq!(tl[2].output, r(6.0, 12.0));
        assert_eq!(output_duration(&tl), 12.0);
    }

    #[test]
    fn title_at_a_cut_boundary_and_inside_a_cut_still_renders() {
        let tl = timeline(10.0, &[cut(2.0, 4.0), title(2.0, 1.0)]);
        assert_eq!(kinds(&tl), vec![&SegmentKind::Source, &SegmentKind::Title { index: 1 }, &SegmentKind::Source]);
        assert_eq!(tl[2].source, r(4.0, 10.0));
        let tl = timeline(10.0, &[cut(2.0, 4.0), title(3.0, 1.0)]);
        assert_eq!(kinds(&tl), vec![&SegmentKind::Source, &SegmentKind::Title { index: 1 }, &SegmentKind::Source]);
        assert_eq!(output_duration(&tl), 9.0);
    }

    #[test]
    fn two_titles_at_one_instant_keep_edit_order_and_a_title_precedes_an_overdub_there() {
        let tl = timeline(10.0, &[title(5.0, 1.0), title(5.0, 2.0), overdub(5.0, 6.0, 0.5)]);
        assert_eq!(
            kinds(&tl),
            vec![&SegmentKind::Source, &SegmentKind::Title { index: 0 }, &SegmentKind::Title { index: 1 }, &SegmentKind::Overdub { index: 2 }, &SegmentKind::Source]
        );
        assert_eq!(tl[2].output, r(6.0, 8.0));
    }

    #[test]
    fn title_at_start_and_end() {
        let tl = timeline(10.0, &[title(0.0, 1.0), title(10.0, 1.0)]);
        assert_eq!(kinds(&tl), vec![&SegmentKind::Title { index: 0 }, &SegmentKind::Source, &SegmentKind::Title { index: 1 }]);
        assert_eq!(output_duration(&tl), 12.0);
    }

    #[test]
    fn remaps_treat_a_title_like_a_freeze() {
        let tl = timeline(10.0, &[title(4.0, 2.0)]);
        assert_eq!(source_to_output_time(4.0, &tl), 4.0);
        assert_eq!(source_to_output_time(4.5, &tl), 6.5);
        assert_eq!(output_to_source_time(5.0, &tl), 4.0);
        assert_eq!(output_to_source_time(6.5, &tl), 4.5);
    }

    #[test]
    fn caption_windows_are_segment_relative_and_split_across_a_cut() {
        let edits = [cut(3.0, 5.0), caption(2.0, 7.0)];
        let tl = timeline(10.0, &edits);
        let w = caption_windows(&tl, &edits);
        assert_eq!(w.len(), tl.len());
        assert_eq!(w[0], vec![CaptionWindow { index: 1, start: 2.0, end: 3.0 }]);
        assert_eq!(w[1], vec![CaptionWindow { index: 1, start: 0.0, end: 2.0 }]);
    }

    #[test]
    fn caption_over_an_overdub_covers_its_whole_hold_and_titles_get_none() {
        let edits = [overdub(2.0, 3.0, 4.0), caption(2.5, 2.8), title(6.0, 1.0)];
        let tl = timeline(10.0, &edits);
        let w = caption_windows(&tl, &edits);
        assert_eq!(w[1], vec![CaptionWindow { index: 1, start: 0.0, end: 4.0 }]);
        let title_i = tl.iter().position(|s| matches!(s.kind, SegmentKind::Title { .. })).unwrap();
        assert!(w[title_i].is_empty());
    }

    #[test]
    fn joins_use_cut_override_then_project_and_always_dip_around_titles() {
        let edits = [
            Edit::Cut { start: 2.0, end: 3.0, transition: Some(Transition::None) },
            cut(5.0, 6.0),
            title(8.0, 1.0),
            overdub(9.0, 9.5, 1.0),
        ];
        let tl = timeline(10.0, &edits);
        let j = joins(&tl, &edits, Transition::Dip);
        let by_after: Vec<(usize, Transition)> = j.iter().map(|j| (j.after, j.transition)).collect();
        // pieces: [0,2) [3,5) [6,8) T [8,9) OD [9.5,10)
        assert_eq!(
            by_after,
            vec![(0, Transition::None), (1, Transition::Dip), (2, Transition::Dip), (3, Transition::Dip), (4, Transition::None), (5, Transition::None)]
        );
        assert_eq!(joins(&tl, &edits, Transition::None)[1].transition, Transition::None);
    }
```

Imports for tests: `use crate::types::{CaptionPos, TitleStyle, Transition};`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p engine editlist::` — Expected: compile errors (`SegmentKind::Title`, `caption_windows`, `joins`, `CaptionWindow`).

- [ ] **Step 3: Implement**

In `editlist.rs`:

```rust
use crate::types::{Edit, Range, Transition};

pub const EPS: f64 = 1e-6;
/// Length of a dip-to-black on either side of a join.
pub const FADE: f64 = 0.25;

#[derive(Debug, Clone, PartialEq)]
pub enum SegmentKind {
    Source,
    Overdub { index: usize },
    /// A title card: no source picture, `edits[index]`'s text for its duration.
    Title { index: usize },
}
```

Replace the body of `timeline`:

```rust
pub fn timeline(duration: f64, edits: &[Edit]) -> Vec<Segment> {
    let overdubs: Vec<(usize, Range, f64)> = edits
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Edit::Overdub { audio_duration, .. } => Some((i, e.range(), *audio_duration)),
            _ => None,
        })
        .filter(|(_, r, _)| !r.is_empty())
        .collect();
    let titles: Vec<(usize, f64, f64)> = edits
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Edit::Title { at, duration, .. } => Some((i, *at, *duration)),
            _ => None,
        })
        .collect();

    // Source pieces: kept ranges minus every overdub range, then split at
    // every title instant so a title can sit between two halves.
    let overdub_holes = normalize_cuts(&overdubs.iter().map(|(_, r, _)| *r).collect::<Vec<_>>());
    let mut cut_points: Vec<f64> = titles.iter().map(|(_, at, _)| *at).collect();
    cut_points.sort_by(|a, b| a.total_cmp(b));
    let mut pieces: Vec<(Range, SegmentKind)> = kept_segments(duration, edits)
        .into_iter()
        .flat_map(|kept| {
            let holes: Vec<Range> = overdub_holes
                .iter()
                .map(|h| Range::new(h.start.max(kept.start), h.end.min(kept.end)))
                .filter(|h| !h.is_empty())
                .collect();
            complement(kept.end, &holes)
                .into_iter()
                .filter(move |r| r.end > kept.start + EPS)
                .map(move |r| Range::new(r.start.max(kept.start), r.end))
        })
        .flat_map(|r| split_at(r, &cut_points))
        .map(|r| (r, SegmentKind::Source))
        .collect();

    pieces.extend(overdubs.iter().map(|(index, r, _)| (*r, SegmentKind::Overdub { index: *index })));
    pieces.extend(titles.iter().map(|(index, at, _)| (Range::new(*at, *at), SegmentKind::Title { index: *index })));
    // Stable: titles at one instant keep edit order; a title precedes an
    // overdub or source piece starting at the same instant.
    pieces.sort_by(|a, b| a.0.start.total_cmp(&b.0.start).then(rank(&a.1).cmp(&rank(&b.1))));

    let mut out_cursor = 0.0;
    pieces
        .into_iter()
        .map(|(source, kind)| {
            let out_len = match kind {
                SegmentKind::Source => source.len(),
                SegmentKind::Overdub { index } | SegmentKind::Title { index } => match &edits[index] {
                    Edit::Overdub { audio_duration, .. } => *audio_duration,
                    Edit::Title { duration, .. } => *duration,
                    _ => unreachable!("segment index points at a non-hold edit"),
                },
            };
            let output = Range::new(out_cursor, out_cursor + out_len);
            out_cursor += out_len;
            Segment { source, output, kind }
        })
        .collect()
}

fn rank(kind: &SegmentKind) -> u8 {
    match kind {
        SegmentKind::Title { .. } => 0,
        SegmentKind::Overdub { .. } => 1,
        SegmentKind::Source => 2,
    }
}

/// Split `r` at every point strictly inside it.
fn split_at(r: Range, points: &[f64]) -> Vec<Range> {
    let mut out = Vec::new();
    let mut cursor = r.start;
    for &p in points {
        if p > cursor + EPS && p < r.end - EPS {
            out.push(Range::new(cursor, p));
            cursor = p;
        }
    }
    out.push(Range::new(cursor, r.end));
    out
}
```

Remaps:

```rust
pub fn source_to_output_time(t: f64, timeline: &[Segment]) -> f64 {
    for seg in timeline {
        if let SegmentKind::Title { .. } = seg.kind {
            if (t - seg.source.start).abs() < EPS {
                return seg.output.start;
            }
            continue;
        }
        if seg.source.contains(t) {
            return match seg.kind {
                SegmentKind::Source => seg.output.start + (t - seg.source.start),
                _ => seg.output.start,
            };
        }
        if t < seg.source.start {
            return seg.output.start;
        }
    }
    output_duration(timeline)
}

pub fn output_to_source_time(t: f64, timeline: &[Segment]) -> f64 {
    for seg in timeline {
        if seg.output.contains(t) {
            return match seg.kind {
                SegmentKind::Source => seg.source.start + (t - seg.output.start),
                _ => seg.source.start,
            };
        }
    }
    timeline.last().map_or(0.0, |s| s.source.end)
}
```

Captions and joins:

```rust
/// A caption's visible span inside one segment, in seconds from that
/// segment's output start.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionWindow {
    pub index: usize,
    pub start: f64,
    pub end: f64,
}

/// Per segment, every caption that overlaps it. Overdubs hold one frame, so
/// a caption touching them covers the whole hold; titles get none.
pub fn caption_windows(segments: &[Segment], edits: &[Edit]) -> Vec<Vec<CaptionWindow>> {
    segments
        .iter()
        .map(|seg| {
            edits
                .iter()
                .enumerate()
                .filter_map(|(index, e)| {
                    let Edit::Caption { start, end, .. } = e else { return None };
                    let cap = Range::new(*start, *end);
                    match seg.kind {
                        SegmentKind::Title { .. } => None,
                        SegmentKind::Overdub { .. } => (cap.start < seg.source.end && cap.end > seg.source.start)
                            .then(|| CaptionWindow { index, start: 0.0, end: seg.output.len() }),
                        SegmentKind::Source => {
                            let a = cap.start.max(seg.source.start);
                            let b = cap.end.min(seg.source.end);
                            (b > a + EPS).then(|| CaptionWindow {
                                index,
                                start: a - seg.source.start,
                                end: b - seg.source.start,
                            })
                        }
                    }
                })
                .collect()
        })
        .collect()
}

/// How the piece at `after` meets the one following it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Join {
    pub after: usize,
    pub transition: Transition,
}

/// The effective transition at every boundary: both sides of a title dip;
/// a boundary that is a cut takes the cut's override, else the project
/// setting; any other boundary (into or out of an overdub) has none.
pub fn joins(segments: &[Segment], edits: &[Edit], project: Transition) -> Vec<Join> {
    segments
        .windows(2)
        .enumerate()
        .map(|(after, pair)| {
            let (a, b) = (&pair[0], &pair[1]);
            let is_title = |s: &Segment| matches!(s.kind, SegmentKind::Title { .. });
            let transition = if is_title(a) || is_title(b) {
                Transition::Dip
            } else if b.source.start > a.source.end + EPS {
                edits
                    .iter()
                    .find_map(|e| match e {
                        Edit::Cut { start, transition: Some(t), .. } if (start - a.source.end).abs() < EPS => Some(*t),
                        _ => None,
                    })
                    .unwrap_or(project)
            } else {
                Transition::None
            };
            Join { after, transition }
        })
        .collect()
}
```

Note the joins test expects the boundary _out of_ the cut at 5–6 (after piece 1) to be `Dip` from the project default and the boundary after piece 0 to be `None` from the override on the cut starting at 2.0 — the override is looked up by the cut whose `start` equals the left piece's `source.end`, which is exactly how a cut is identified at a join. Adjust the test's expected vector only if you find the piece indexing in the comment wrong; the semantics above are binding.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p engine` — Expected: all pass, including the earlier `timeline_*` tests (source-only behaviour unchanged).

- [ ] **Step 5: Lint and commit**

```bash
git add engine/src/editlist.rs
git commit -m "engine: insert title segments, window captions and resolve joins in the timeline"
```

---

### Task 3: Engine ffmpeg — title pieces, caption drawtext, dip fades

**Files:**

- Modify: `engine/src/ffmpeg.rs`

**Interfaces:**

- Produces:

```rust
pub struct VideoInfo { pub width: u32, pub height: u32, pub fps: f64 }
pub struct ExportOptions<'a> { …existing…, pub video: Option<VideoInfo>, pub font: &'a Path, pub transition: Transition }
pub fn drawtext_escape(s: &str) -> String;
pub const DEFAULT_VIDEO: VideoInfo = VideoInfo { width: 1280, height: 720, fps: 30.0 };
```

- [ ] **Step 1: Write the failing tests**

Add to `ffmpeg.rs` `mod tests` (extend the existing `opts()`/graph helpers with `video: Some(VideoInfo{1280,720,30.0})`, `font: Path::new("/fonts/F.ttf")`, `transition: Transition::None` — read the test module first and add fields to its options constructor):

```rust
    #[test]
    fn drawtext_escape_neutralises_quotes_and_backslashes() {
        assert_eq!(drawtext_escape("It's 50% \\ done: yes"), "It’s 50% \\\\ done: yes");
    }

    #[test]
    fn title_piece_is_a_color_source_with_text_and_silence() {
        let edits = [Edit::Title { at: 2.0, duration: 3.0, text: "Hello".into(), subtitle: Some("sub".into()), style: TitleStyle::Accent }];
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(g.contains("color=c=0x2563eb:s=1280x720:r=30:d=3"), "{g}");
        assert!(g.contains("drawtext=fontfile=/fonts/F.ttf:expansion=none:text='Hello'"), "{g}");
        assert!(g.contains("text='sub'"), "{g}");
        assert!(g.contains("anullsrc=r=48000:cl=stereo,atrim=end=3"), "{g}");
        assert!(g.contains("concat=n=3:v=1:a=1"), "{g}");
        assert!(g.contains("setsar=1"), "{g}");
    }

    #[test]
    fn title_without_video_info_uses_the_default_frame() {
        let edits = [Edit::Title { at: 2.0, duration: 1.0, text: "T".into(), subtitle: None, style: TitleStyle::Dark }];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| o.video = None);
        assert!(g.contains("s=1280x720:r=30"), "{g}");
        assert!(g.contains("c=0x111111"), "{g}");
    }

    #[test]
    fn audio_only_export_keeps_title_silence_and_skips_captions() {
        let edits = [
            Edit::Title { at: 2.0, duration: 1.0, text: "T".into(), subtitle: None, style: TitleStyle::Dark },
            Edit::Caption { start: 3.0, end: 4.0, text: "c".into(), position: CaptionPos::BottomLeft },
        ];
        let g = graph(&edits, MediaKind::Audio, OutputFormat::Mp3);
        assert!(g.contains("anullsrc"), "{g}");
        assert!(!g.contains("drawtext"), "{g}");
        assert!(g.contains("concat=n=3:v=0:a=1"), "{g}");
    }

    #[test]
    fn caption_is_drawn_on_its_segment_with_an_enable_window() {
        let edits = [cut(3.0, 5.0), Edit::Caption { start: 2.0, end: 7.0, text: "Ada".into(), position: CaptionPos::BottomCenter }];
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(g.contains("text='Ada':fontsize=25:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=12:x=(w-text_w)/2:y=h*0.85-text_h:enable='between(t,2,3)'"), "{g}");
        assert!(g.contains("enable='between(t,0,2)'"), "{g}");
    }

    #[test]
    fn dip_adds_fade_pairs_only_at_dipping_joins() {
        let edits = [cut(3.0, 5.0)];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| o.transition = Transition::Dip);
        assert!(g.contains("fade=t=out:st=2.75:d=0.25"), "{g}");
        assert!(g.contains("afade=t=out:st=2.75:d=0.25"), "{g}");
        assert!(g.contains("fade=t=in:st=0:d=0.25"), "{g}");
        assert!(g.contains("afade=t=in:st=0:d=0.25"), "{g}");
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(!g.contains("fade="), "{g}");
    }

    #[test]
    fn a_piece_shorter_than_half_a_second_is_not_faded() {
        let edits = [cut(0.3, 5.0)];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| o.transition = Transition::Dip);
        assert!(!g.contains("fade=t=out"), "{g}");
        assert!(g.contains("fade=t=in"), "{g}");
    }
```

Provide `graph_with(edits, kind, format, tweak: impl FnOnce(&mut ExportOptions))` in the test module (build default opts, apply `tweak`, call `build_ffmpeg_args`, return the `-filter_complex` value). Update the existing assertions that end with `…setpts=PTS-STARTPTS[v0]` to include the new `,setsar=1` (video pieces only).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p engine ffmpeg::` — Expected: compile errors.

- [ ] **Step 3: Implement**

```rust
use crate::editlist::{caption_windows, joins, timeline, Segment, SegmentKind, FADE};
use crate::types::{CaptionPos, Edit, MediaKind, TitleStyle, Transition};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo { pub width: u32, pub height: u32, pub fps: f64 }
pub const DEFAULT_VIDEO: VideoInfo = VideoInfo { width: 1280, height: 720, fps: 30.0 };

pub struct ExportOptions<'a> {
    // existing fields …
    /// Frame size and rate of the source picture; titles must match it.
    pub video: Option<VideoInfo>,
    /// Font file for `drawtext`.
    pub font: &'a Path,
    pub transition: Transition,
}

/// Make `s` safe inside a single-quoted `drawtext` text with `expansion=none`:
/// apostrophes become typographic, backslashes are doubled.
pub fn drawtext_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "’")
}

fn title_colors(style: TitleStyle) -> (&'static str, &'static str) {
    match style {
        TitleStyle::Dark => ("0x111111", "white"),
        TitleStyle::Light => ("0xf6f6f7", "0x17181a"),
        TitleStyle::Accent => ("0x2563eb", "white"),
    }
}

fn title_video(edit: &Edit, video: VideoInfo, font: &Path, hold: f64) -> String {
    let Edit::Title { text, subtitle, style, .. } = edit else { unreachable!() };
    let (bg, fg) = title_colors(*style);
    let font = font.to_string_lossy();
    let big = video.height / 12;
    let small = video.height / 24;
    let mut s = format!(
        "color=c={bg}:s={w}x{h}:r={fps}:d={d},format=yuv420p,setsar=1",
        w = video.width, h = video.height, fps = fmt(video.fps), d = fmt(hold)
    );
    match subtitle {
        Some(sub) if !sub.trim().is_empty() => {
            let _ = write!(s, ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={big}:fontcolor={fg}:x=(w-text_w)/2:y=h*0.42-text_h/2", drawtext_escape(text));
            let _ = write!(s, ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={small}:fontcolor={fg}:x=(w-text_w)/2:y=h*0.58-text_h/2", drawtext_escape(sub));
        }
        _ => {
            let _ = write!(s, ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={big}:fontcolor={fg}:x=(w-text_w)/2:y=(h-text_h)/2", drawtext_escape(text));
        }
    }
    s
}

fn silence(hold: f64) -> String {
    format!("anullsrc=r=48000:cl=stereo,atrim=end={},asetpts=PTS-STARTPTS", fmt(hold))
}

fn caption_filter(edit: &Edit, video: VideoInfo, font: &Path, start: f64, end: f64) -> String {
    let Edit::Caption { text, position, .. } = edit else { unreachable!() };
    let (x, y) = match position {
        CaptionPos::BottomLeft => ("w*0.05", "h*0.85-text_h"),
        CaptionPos::BottomCenter => ("(w-text_w)/2", "h*0.85-text_h"),
        CaptionPos::TopLeft => ("w*0.05", "h*0.08"),
    };
    format!(
        ",drawtext=fontfile={}:expansion=none:text='{}':fontsize={}:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=12:x={x}:y={y}:enable='between(t,{},{})'",
        font.to_string_lossy(), drawtext_escape(text), video.height / 28, fmt(start), fmt(end)
    )
}
```

In `build_ffmpeg_args`, after `segments`:

```rust
    let video_info = opts.video.unwrap_or(DEFAULT_VIDEO);
    let windows = caption_windows(&segments, edits);
    let joins = joins(&segments, edits, opts.transition);
    let dip_out = |i: usize| joins.iter().any(|j| j.after == i && j.transition == Transition::Dip) && segments[i].output.len() >= 2.0 * FADE;
    let dip_in = |i: usize| i > 0 && joins.iter().any(|j| j.after + 1 == i && j.transition == Transition::Dip) && segments[i].output.len() >= 2.0 * FADE;
```

and in the segment loop build each piece's video and audio chains as `String`s (instead of writing straight into `graph`), then append fades and captions before the label:

```rust
        let (mut v, mut a) = match seg.kind {
            SegmentKind::Source => (render_video.then(|| video_trim(seg)), audio_trim(seg)),
            SegmentKind::Overdub { index } => {
                let hold = seg.output.len();
                (render_video.then(|| freeze_frame(seg, opts.duration, hold)), overdub_audio(input_index[&index], hold))
            }
            SegmentKind::Title { index } => {
                let hold = seg.output.len();
                (render_video.then(|| title_video(&edits[index], video_info, opts.font, hold)), silence(hold))
            }
        };
        if let Some(v) = v.as_mut() {
            for w in &windows[i] {
                v.push_str(&caption_filter(&edits[w.index], video_info, opts.font, w.start, w.end));
            }
            if dip_in(i) { let _ = write!(v, ",fade=t=in:st=0:d={}", fmt(FADE)); }
            if dip_out(i) { let _ = write!(v, ",fade=t=out:st={}:d={}", fmt(seg.output.len() - FADE), fmt(FADE)); }
        }
        if dip_in(i) { let _ = write!(a, ",afade=t=in:st=0:d={}", fmt(FADE)); }
        if dip_out(i) { let _ = write!(a, ",afade=t=out:st={}:d={}", fmt(seg.output.len() - FADE), fmt(FADE)); }
        if let Some(v) = v { let _ = write!(graph, "{v}[v{i}];"); }
        let _ = write!(graph, "{a}[a{i}];");
```

`video_trim` and `freeze_frame` append `,setsar=1` so the `color` source (SAR 1) concatenates with the source picture. Title pieces on audio-only exports produce only the silence chain (video chain `None`), so `concat` counts still match.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p engine` — Expected: all pass.

- [ ] **Step 5: Lint and commit**

```bash
git add engine/src/ffmpeg.rs
git commit -m "engine: render titles, captions and dip transitions in the ffmpeg plan"
```

---

### Task 4: Server — probe dimensions, font config, validation, export wiring

**Files:**

- Modify: `server/src/media.rs`, `server/src/routes.rs`, `server/src/config.rs`, `server/src/ops.rs`, `server/src/projects.rs` (`test_support::seed_media` gains dims)

**Interfaces:**

- Produces: `Probe { duration, kind, video: Option<VideoInfo> }`; `Meta { …, #[serde(default)] video: Option<VideoInfo> }` (serialised as `video: {width,height,fps} | null`); `Config.title_font: PathBuf` (`TITLE_FONT`, default per OS); `GET /api/projects/:id` `doc` carries `transition`; `DocState.transition`.

- [ ] **Step 1: Write the failing tests**

Append to `server/src/ops.rs` `mod tests`:

```rust
    fn title_op(op_id: &str, at: f64, duration: f64) -> Value {
        json!({ "opId": op_id, "kind": "addtitle", "at": at, "duration": duration, "text": "Intro", "subtitle": null, "style": "dark" })
    }

    #[tokio::test]
    async fn titles_captions_and_transitions_round_trip_through_the_doc() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let (status, body) = post_ops(&state, &ada, &project, vec![
            title_op("t", 2.0, 3.0),
            json!({ "opId": "c", "kind": "addcaption", "start": 1.0, "end": 4.0, "text": "Ada", "position": "bottomLeft" }),
            json!({ "opId": "s", "kind": "settransition", "transition": "dip" }),
            cut("k", 5.0, 6.0),
            json!({ "opId": "o", "kind": "setcuttransition", "start": 5.0, "transition": "none" }),
        ]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["transition"], "dip");
        assert_eq!(body["edits"][0]["kind"], "title");
        assert_eq!(body["edits"][1]["kind"], "caption");
        assert_eq!(body["edits"][2]["transition"], "none");
        let (_, got, _) = call(app(&state), json_req(Method::GET, &format!("/api/projects/{project}"), Some(&ada), None)).await;
        assert_eq!(got["doc"]["transition"], "dip");
    }

    #[tokio::test]
    async fn title_and_caption_validation() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        for (i, bad) in [
            title_op("a", 11.0, 3.0),
            title_op("b", 2.0, 0.2),
            title_op("c", 2.0, 31.0),
            json!({ "opId": "d", "kind": "addtitle", "at": 1.0, "duration": 2.0, "text": "x".repeat(201), "subtitle": null, "style": "dark" }),
            json!({ "opId": "e", "kind": "addcaption", "start": 3.0, "end": 2.0, "text": "x", "position": "topLeft" }),
            json!({ "opId": "f", "kind": "settransition", "transition": "crossfade" }),
            json!({ "opId": "g", "kind": "setcuttransition", "start": 1.0, "transition": "crossfade" }),
        ].into_iter().enumerate() {
            let (status, body) = post_ops(&state, &ada, &project, vec![bad]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "case {i}: {body}");
            assert_eq!(body["index"], 0);
        }
        let (_, body) = post_ops(&state, &ada, &project, vec![json!({ "opId": "z", "kind": "settransition", "transition": "crossfade" })]).await;
        assert_eq!(body["error"], "crossfade is not supported yet");
    }

    #[tokio::test]
    async fn at_most_32_titles_per_project() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        let ops: Vec<Value> = (0..32).map(|i| title_op(&format!("t{i}"), 1.0, 1.0)).collect();
        let (status, _) = post_ops(&state, &ada, &project, ops).await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = post_ops(&state, &ada, &project, vec![title_op("t32", 1.0, 1.0)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["index"], 0);
    }

    #[tokio::test]
    async fn export_fails_clearly_without_the_title_font() {
        let (state, _d, ada, _bob, project) = setup(None).await;
        post_ops(&state, &ada, &project, vec![title_op("t", 2.0, 1.0)]).await;
        // seed_media leaves no font; config points at a missing file in the temp dir.
        let (status, body, _) = call(app(&state), json_req(Method::POST, &format!("/api/projects/{project}/export"), Some(&ada), Some(json!({})))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("TITLE_FONT"), "{body}");
        // Without a title the missing font does not matter.
        let (state, _d, ada, _bob, project) = setup(None).await;
        post_ops(&state, &ada, &project, vec![cut("k", 0.0, 4.0)]).await;
        let (status, body, _) = call(app(&state), json_req(Method::POST, &format!("/api/projects/{project}/export"), Some(&ada), Some(json!({})))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
```

In `server/src/test_util.rs::state()` set `config.title_font = dir.path().join("missing-font.ttf")`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p server ops::` — Expected: compile/assert failures.

- [ ] **Step 3: Implement**

`server/src/config.rs`: add `pub title_font: PathBuf,` set from `TITLE_FONT` with default `if cfg!(target_os = "macos") { "/System/Library/Fonts/Helvetica.ttc" } else { "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf" }`.

`server/src/media.rs`: `Probe` gains `pub video: Option<VideoInfo>`; `ProbeStream` gains `#[serde(default)] width: Option<u32>, height: Option<u32>, r_frame_rate: Option<String>`; ffprobe `-show_entries` adds `,width,height,r_frame_rate` to the `stream=` list; parse `r_frame_rate` `"30000/1001"` → `f64` (numerator/denominator; fall back to 30.0 on parse failure); set `video` from the first real video stream. Re-export `engine::VideoInfo` from here or import it in `routes.rs`.

`server/src/routes.rs`: `Meta` gains `#[serde(default)] pub video: Option<VideoInfo>` (`VideoInfo` needs `Serialize, Deserialize` in the engine — add the derives there with `rename_all = "camelCase"`). `store_upload` and `library::import` fill it from the probe. In `export`:

```rust
    let mut meta = read_meta(&dir).await?;
    if meta.kind == MediaKind::Video && meta.video.is_none() {
        // Media probed before dimensions were recorded: probe once more and remember.
        if let Ok(p) = media::probe(&dir.join(format!("source.{}", meta.ext))).await {
            meta.video = p.video;
            let _ = tokio::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?).await;
        }
    }
    let needs_font = doc.edits.iter().any(|e| matches!(e, Edit::Title { .. } | Edit::Caption { .. }));
    if needs_font && !state.config.title_font.is_file() {
        return Err(AppError::bad_request(format!(
            "titles need a font: TITLE_FONT points at {} which does not exist",
            state.config.title_font.display()
        )));
    }
```

and pass `video: meta.video, font: &state.config.title_font, transition: doc.transition` in `ExportOptions`. `planned` uses the updated `timeline` (titles included) automatically.

`server/src/ops.rs` `validate` arms (after the existing ones; `dir`/`duration` in scope; counts need the current fold — compute once before the loop in `apply_ops`: `let (_, current) = load_doc(state, &project.id).await?; let mut titles = current.edits.iter().filter(|e| matches!(e, Edit::Title{..})).count(); let mut captions = …;` and pass `&mut titles, &mut captions` into `validate`, incrementing on `AddTitle`/`AddCaption` — the fold is cached so this is cheap):

```rust
        Op::AddTitle { at, duration: d, text, subtitle, .. } | Op::EditTitle { at, duration: d, text, subtitle, .. } => {
            if !(0.0..=duration).contains(at) { return Err(AppError::bad_request_at(index, "title is outside the media")); }
            if !(0.5..=30.0).contains(d) { return Err(AppError::bad_request_at(index, "title duration must be between 0.5 and 30 seconds")); }
            if text.chars().count() > 200 || subtitle.as_deref().is_some_and(|s| s.chars().count() > 200) {
                return Err(AppError::bad_request_at(index, "title text is too long"));
            }
            if matches!(op, Op::AddTitle { .. }) {
                if *titles >= MAX_TITLES { return Err(AppError::bad_request_at(index, "too many titles")); }
                *titles += 1;
            }
            Ok(())
        }
        Op::RemoveTitle { at } => check_range(*at, *at),
        Op::AddCaption { start, end, text, .. } => {
            check_range(*start, *end)?;
            if *end <= *start { return Err(AppError::bad_request_at(index, "caption range is empty")); }
            if text.chars().count() > 200 { return Err(AppError::bad_request_at(index, "caption text is too long")); }
            if *captions >= MAX_CAPTIONS { return Err(AppError::bad_request_at(index, "too many captions")); }
            *captions += 1;
            Ok(())
        }
        Op::RemoveCaption { start } => check_range(*start, *start),
        Op::SetTransition { transition } | Op::SetCutTransition { transition: Some(transition), .. } => {
            if *transition == Transition::Crossfade { return Err(AppError::bad_request_at(index, "crossfade is not supported yet")); }
            Ok(())
        }
        Op::SetCutTransition { start, transition: None } => check_range(*start, *start),
```

`DocState` gains `pub transition: Transition` filled from the fold; `seed_media` in `projects.rs::test_support` sets `video: None`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p server` — Expected: all pass; `bus::`/`ws::` untouched.

- [ ] **Step 5: Lint and commit**

```bash
git add server/src engine/src
git commit -m "server: validate title, caption and transition ops and wire them into export"
```

---

### Task 5: Client mirrors — types, ops, reducer, editlist, tokens

**Files:**

- Modify: `client/src/types.ts`, `client/src/ops.ts`, `client/src/ops.test.ts`, `client/src/editor.ts`, `client/src/editor.test.ts`, `client/src/editlist.ts`, `client/src/editlist.test.ts`, `client/src/tokens.ts`, `client/src/tokens.test.ts`, `client/src/App.tsx` (only if a type change breaks the build)

**Interfaces:**

- Produces:

```ts
// types.ts
export type Transition = 'none' | 'dip' | 'crossfade';
export type TitleStyle = 'dark' | 'light' | 'accent';
export type CaptionPos = 'bottomLeft' | 'bottomCenter' | 'topLeft';
export interface CutEdit extends Range {
  kind: 'cut';
  transition?: Transition;
}
export interface TitleEdit {
  kind: 'title';
  at: number;
  duration: number;
  text: string;
  subtitle: string | null;
  style: TitleStyle;
}
export interface CaptionEdit extends Range {
  kind: 'caption';
  text: string;
  position: CaptionPos;
}
export type Edit = CutEdit | OverdubEdit | TitleEdit | CaptionEdit;
// ops.ts: Op gains addtitle/edittitle/removetitle/addcaption/removecaption/settransition/setcuttransition; DocState gains transition
// editor.ts: EditorState.transition; actions addTitle{at,duration,text,subtitle,style}, editTitle{same}, removeTitle{at}, addCaption{start,end,text,position}, removeCaption{start}, setTransition{transition}, setCutTransition{start,transition}; sync/remote carry transition
// editlist.ts: titles(edits), captions(edits), captionsAt(t, edits), outputDuration adds titles, pieces(duration, edits): Piece[] {source: Range, kind: 'source'|'overdub'|'title', index?}, joins(pieces, edits, project): Join[] {after, transition}, nearDipJoin(t, pieces, joins): boolean
// tokens.ts: Token gains { kind: 'title'; title: TitleEdit; before: number } placed before the first word with start >= at (before = words.length after the last)
```

- [ ] **Step 1: Write the failing tests**

`client/src/ops.test.ts` add:

```ts
it('maps the title, caption and transition actions', () => {
  const t = { at: 1.25, duration: 3, text: 'Intro', subtitle: null, style: 'dark' as const };
  expect(opForAction(loaded, { type: 'addTitle', ...t })).toEqual({ kind: 'addtitle', ...t });
  expect(opForAction(loaded, { type: 'editTitle', ...t, text: 'X' })).toEqual({
    kind: 'edittitle',
    ...t,
    text: 'X',
  });
  expect(opForAction(loaded, { type: 'removeTitle', at: 1.25 })).toEqual({
    kind: 'removetitle',
    at: 1.25,
  });
  expect(
    opForAction(select(loaded, 1, 2), { type: 'addCaption', text: 'Ada', position: 'topLeft' }),
  ).toEqual({
    kind: 'addcaption',
    start: 0.91,
    end: 2.0,
    text: 'Ada',
    position: 'topLeft',
  });
  expect(opForAction(loaded, { type: 'addCaption', text: 'Ada', position: 'topLeft' })).toBeNull();
  expect(opForAction(loaded, { type: 'removeCaption', start: 0.91 })).toEqual({
    kind: 'removecaption',
    start: 0.91,
  });
  expect(opForAction(loaded, { type: 'setTransition', transition: 'dip' })).toEqual({
    kind: 'settransition',
    transition: 'dip',
  });
  expect(opForAction(loaded, { type: 'setCutTransition', start: 1, transition: null })).toEqual({
    kind: 'setcuttransition',
    start: 1,
    transition: null,
  });
});
```

`client/src/editor.test.ts` add:

```ts
describe('titles, captions and transitions', () => {
  const t = { at: 1.25, duration: 3, text: 'Intro', subtitle: null, style: 'dark' as const };
  it('adds, edits and removes a title', () => {
    let s = editorReducer(loaded, { type: 'addTitle', ...t });
    expect(s.edits).toEqual([{ kind: 'title', ...t }]);
    s = editorReducer(s, { type: 'editTitle', ...t, text: 'Part 2', style: 'accent' });
    expect(s.edits[0]).toMatchObject({ text: 'Part 2', style: 'accent' });
    s = editorReducer(s, { type: 'removeTitle', at: 1.25 });
    expect(s.edits).toEqual([]);
  });
  it('captions cover the selection and replace overlapping ones', () => {
    let s = editorReducer(select(loaded, 1, 2), {
      type: 'addCaption',
      text: 'A',
      position: 'bottomLeft',
    });
    s = editorReducer(select(s, 2, 3), { type: 'addCaption', text: 'B', position: 'bottomLeft' });
    expect(s.edits).toEqual([
      { kind: 'caption', start: 1.25, end: 20, text: 'B', position: 'bottomLeft' },
    ]);
    s = editorReducer(s, { type: 'removeCaption', start: 1.25 });
    expect(s.edits).toEqual([]);
  });
  it('sets the project transition and a per-cut override, and sync/remote carry it', () => {
    let s = editorReducer(loaded, { type: 'setTransition', transition: 'dip' });
    expect(s.transition).toBe('dip');
    s = editorReducer(select(s, 0), { type: 'deleteSelection' });
    s = editorReducer(s, { type: 'setCutTransition', start: 0, transition: 'none' });
    expect(s.edits[0]).toMatchObject({ kind: 'cut', transition: 'none' });
    s = editorReducer(s, {
      type: 'remote',
      headSeq: 9,
      edits: [],
      speakerNames: [],
      transition: 'none',
    });
    expect(s.transition).toBe('none');
  });
});
```

`client/src/editlist.test.ts` add:

```ts
describe('titles and joins', () => {
  const title = (at: number, duration: number): TitleEdit => ({
    kind: 'title',
    at,
    duration,
    text: 'T',
    subtitle: null,
    style: 'dark',
  });
  it('outputDuration adds title durations', () => {
    expect(outputDuration(10, [title(4, 2), { kind: 'cut', start: 1, end: 2 }])).toBe(11);
  });
  it('pieces mirror the engine: split at a title, title precedes an overdub at the same instant', () => {
    const p = pieces(10, [
      title(5, 1),
      { kind: 'overdub', start: 5, end: 6, text: 'x', audioUrl: '/a', audioDuration: 0.5 },
    ]);
    expect(p.map((x) => x.kind)).toEqual(['source', 'title', 'overdub', 'source']);
    expect(p[0]?.source).toEqual({ start: 0, end: 5 });
    expect(p[3]?.source).toEqual({ start: 6, end: 10 });
  });
  it('joins: override, then project default, always dip around titles, none around overdubs', () => {
    const edits: Edit[] = [
      { kind: 'cut', start: 2, end: 3, transition: 'none' },
      { kind: 'cut', start: 5, end: 6 },
      title(8, 1),
      { kind: 'overdub', start: 9, end: 9.5, text: 'x', audioUrl: '/a', audioDuration: 1 },
    ];
    const p = pieces(10, edits);
    expect(joins(p, edits, 'dip').map((j) => j.transition)).toEqual([
      'none',
      'dip',
      'dip',
      'dip',
      'none',
      'none',
    ]);
  });
  it('nearDipJoin is true within 0.25 s of a dipping boundary in source time', () => {
    const edits: Edit[] = [{ kind: 'cut', start: 3, end: 5 }];
    const p = pieces(10, edits);
    const j = joins(p, edits, 'dip');
    expect(nearDipJoin(2.9, p, j)).toBe(true);
    expect(nearDipJoin(5.2, p, j)).toBe(true);
    expect(nearDipJoin(4, p, j)).toBe(false);
    expect(nearDipJoin(2.5, p, j)).toBe(false);
  });
  it('captionsAt returns captions whose range contains t', () => {
    const c: CaptionEdit = { kind: 'caption', start: 1, end: 2, text: 'c', position: 'topLeft' };
    expect(captionsAt(1.5, [c])).toEqual([c]);
    expect(captionsAt(2, [c])).toEqual([]);
  });
});
```

`client/src/tokens.test.ts` add:

```ts
it('places a title token before the first word at or after its instant', () => {
  const t: TitleEdit = {
    kind: 'title',
    at: 1.0,
    duration: 2,
    text: 'T',
    subtitle: null,
    style: 'dark',
  };
  const kinds = tokenize(words, [t]).map((k) =>
    k.kind === 'title' ? `title@${k.before}` : k.kind,
  );
  expect(kinds).toEqual(['word', 'title@1', 'word', 'word', 'word']);
  const late: TitleEdit = { ...t, at: 19 };
  expect(tokenize(words, [late]).at(-1)).toMatchObject({ kind: 'title', before: 4 });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run` — Expected: type/import failures in the four test files.

- [ ] **Step 3: Implement**

`types.ts` as in Interfaces. `ops.ts`: extend `Op` union with the seven kinds (fields exactly as the server: `addtitle {at,duration,text,subtitle,style}`, `edittitle` same, `removetitle {at}`, `addcaption {start,end,text,position}`, `removecaption {start}`, `settransition {transition}`, `setcuttransition {start, transition: Transition | null}`); `DocState.transition: Transition`; `opForAction` cases: `addTitle`/`editTitle` → spread; `removeTitle`; `addCaption` → `rangeForWords` over the selection (null without one); `removeCaption`; `setTransition`; `setCutTransition`.

`editor.ts`: `EditorState.transition: Transition` (initial `'none'`); `sync` and `remote` copy `transition` from the doc (`remote` action type gains `transition`; `useRealtime`'s `RemoteDoc` gains it and `onRemoteDoc` in `App.tsx` passes it through); reducer cases mirror the engine fold exactly (edit-in-place by `at` with `Math.abs(a - at) < 1e-6`, remove all at that instant, caption replaces overlapping, `setCutTransition` sets on cuts with that `start`). `deleteSelection`'s new cut has no `transition` key.

`editlist.ts`: `titles`, `captions`, `captionsAt`, `outputDuration` (+ sum of title durations), `pieces` (port of the engine's `timeline` without output ranges: kept segments minus overdub holes, split at title instants, plus overdub and title pieces, stable-sorted by start then rank title<overdub<source), `joins` (port of the engine's rule), `nearDipJoin(t, pieces, joins)`: true when some join with `'dip'` has `|t - pieces[after].source.end| < FADE` or `|t - pieces[after+1].source.start| < FADE` (`FADE = 0.25`). `cutRanges`/`overdubs` filters need no change.

`tokens.ts`: before the word loop compute `const cards = titles(edits).sort((a, b) => a.at - b.at)`; while iterating, before pushing word `i`, emit every title with `at <= words[i].start` (and, for titles at exactly `words[i].start`, still before the word); after the loop emit remaining titles with `before: words.length`. `tokenStart`/`tokenEnd` for a title token return `before` (clamped to `words.length - 1` for `tokenEnd`); `turnContains` unchanged.

- [ ] **Step 4: Run to verify pass**

Run: `npx vitest run && npm run build -w client && npm run lint` — Expected: clean (fix any exhaustive-switch or prop-type fallout in `App.tsx`/`useRealtime.ts` from the `transition` field).

- [ ] **Step 5: Commit**

```bash
git add client/src
git commit -m "client: mirror titles, captions and transitions in the model, reducer and timeline"
```

---

### Task 6: Client preview — title pause, caption overlay, dip fade, scrubber marks

**Files:**

- Modify: `client/src/usePlayback.ts`, `client/src/usePlayback.test.ts` (create), `client/src/components/Player.tsx`, `client/src/components/TitleCard.tsx` (create), `client/src/styles.css`

**Interfaces:**

- Produces: `usePlayback` returns `titling: TitleEdit | null`; pure helper `titleCrossed(titles: TitleEdit[], prev: number, now: number): TitleEdit | null` (first title with `prev < at <= now`, by `at` then edit order); `<TitleCard title />` and `<Caption caption />` components; Player props unchanged (reads `edits` and `playback`), plus `transition: Transition`.

- [ ] **Step 1: Write the failing test**

`client/src/usePlayback.test.ts`:

```ts
import { describe, expect, it } from 'vitest';

import { titleCrossed } from './usePlayback';
import type { TitleEdit } from './types';

const t = (at: number): TitleEdit => ({
  kind: 'title',
  at,
  duration: 1,
  text: 'T',
  subtitle: null,
  style: 'dark',
});

describe('titleCrossed', () => {
  it('returns the first title whose instant was crossed since the previous tick', () => {
    expect(titleCrossed([t(5)], 4.9, 5.0)).toEqual(t(5));
    expect(titleCrossed([t(5)], 5.0, 5.1)).toBeNull();
    expect(titleCrossed([t(7), t(5)], 4.9, 8)).toEqual(t(5));
    expect(titleCrossed([t(5)], 6, 4)).toBeNull(); // seeking backwards never triggers
  });
});
```

- [ ] **Step 2: Run to verify failure** — `npx vitest run client/src/usePlayback.test.ts` → import error.

- [ ] **Step 3: Implement**

`usePlayback.ts`:

```ts
/** The earliest title whose instant lies in (prev, now]; null when none or when moving backwards. */
export function titleCrossed(titles: TitleEdit[], prev: number, now: number): TitleEdit | null {
  if (now <= prev) return null;
  let best: TitleEdit | null = null;
  for (const title of titles) {
    if (title.at > prev && title.at <= now && (best === null || title.at < best.at)) best = title;
  }
  return best;
}
```

State: `const [titling, setTitling] = useState<TitleEdit | null>(null)`, refs `activeTitle`, `titleTimer`, `lastTime` (updated in `sync` and reset by `seek`). In `tick`, before the overdub/skip logic: if `activeTitle.current` return after `sync()` (the video is paused; nothing to do); else `const crossed = titleCrossed(titles(editsRef.current), lastTime.current, t)`; if crossed → `enterTitle(crossed)`: pause the media, `media.currentTime = crossed.at`, set `activeTitle`/`titling`, `titleTimer = setTimeout(() => leaveTitle(true), crossed.duration * 1000)`. `leaveTitle(resume)`: clear timer, `lastTime.current = title.at` (so the crossing does not re-trigger), clear state, resume if `wantPlaying`. Undo while titling (title no longer in `editsRef`) → `leaveTitle(true)` like the overdub check. `onPause` treats an active title like an active overdub (not a user pause). Cleanup on unmount clears the timer. `seek(t)` sets `lastTime.current = t` and, if titling, leaves the title without resuming.

`TitleCard.tsx`:

```tsx
import type { CaptionEdit, TitleEdit } from '../types';

export function TitleCard({ title }: { title: TitleEdit }) {
  return (
    <div className={`title-card ${title.style}`} aria-live="polite">
      <div className="title-text">{title.text}</div>
      {title.subtitle && <div className="title-sub">{title.subtitle}</div>}
    </div>
  );
}

export function Caption({ caption }: { caption: CaptionEdit }) {
  return <div className={`caption ${caption.position}`}>{caption.text}</div>;
}
```

`Player.tsx`: after the overdub badge render `{playback.titling && <TitleCard title={playback.titling} />}` and `{captionsAt(playback.currentTime, edits).map((c) => <Caption key={c.start} caption={c} />)}`; compute `const p = pieces(duration, edits); const j = joins(p, edits, transition);` (memoised on `[duration, edits, transition]`) and add class `fading` to `.frame` when `transition !== 'none' || j.some(dip)` and `nearDipJoin(playback.currentTime, p, j)`; scrubber: `titles(edits).map(t => <span className="mark title" style={{ left: pct(t.at) }} />)`. The stats line's `outputDuration` already includes titles (Task 5).

CSS (append; tokens exist): `.title-card` absolute inset 0, grid place-items center, text-align center, padding 6%; `.title-card.dark { background:#111; color:#fff }`, `.light { background:#f6f6f7; color:#17181a }`, `.accent { background:#2563eb; color:#fff }`; `.title-text { font-size: clamp(20px, 6cqw, 48px); font-weight: 650; letter-spacing:-0.01em }`, `.title-sub { font-size: clamp(13px, 3cqw, 22px); opacity: 0.8; margin-top: 0.4em }` (give `.frame` `container-type: inline-size`); `.caption { position:absolute; padding:4px 10px; background:rgba(0,0,0,0.55); color:#fff; font-size: clamp(12px, 2.6cqw, 18px); border-radius:4px; pointer-events:none }` with `.bottomLeft { left:5%; bottom:12% }`, `.bottomCenter { left:50%; transform:translateX(-50%); bottom:12% }`, `.topLeft { left:5%; top:8% }`; `.frame.fading video { opacity: 0.15; transition: opacity 0.25s }`, `.frame video { transition: opacity 0.25s }`; `.scrubber .mark.title { background: #f59e0b; width: 2px }`.

- [ ] **Step 4: Verify** — `npx vitest run && npm run build -w client && npm run lint` clean. Manual: add a title via a curl `POST /ops` (`addtitle`) to a project open in the browser — playback pauses on the card for its duration, then resumes; add a caption — it overlays while the playhead is in range; `settransition` dip — the frame dims briefly at each cut.

- [ ] **Step 5: Commit**

```bash
git add client/src
git commit -m "client: preview title cards, captions and dip transitions in the player"
```

---

### Task 7: Client UI — transcript cards, toolbar, dialogs, App wiring

**Files:**

- Create: `client/src/components/TitleDialog.tsx`, `client/src/components/CaptionDialog.tsx`
- Modify: `client/src/components/Transcript.tsx`, `client/src/components/Toolbar.tsx`, `client/src/App.tsx`, `client/src/styles.css`

**Interfaces:**

- `Transcript` props: `+ selectedTitle: number | null` (a title's `at`), `onTitleClick(at)`, `onTitleOpen(at)`, `onCaptionClick(start)`, `onCutTransition(start, transition: Transition | null)` (shown on gap tokens).
- `Toolbar` props: `+ onAddTitle()`, `onAddCaption()`, `transition: Transition`, `onTransition(t)`; `hasTitleSelection: boolean` (Delete removes the selected title).
- `TitleDialog` props: `{ initial?: TitleEdit; at: number; onSubmit(fields: Omit<TitleEdit,'kind'|'at'>): void; onCancel() }`; `CaptionDialog`: `{ original: string; onSubmit(text: string, position: CaptionPos): void; onCancel() }`.

- [ ] **Step 1: Dialogs**

`TitleDialog.tsx`: modal shell copied from `OverdubDialog` (`modal-backdrop`/`modal`); fields: text input (autofocus, required), subtitle input, three style swatches (radio buttons rendered as colored chips with the `TitleStyle` names), duration number input 0.5–30 step 0.5 default 3; submit label "Add title" / "Save"; Escape cancels; Cmd/Ctrl+Enter submits. `CaptionDialog.tsx`: text input prefilled with `original` (the selected words), position select (Bottom left / Bottom center / Top left), "Add caption".

- [ ] **Step 2: Transcript**

Render `token.kind === 'title'` as

```tsx
<button
  type="button"
  className={`title-token ${token.title.style}${selectedTitle === token.title.at ? ' selected' : ''}`}
  onClick={() => onTitleClick(token.title.at)}
  onDoubleClick={() => onTitleOpen(token.title.at)}
  onKeyDown={(e) => {
    if (e.key === 'Enter') onTitleOpen(token.title.at);
  }}
>
  <span className="title-token-text">{token.title.text}</span>
  <span className="muted">{token.title.duration}s</span>
</button>
```

as a block-level element (own line) inside the turn's `<p>`; do not attach the word press/drag handlers. Captions: for a word index `i`, `captionAt(i)` = the caption whose range covers `words[i]`; when a caption starts at this word, render a `<span className="caption-tag" onClick={() => onCaptionClick(c.start)} title="Click to remove">{c.text}</span>` immediately after the word. Gap tokens (cuts hidden): add a tiny `<button className="gap-transition" title="Transition for this cut">{override ?? '·'}</button>` cycling `null → 'dip' → 'none' → null` via `onCutTransition(cutStartForGap, next)` where `cutStartForGap = words[first].start` (the cut op's start is the first word's start — matches how `deleteSelection` creates cuts).

- [ ] **Step 3: Toolbar and App**

Toolbar: "Add title" (always enabled for editors), "Add caption" (needs a word selection), a `<select>` labeled Transitions: `none` "Jump cut", `dip` "Dip to black", `crossfade` "Crossfade (soon)" disabled; `Delete` handles a selected title too.

App: state `titleDialog: { at: number; initial?: TitleEdit } | null`, `captionDialogOpen`, `selectedTitle: number | null`. `onAddTitle`: `at` = end of the selected range (`rangeForWords(...).end`) if a selection, else `playback.currentTime`; open `TitleDialog`. Submit → `edit({ type: 'addTitle', at, ...fields })` (or `editTitle` when `initial`). `onTitleOpen(at)` finds the title and opens the dialog with `initial`. Delete/Backspace with `selectedTitle !== null` → `edit({ type: 'removeTitle', at })`. Caption submit → `edit({ type: 'addCaption', text, position })` (uses the selection); `onCaptionClick(start)` → `edit({ type: 'removeCaption', start })` after a `confirm`-free click (a second click on the tag removes — keep it simple: single click removes, the tag's `title` says so). `onTransition` → `edit({ type: 'setTransition', transition })`; `onCutTransition` → `edit({ type: 'setCutTransition', start, transition })`. Word selection clears `selectedTitle` and vice versa. `readOnly` disables all of it (`Transcript` ignores title/caption clicks when read-only). `Player` gets `transition={editor.transition}`.

CSS: `.title-token { display:flex; justify-content:space-between; align-items:center; width:100%; margin:6px 0; padding:8px 12px; border-radius:8px; border:1px solid var(--border); font:inherit; text-align:left; cursor:pointer }` with `.dark/.light/.accent` backgrounds matching the card, `.selected { box-shadow: 0 0 0 2px var(--accent) }`; `.caption-tag { display:inline-block; margin-left:4px; padding:0 6px; font-size:11px; border-radius:4px; background: rgba(0,0,0,0.55); color:#fff; cursor:pointer; vertical-align: middle }`; `.gap-transition` tiny ghost; swatches in the dialog.

- [ ] **Step 4: Verify** — `npm run build -w client && npm run lint && npx vitest run` clean. Manual in the browser: select words → Add caption → tag appears and the overlay shows during playback; Add title → card token appears between words, playback pauses on it; double-click edits; Delete removes; Transitions → Dip dims cuts; a second browser sees all of it live.

- [ ] **Step 5: Commit**

```bash
git add client/src
git commit -m "client: add title, caption and transition controls to the editor"
```

---

### Task 8: README

**Files:** `README.md`

- [ ] **Step 1:** In "How it works" step 2, add: "Titles are cards inserted at a word boundary, captions are text drawn over a word range, and a project-wide (or per-cut) transition dips to black where pieces meet; all three are operations like any other." In "Export" (step 4) mention `color`/`drawtext`/`fade`. In Setup add a `TITLE_FONT` row to the env table (default per OS) and a sentence: exports with titles or captions need it to exist. Limitations: "Crossfade transitions are not implemented yet (dip-to-black only)." Run `npx prettier --write README.md && npm test && npm run lint`.

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "README: document titles, captions, transitions and TITLE_FONT"
```

---

## Self-review

**Spec coverage:** data model + fold (T1); timeline/caption windows/joins/remaps (T2); ffmpeg title/caption/dip, defaults, escaping, audio-only (T3); probe dims, font config + 400, validation incl. caps and crossfade rejection, `DocState.transition` (T4); client model/reducer/timeline mirrors and title tokens (T5); preview pause/overlay/fade/scrubber (T6); transcript cards, caption tags, toolbar, dialogs, per-cut override on gap tokens, read-only (T7); README (T8). Realtime needs nothing: `remote`/`sync` carry `transition` (T5) and the rest rides `edits`.

**Placeholder scan:** none. Task 7 describes UI wiring in prose with exact prop names and action shapes rather than full component listings; the dialogs copy an existing modal.

**Type consistency:** `Transition`/`TitleStyle`/`CaptionPos` string values match serde (`lowercase` / `camelCase`); op kinds lowercase on both sides; `setcuttransition.transition` is `Transition | null` ↔ `Option<Transition>`; `Edit::Cut.transition` omitted when `None` ↔ optional `transition?`; `DocState.transition` ↔ `EditorState.transition`; `CaptionWindow`/`Join` field names match between engine and the client mirror (`after`, `transition`).
