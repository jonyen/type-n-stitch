//! The operation log: what a collaborator did, in server order. A project's
//! edit list is the fold of its operations; undone operations are skipped.
//! Mirrors the rules in the client's `editor.ts` reducer.

use serde::{Deserialize, Serialize};

use crate::editlist::{piece_starts, EPS};
use crate::types::{default_duck, CaptionPos, Edit, Frame, Range, Source, TitleStyle, Transition};

/// Upper bound on speaker indices. Diarization never finds this many voices;
/// the cap exists so a stored `RenameSpeaker` cannot ask the fold for an
/// arbitrarily large name list. The server rejects anything at or above it.
pub const MAX_SPEAKERS: usize = 64;

/// One change submitted by a client. `Undo` and `Redo` never appear in a
/// fold's output; the server marks their targets so `fold` can skip them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Op {
    Cut {
        start: f64,
        end: f64,
    },
    #[serde(rename_all = "camelCase")]
    Overdub {
        start: f64,
        end: f64,
        text: String,
        audio_url: String,
        audio_duration: f64,
    },
    #[serde(rename_all = "camelCase")]
    ApplyCuts {
        cuts: Vec<Range>,
    },
    #[serde(rename_all = "camelCase")]
    RenameSpeaker {
        speaker: u32,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    Undo {
        target_seq: i64,
    },
    #[serde(rename_all = "camelCase")]
    Redo {
        target_seq: i64,
    },
    #[serde(rename_all = "camelCase")]
    AddTitle {
        at: f64,
        duration: f64,
        text: String,
        subtitle: Option<String>,
        style: TitleStyle,
    },
    #[serde(rename_all = "camelCase")]
    EditTitle {
        at: f64,
        duration: f64,
        text: String,
        subtitle: Option<String>,
        style: TitleStyle,
    },
    RemoveTitle {
        at: f64,
    },
    AddCaption {
        start: f64,
        end: f64,
        text: String,
        position: CaptionPos,
    },
    RemoveCaption {
        start: f64,
    },
    SetTransition {
        transition: Transition,
    },
    SetCutTransition {
        start: f64,
        transition: Option<Transition>,
    },
    Split {
        at: f64,
    },
    Unsplit {
        at: f64,
    },
    /// `before: None` moves the piece to the end.
    Move {
        piece: f64,
        before: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    AddBroll {
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
    },
    RemoveBroll {
        start: f64,
    },
    #[serde(rename_all = "camelCase")]
    AddAudio {
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        gain: f64,
        #[serde(default = "default_duck")]
        duck: bool,
    },
    EditAudio {
        start: f64,
        gain: f64,
        duck: bool,
    },
    RemoveAudio {
        start: f64,
    },
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
}

/// An operation as stored: its position in the log, who sent it, and
/// whether a later undo removed it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeqOp {
    pub seq: i64,
    pub author_id: String,
    pub op: Op,
    pub undone: bool,
}

/// Everything the fold produces: the edit list the engine renders plus
/// project-level state that is not an edit.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDoc {
    pub edits: Vec<Edit>,
    /// Display names by speaker index; empty strings mean "unnamed".
    pub speaker_names: Vec<String>,
    #[serde(default)]
    pub transition: Transition,
    /// Source instants where a piece is split, ascending and deduplicated.
    #[serde(default)]
    pub splits: Vec<f64>,
    /// Explicit piece order, front to back, by piece start. Entries for
    /// pieces the fold no longer knows about are left in place; callers that
    /// build a timeline ignore them.
    #[serde(default)]
    pub order: Vec<f64>,
    /// Videos appended by `AddSource`, in stitched order. The project's own
    /// media is source 0 and is *not* listed here; see `all_sources`.
    #[serde(default)]
    pub sources: Vec<Source>,
}

/// Replay the log in order, skipping undone operations.
pub fn fold(ops: &[SeqOp]) -> ProjectDoc {
    let mut doc = ProjectDoc::default();
    for entry in ops.iter().filter(|o| !o.undone) {
        apply_op(&mut doc, &entry.op);
    }
    doc
}

fn inside(inner: Range, outer: Range) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

fn overlaps(a: Range, b: Range) -> bool {
    a.start < b.end && a.end > b.start
}

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

pub fn apply_op(doc: &mut ProjectDoc, op: &Op) {
    match op {
        Op::Cut { start, end } => {
            let cut = Range::new(*start, *end);
            // Deleting an overdubbed passage removes the overdub with it.
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if inside(e.range(), cut)));
            doc.edits.push(Edit::Cut {
                start: *start,
                end: *end,
                transition: None,
            });
        }
        Op::Overdub {
            start,
            end,
            text,
            audio_url,
            audio_duration,
        } => {
            let span = Range::new(*start, *end);
            // A new overdub replaces any it overlaps.
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if overlaps(e.range(), span)));
            doc.edits.push(Edit::Overdub {
                start: *start,
                end: *end,
                text: text.clone(),
                audio_url: audio_url.clone(),
                audio_duration: *audio_duration,
            });
        }
        Op::ApplyCuts { cuts } => {
            for r in cuts {
                // The same rule as a single cut: an overdub wholly inside goes with it.
                doc.edits
                    .retain(|e| !matches!(e, Edit::Overdub { .. } if inside(e.range(), *r)));
                doc.edits.push(Edit::Cut {
                    start: r.start,
                    end: r.end,
                    transition: None,
                });
            }
        }
        Op::RenameSpeaker { speaker, name } => {
            let i = *speaker as usize;
            // Defensive: the server validates this before storing an op, but an
            // already-stored op must never make the fold unrepresentable.
            if i >= MAX_SPEAKERS {
                return;
            }
            if doc.speaker_names.len() <= i {
                doc.speaker_names.resize(i + 1, String::new());
            }
            doc.speaker_names[i] = name.trim().to_owned();
        }
        // Undo and redo only flip `undone` flags; the server does that when
        // it appends them, so here they are no-ops.
        Op::Undo { .. } | Op::Redo { .. } => {}
        Op::AddTitle {
            at,
            duration,
            text,
            subtitle,
            style,
        } => doc.edits.push(Edit::Title {
            at: *at,
            duration: *duration,
            text: text.clone(),
            subtitle: subtitle.clone(),
            style: *style,
        }),
        Op::EditTitle {
            at,
            duration,
            text,
            subtitle,
            style,
        } => {
            if let Some(Edit::Title {
                duration: d,
                text: t,
                subtitle: s,
                style: st,
                ..
            }) = doc
                .edits
                .iter_mut()
                .find(|e| matches!(e, Edit::Title { at: a, .. } if (a - at).abs() < EPS))
            {
                *d = *duration;
                *t = text.clone();
                *s = subtitle.clone();
                *st = *style;
            }
        }
        Op::RemoveTitle { at } => doc
            .edits
            .retain(|e| !matches!(e, Edit::Title { at: a, .. } if (a - at).abs() < EPS)),
        Op::AddCaption {
            start,
            end,
            text,
            position,
        } => {
            let span = Range::new(*start, *end);
            // A new caption replaces any it overlaps, like an overdub.
            doc.edits
                .retain(|e| !matches!(e, Edit::Caption { .. } if overlaps(e.range(), span)));
            doc.edits.push(Edit::Caption {
                start: *start,
                end: *end,
                text: text.clone(),
                position: *position,
            });
        }
        Op::RemoveCaption { start } => doc
            .edits
            .retain(|e| !matches!(e, Edit::Caption { start: s, .. } if (s - start).abs() < EPS)),
        Op::SetTransition { transition } => doc.transition = *transition,
        Op::SetCutTransition { start, transition } => {
            for e in &mut doc.edits {
                if let Edit::Cut {
                    start: s,
                    transition: t,
                    ..
                } = e
                {
                    if (*s - start).abs() < EPS {
                        *t = *transition;
                    }
                }
            }
        }
        Op::Split { at } => add_split(doc, *at),
        // A join between two sources stays split, so no piece spans two files.
        Op::Unsplit { at } => {
            if !is_join(doc, *at) {
                doc.splits.retain(|s| (s - at).abs() >= EPS);
            }
        }
        Op::Move { piece, before } => {
            let current = piece_starts(&doc.edits, &doc.splits);
            let has = |list: &[f64], x: f64| list.iter().any(|s| (s - x).abs() < EPS);
            if !has(&current, *piece) {
                return;
            }
            // Materialise: live entries of `order` first, then every other
            // current piece in source order — the same rule the timeline uses.
            let mut effective: Vec<f64> = Vec::new();
            for &s in doc.order.iter().chain(current.iter()) {
                if has(&current, s) && !has(&effective, s) {
                    effective.push(s);
                }
            }
            effective.retain(|s| (s - piece).abs() >= EPS);
            let at = before
                .and_then(|b| effective.iter().position(|s| (s - b).abs() < EPS))
                .unwrap_or(effective.len());
            effective.insert(at, *piece);
            doc.order = effective;
        }
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
        Op::AddAudio {
            start,
            end,
            media,
            offset,
            gain,
            duck,
        } => doc.edits.push(Edit::Audio {
            start: *start,
            end: *end,
            media: media.clone(),
            offset: *offset,
            gain: *gain,
            duck: *duck,
        }),
        Op::EditAudio { start, gain, duck } => {
            for e in &mut doc.edits {
                if let Edit::Audio {
                    start: s,
                    gain: g,
                    duck: d,
                    ..
                } = e
                {
                    if (*s - start).abs() < EPS {
                        *g = *gain;
                        *d = *duck;
                    }
                }
            }
        }
        Op::RemoveAudio { start } => doc
            .edits
            .retain(|e| !matches!(e, Edit::Audio { start: s, .. } if (s - start).abs() < EPS)),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CaptionPos, Frame, Source, TitleStyle, Transition};

    fn op(seq: i64, op: Op) -> SeqOp {
        SeqOp {
            seq,
            author_id: "u1".into(),
            op,
            undone: false,
        }
    }

    fn undone(seq: i64, op: Op) -> SeqOp {
        SeqOp {
            undone: true,
            ..self::op(seq, op)
        }
    }

    fn cut(start: f64, end: f64) -> Op {
        Op::Cut { start, end }
    }

    fn overdub(start: f64, end: f64) -> Op {
        Op::Overdub {
            start,
            end,
            text: "x".into(),
            audio_url: "/data/m/overdub-0.wav".into(),
            audio_duration: 1.0,
        }
    }

    fn broll(start: f64, end: f64) -> Op {
        Op::AddBroll {
            start,
            end,
            media: "asset-1".into(),
            offset: 0.0,
        }
    }

    #[test]
    fn split_and_unsplit_keep_splits_sorted_and_unique() {
        let doc = fold(&[
            op(1, Op::Split { at: 5.0 }),
            op(2, Op::Split { at: 2.0 }),
            op(3, Op::Split { at: 5.0 }),
        ]);
        assert_eq!(doc.splits, vec![2.0, 5.0]);
        let doc = fold(&[op(1, Op::Split { at: 5.0 }), op(2, Op::Unsplit { at: 5.0 })]);
        assert!(doc.splits.is_empty());
    }

    #[test]
    fn move_materialises_the_order_and_ignores_unknown_pieces() {
        // Pieces: [0,2) [2,5) [5,..) via a split at 2 and a split at 5.
        let base = vec![op(1, Op::Split { at: 2.0 }), op(2, Op::Split { at: 5.0 })];
        let mut log = base.clone();
        log.push(op(
            3,
            Op::Move {
                piece: 5.0,
                before: Some(0.0),
            },
        ));
        assert_eq!(fold(&log).order, vec![5.0, 0.0, 2.0]);
        let mut log = base.clone();
        log.push(op(
            3,
            Op::Move {
                piece: 0.0,
                before: None,
            },
        ));
        assert_eq!(fold(&log).order, vec![2.0, 5.0, 0.0]);
        let mut log = base;
        log.push(op(
            3,
            Op::Move {
                piece: 9.0,
                before: None,
            },
        ));
        assert!(
            fold(&log).order.is_empty(),
            "an unknown piece moves nothing"
        );
    }

    #[test]
    fn a_cut_after_a_move_leaves_a_stale_order_entry_in_place() {
        let doc = fold(&[
            op(1, Op::Split { at: 2.0 }),
            op(
                2,
                Op::Move {
                    piece: 2.0,
                    before: Some(0.0),
                },
            ),
            op(3, cut(2.0, 3.0)),
        ]);
        // The fold never rewrites `order`; the timeline ignores 2.0 later.
        assert_eq!(doc.order, vec![2.0, 0.0]);
    }

    #[test]
    fn broll_replaces_overlaps_and_audio_edits_by_start() {
        let doc = fold(&[op(1, broll(1.0, 3.0)), op(2, broll(2.0, 4.0))]);
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
        let doc = fold(&[
            op(
                1,
                Op::AddAudio {
                    start: 0.0,
                    end: 8.0,
                    media: "m".into(),
                    offset: 0.0,
                    gain: 0.0,
                    duck: true,
                },
            ),
            op(
                2,
                Op::EditAudio {
                    start: 0.0,
                    gain: -6.0,
                    duck: false,
                },
            ),
        ]);
        assert_eq!(
            doc.edits,
            vec![Edit::Audio {
                start: 0.0,
                end: 8.0,
                media: "m".into(),
                offset: 0.0,
                gain: -6.0,
                duck: false
            }]
        );
        let doc = fold(&[
            op(
                1,
                Op::AddAudio {
                    start: 0.0,
                    end: 8.0,
                    media: "m".into(),
                    offset: 0.0,
                    gain: 0.0,
                    duck: true,
                },
            ),
            op(2, Op::RemoveAudio { start: 0.0 }),
            op(3, broll(1.0, 2.0)),
            op(4, Op::RemoveBroll { start: 1.0 }),
        ]);
        assert!(doc.edits.is_empty());
    }

    #[test]
    fn new_ops_round_trip_json_with_defaults() {
        let op: Op =
            serde_json::from_str(r#"{"kind":"addaudio","start":0,"end":1,"media":"m"}"#).unwrap();
        assert_eq!(
            op,
            Op::AddAudio {
                start: 0.0,
                end: 1.0,
                media: "m".into(),
                offset: 0.0,
                gain: 0.0,
                duck: true
            }
        );
        let op: Op = serde_json::from_str(r#"{"kind":"move","piece":2,"before":null}"#).unwrap();
        assert_eq!(
            op,
            Op::Move {
                piece: 2.0,
                before: None
            }
        );
        let doc: ProjectDoc = serde_json::from_str(r#"{"edits":[],"speakerNames":[]}"#).unwrap();
        assert!(doc.splits.is_empty() && doc.order.is_empty());
    }

    #[test]
    fn empty_log_is_empty_doc() {
        assert_eq!(fold(&[]), ProjectDoc::default());
    }

    #[test]
    fn cuts_append_in_order() {
        let doc = fold(&[op(1, cut(1.0, 2.0)), op(2, cut(5.0, 6.0))]);
        assert_eq!(
            doc.edits,
            vec![
                Edit::Cut {
                    start: 1.0,
                    end: 2.0,
                    transition: None
                },
                Edit::Cut {
                    start: 5.0,
                    end: 6.0,
                    transition: None
                }
            ]
        );
    }

    #[test]
    fn undone_ops_are_skipped() {
        let doc = fold(&[undone(1, cut(1.0, 2.0)), op(2, cut(5.0, 6.0))]);
        assert_eq!(
            doc.edits,
            vec![Edit::Cut {
                start: 5.0,
                end: 6.0,
                transition: None
            }]
        );
    }

    #[test]
    fn undo_and_redo_rows_produce_no_edits() {
        let doc = fold(&[
            op(1, cut(1.0, 2.0)),
            op(2, Op::Undo { target_seq: 1 }),
            op(3, Op::Redo { target_seq: 2 }),
        ]);
        // The server flips `undone` on row 1; the fold only reads that flag.
        assert_eq!(
            doc.edits,
            vec![Edit::Cut {
                start: 1.0,
                end: 2.0,
                transition: None
            }]
        );
    }

    #[test]
    fn cut_removes_overdubs_inside_it() {
        let doc = fold(&[op(1, overdub(2.0, 3.0)), op(2, cut(1.0, 4.0))]);
        assert_eq!(
            doc.edits,
            vec![Edit::Cut {
                start: 1.0,
                end: 4.0,
                transition: None
            }]
        );
    }

    #[test]
    fn cut_keeps_overdubs_that_only_overlap() {
        let doc = fold(&[op(1, overdub(2.0, 5.0)), op(2, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 2);
        assert!(matches!(doc.edits[0], Edit::Overdub { start, .. } if start == 2.0));
    }

    #[test]
    fn overdub_replaces_overlapping_overdubs() {
        let doc = fold(&[op(1, overdub(2.0, 4.0)), op(2, overdub(3.0, 5.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(
            matches!(doc.edits[0], Edit::Overdub { start, end, .. } if start == 3.0 && end == 5.0)
        );
    }

    #[test]
    fn undoing_a_cut_restores_the_overdub_it_removed() {
        let doc = fold(&[op(1, overdub(2.0, 3.0)), undone(2, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Overdub { .. }));
    }

    #[test]
    fn apply_cuts_appends_every_cut() {
        let doc = fold(&[op(
            1,
            Op::ApplyCuts {
                cuts: vec![Range::new(1.0, 2.0), Range::new(3.0, 4.0)],
            },
        )]);
        assert_eq!(doc.edits.len(), 2);
    }

    #[test]
    fn apply_cuts_drops_an_overdub_it_covers_like_a_single_cut() {
        let doc = fold(&[
            op(1, overdub(2.0, 3.0)),
            op(
                2,
                Op::ApplyCuts {
                    cuts: vec![Range::new(1.0, 4.0), Range::new(6.0, 7.0)],
                },
            ),
        ]);
        assert!(!doc.edits.iter().any(|e| matches!(e, Edit::Overdub { .. })));
        assert_eq!(doc.edits.len(), 2);
        // An overdub only partly covered stays.
        let doc = fold(&[
            op(1, overdub(2.0, 3.0)),
            op(
                2,
                Op::ApplyCuts {
                    cuts: vec![Range::new(2.5, 4.0)],
                },
            ),
        ]);
        assert!(doc.edits.iter().any(|e| matches!(e, Edit::Overdub { .. })));
    }

    #[test]
    fn rename_speaker_grows_the_name_list() {
        let doc = fold(&[
            op(
                1,
                Op::RenameSpeaker {
                    speaker: 2,
                    name: "Ada".into(),
                },
            ),
            op(
                2,
                Op::RenameSpeaker {
                    speaker: 0,
                    name: "Bob".into(),
                },
            ),
        ]);
        assert_eq!(doc.speaker_names, vec!["Bob", "", "Ada"]);
    }

    #[test]
    fn rename_speaker_out_of_range_is_ignored() {
        let doc = fold(&[
            op(
                1,
                Op::RenameSpeaker {
                    speaker: 0,
                    name: "Ada".into(),
                },
            ),
            op(
                2,
                Op::RenameSpeaker {
                    speaker: u32::MAX,
                    name: "Nobody".into(),
                },
            ),
            op(
                3,
                Op::RenameSpeaker {
                    speaker: MAX_SPEAKERS as u32,
                    name: "Nobody".into(),
                },
            ),
        ]);
        assert_eq!(doc.speaker_names, vec!["Ada"]);
    }

    #[test]
    fn two_authors_interleave_by_seq() {
        let a = SeqOp {
            seq: 1,
            author_id: "a".into(),
            op: cut(1.0, 2.0),
            undone: false,
        };
        let b = SeqOp {
            seq: 2,
            author_id: "b".into(),
            op: cut(3.0, 4.0),
            undone: false,
        };
        let doc = fold(&[a, b]);
        assert_eq!(doc.edits.len(), 2);
    }

    #[test]
    fn op_json_round_trips_with_camel_case() {
        let json = serde_json::to_value(Op::Undo { target_seq: 7 }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "undo", "targetSeq": 7 }));
        let back: Op = serde_json::from_value(json).unwrap();
        assert_eq!(back, Op::Undo { target_seq: 7 });
        let cuts: Op =
            serde_json::from_str(r#"{"kind":"applycuts","cuts":[{"start":1,"end":2}]}"#).unwrap();
        assert!(matches!(cuts, Op::ApplyCuts { .. }));
    }

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
        assert!(
            matches!(doc.edits[0], Edit::Title { at, duration, .. } if at == 5.0 && duration == 3.0)
        );

        let doc = fold(&[
            op(1, title(5.0)),
            op(
                2,
                Op::EditTitle {
                    at: 5.0,
                    duration: 2.0,
                    text: "Part two".into(),
                    subtitle: Some("sub".into()),
                    style: TitleStyle::Accent,
                },
            ),
        ]);
        assert_eq!(doc.edits.len(), 1);
        assert!(
            matches!(&doc.edits[0], Edit::Title { duration, text, subtitle: Some(s), style: TitleStyle::Accent, .. }
            if *duration == 2.0 && text == "Part two" && s == "sub")
        );

        let doc = fold(&[
            op(1, title(5.0)),
            op(2, title(5.0)),
            op(3, Op::RemoveTitle { at: 5.0 }),
        ]);
        assert!(
            doc.edits.is_empty(),
            "remove drops every title at that instant"
        );
        let doc = fold(&[
            op(1, title(5.0)),
            op(
                2,
                Op::EditTitle {
                    at: 9.0,
                    duration: 1.0,
                    text: "x".into(),
                    subtitle: None,
                    style: TitleStyle::Dark,
                },
            ),
        ]);
        assert_eq!(doc.edits.len(), 1, "editing a missing title is a no-op");
    }

    #[test]
    fn captions_replace_overlapping_ones_and_remove_by_start() {
        let cap = |start: f64, end: f64| Op::AddCaption {
            start,
            end,
            text: "name".into(),
            position: CaptionPos::BottomLeft,
        };
        let doc = fold(&[op(1, cap(1.0, 3.0)), op(2, cap(2.0, 4.0))]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Caption { start, .. } if start == 2.0));
        let doc = fold(&[
            op(1, cap(1.0, 3.0)),
            op(2, cap(5.0, 6.0)),
            op(3, Op::RemoveCaption { start: 1.0 }),
        ]);
        assert_eq!(doc.edits.len(), 1);
        assert!(matches!(doc.edits[0], Edit::Caption { start, .. } if start == 5.0));
    }

    #[test]
    fn cuts_keep_titles_and_captions_inside_them() {
        let cap = Op::AddCaption {
            start: 2.0,
            end: 3.0,
            text: "n".into(),
            position: CaptionPos::TopLeft,
        };
        let doc = fold(&[op(1, title(2.5)), op(2, cap), op(3, cut(1.0, 4.0))]);
        assert_eq!(doc.edits.len(), 3);
    }

    #[test]
    fn transitions_project_wide_and_per_cut() {
        let doc = fold(&[
            op(1, cut(1.0, 2.0)),
            op(
                2,
                Op::SetTransition {
                    transition: Transition::Dip,
                },
            ),
        ]);
        assert_eq!(doc.transition, Transition::Dip);
        assert!(matches!(
            doc.edits[0],
            Edit::Cut {
                transition: None,
                ..
            }
        ));
        let doc = fold(&[
            op(1, cut(1.0, 2.0)),
            op(
                2,
                Op::SetCutTransition {
                    start: 1.0,
                    transition: Some(Transition::None),
                },
            ),
        ]);
        assert!(matches!(
            doc.edits[0],
            Edit::Cut {
                transition: Some(Transition::None),
                ..
            }
        ));
        let doc = fold(&[op(
            1,
            Op::SetCutTransition {
                start: 9.0,
                transition: Some(Transition::Dip),
            },
        )]);
        assert!(doc.edits.is_empty(), "no cut at that start: no-op");
    }

    #[test]
    fn new_ops_and_edits_serialise_with_expected_tags() {
        let json = serde_json::to_value(Op::RemoveTitle { at: 2.0 }).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "removetitle", "at": 2.0 })
        );
        let json = serde_json::to_value(Op::SetCutTransition {
            start: 1.0,
            transition: Some(Transition::Dip),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "setcuttransition", "start": 1.0, "transition": "dip" })
        );
        let json = serde_json::to_value(Edit::Cut {
            start: 1.0,
            end: 2.0,
            transition: None,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "cut", "start": 1.0, "end": 2.0 }),
            "None is omitted"
        );
        let old: Edit = serde_json::from_str(r#"{"kind":"cut","start":1,"end":2}"#).unwrap();
        assert!(matches!(
            old,
            Edit::Cut {
                transition: None,
                ..
            }
        ));
        let t: Edit = serde_json::from_str(
            r#"{"kind":"title","at":1,"duration":2,"text":"T","subtitle":null,"style":"light"}"#,
        )
        .unwrap();
        assert!(matches!(
            t,
            Edit::Title {
                style: TitleStyle::Light,
                ..
            }
        ));
        let c: Edit = serde_json::from_str(
            r#"{"kind":"caption","start":1,"end":2,"text":"T","position":"bottomCenter"}"#,
        )
        .unwrap();
        assert!(matches!(
            c,
            Edit::Caption {
                position: CaptionPos::BottomCenter,
                ..
            }
        ));
        let doc: ProjectDoc = serde_json::from_str(r#"{"edits":[],"speakerNames":[]}"#).unwrap();
        assert_eq!(doc.transition, Transition::None);
    }

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
        let doc = fold(&[
            op(1, Op::Split { at: 4.0 }),
            undone(2, add_source(10.0, 5.0)),
        ]);
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
            op(
                3,
                Op::RemoveLayer {
                    track: 3,
                    start: 1.0,
                },
            ),
            op(
                4,
                Op::RemoveLayer {
                    track: 2,
                    start: 9.0,
                },
            ),
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
}
