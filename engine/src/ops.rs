//! The operation log: what a collaborator did, in server order. A project's
//! edit list is the fold of its operations; undone operations are skipped.
//! Mirrors the rules in the client's `editor.ts` reducer.

use serde::{Deserialize, Serialize};

use crate::types::{Edit, Range};

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
}

/// Replay the log in order, skipping undone operations.
pub fn fold(ops: &[SeqOp]) -> ProjectDoc {
    let mut doc = ProjectDoc::default();
    for entry in ops.iter().filter(|o| !o.undone) {
        apply(&mut doc, &entry.op);
    }
    doc
}

fn inside(inner: Range, outer: Range) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

fn overlaps(a: Range, b: Range) -> bool {
    a.start < b.end && a.end > b.start
}

fn apply(doc: &mut ProjectDoc, op: &Op) {
    match op {
        Op::Cut { start, end } => {
            let cut = Range::new(*start, *end);
            // Deleting an overdubbed passage removes the overdub with it.
            doc.edits
                .retain(|e| !matches!(e, Edit::Overdub { .. } if inside(e.range(), cut)));
            doc.edits.push(Edit::Cut {
                start: *start,
                end: *end,
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
        Op::ApplyCuts { cuts } => doc.edits.extend(cuts.iter().map(|r| Edit::Cut {
            start: r.start,
            end: r.end,
        })),
        Op::RenameSpeaker { speaker, name } => {
            let i = *speaker as usize;
            if doc.speaker_names.len() <= i {
                doc.speaker_names.resize(i + 1, String::new());
            }
            doc.speaker_names[i] = name.trim().to_owned();
        }
        // Undo and redo only flip `undone` flags; the server does that when
        // it appends them, so here they are no-ops.
        Op::Undo { .. } | Op::Redo { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    end: 2.0
                },
                Edit::Cut {
                    start: 5.0,
                    end: 6.0
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
                end: 6.0
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
                end: 2.0
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
                end: 4.0
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
}
