//! Edit-list math: turning a list of cuts and overdubs into a timeline and
//! mapping times between the source and the rendered output.

use serde::{Deserialize, Serialize};

use crate::types::{Edit, Range, Transition, Word};

/// Two ranges closer than this are treated as touching.
pub const EPS: f64 = 1e-6;

/// Length of a dip-to-black on either side of a join.
pub const FADE: f64 = 0.25;

/// A transcript word's fate under the current edit list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WordStatus {
    Kept,
    Cut,
    Overdub,
}

/// Whether `r` fully covers `w`, with `EPS` slack on both edges — mirrors the
/// client's `covers` in `client/src/editlist.ts`.
fn covers(r: Range, w: &Word) -> bool {
    r.start <= w.start + EPS && r.end >= w.end - EPS
}

/// A word's status under `edits`: an `Overdub` covering it wins, then a
/// `Cut`, else it is kept. Titles and captions never affect status.
pub fn word_status(word: &Word, edits: &[Edit]) -> WordStatus {
    let overdubbed = edits.iter().any(|e| match e {
        Edit::Overdub { .. } => covers(e.range(), word),
        _ => false,
    });
    if overdubbed {
        return WordStatus::Overdub;
    }
    let cut = edits.iter().any(|e| match e {
        Edit::Cut { .. } => covers(e.range(), word),
        _ => false,
    });
    if cut {
        return WordStatus::Cut;
    }
    WordStatus::Kept
}

/// One piece of the rendered output, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Where this piece comes from in the source.
    pub source: Range,
    /// Where this piece lands in the output.
    pub output: Range,
    pub kind: SegmentKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SegmentKind {
    /// Source picture and sound, played through.
    Source,
    /// Frozen first frame of `source` with `edits[index]`'s audio.
    Overdub { index: usize },
    /// A title card: no source picture, `edits[index]`'s text for its duration.
    Title { index: usize },
}

/// Sort cuts and merge any that overlap or touch. Empty ranges are dropped.
pub fn normalize_cuts(cuts: &[Range]) -> Vec<Range> {
    let mut sorted: Vec<Range> = cuts.iter().copied().filter(|r| !r.is_empty()).collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut merged: Vec<Range> = Vec::with_capacity(sorted.len());
    for r in sorted {
        match merged.last_mut() {
            Some(last) if r.start <= last.end + EPS => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

fn cut_ranges(edits: &[Edit]) -> Vec<Range> {
    edits
        .iter()
        .filter_map(|e| match e {
            Edit::Cut { .. } => Some(e.range()),
            _ => None,
        })
        .collect()
}

/// Subtract `holes` (sorted, non-overlapping) from `[0, duration)`.
fn complement(duration: f64, holes: &[Range]) -> Vec<Range> {
    let mut kept = Vec::new();
    let mut cursor = 0.0;
    for hole in holes {
        let start = hole.start.clamp(0.0, duration);
        let end = hole.end.clamp(0.0, duration);
        if start > cursor + EPS {
            kept.push(Range::new(cursor, start));
        }
        cursor = cursor.max(end);
    }
    if duration > cursor + EPS {
        kept.push(Range::new(cursor, duration));
    }
    kept
}

/// Source ranges that survive the cuts. Overdub ranges are *kept*: they still
/// occupy time in the output, just with different sound.
pub fn kept_segments(duration: f64, edits: &[Edit]) -> Vec<Range> {
    complement(duration, &normalize_cuts(&cut_ranges(edits)))
}

/// Source starts of every piece (kept ranges divided at splits), ascending. The
/// last entry may equal the duration when a cut runs to the end; callers that
/// know the duration drop it.
pub fn piece_starts(edits: &[Edit], splits: &[f64]) -> Vec<f64> {
    let cuts = normalize_cuts(&cut_ranges(edits));
    let mut starts = Vec::new();
    let mut cursor = 0.0;
    for cut in &cuts {
        if cut.start > cursor + EPS {
            starts.push(cursor);
        }
        cursor = cursor.max(cut.end);
    }
    starts.push(cursor);
    for &s in splits {
        let in_cut = cuts.iter().any(|c| s > c.start - EPS && s < c.end + EPS);
        if !in_cut && !starts.iter().any(|x| (x - s).abs() < EPS) {
            starts.push(s);
        }
    }
    starts.sort_by(f64::total_cmp);
    starts
}

/// The rendered output, piece by piece: kept source ranges split around
/// overdubs and title instants, overdub ranges holding their first frame for
/// the length of the synthesized audio, and title cards holding for their own
/// duration. Where an overdub overlaps a cut, the overdub wins.
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

    // Overdub pieces sit at their source position but take `audio_duration`.
    pieces.extend(
        overdubs
            .iter()
            .map(|(index, r, _)| (*r, SegmentKind::Overdub { index: *index })),
    );
    // Title cards are zero-length in the source and stretch the output.
    pieces.extend(
        titles
            .iter()
            .map(|(index, at, _)| (Range::new(*at, *at), SegmentKind::Title { index: *index })),
    );
    // Stable: titles at one instant keep edit order; a title precedes an
    // overdub or source piece starting at the same instant.
    pieces.sort_by(|a, b| {
        a.0.start
            .total_cmp(&b.0.start)
            .then(rank(&a.1).cmp(&rank(&b.1)))
    });

    let mut out_cursor = 0.0;
    pieces
        .into_iter()
        .map(|(source, kind)| {
            let out_len = match kind {
                SegmentKind::Source => source.len(),
                SegmentKind::Overdub { index } | SegmentKind::Title { index } => {
                    match &edits[index] {
                        Edit::Overdub { audio_duration, .. } => *audio_duration,
                        Edit::Title { duration, .. } => *duration,
                        _ => unreachable!("segment index points at a non-hold edit"),
                    }
                }
            };
            let output = Range::new(out_cursor, out_cursor + out_len);
            out_cursor += out_len;
            Segment {
                source,
                output,
                kind,
            }
        })
        .collect()
}

/// Ordering of pieces that begin at the same instant.
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

/// Total length of the rendered output.
pub fn output_duration(timeline: &[Segment]) -> f64 {
    timeline.last().map_or(0.0, |s| s.output.end)
}

/// Map a source time to the output. Times inside a cut map to the moment the
/// cut closes; times inside an overdub map to where that overdub begins.
pub fn source_to_output_time(t: f64, timeline: &[Segment]) -> f64 {
    for seg in timeline {
        if let SegmentKind::Title { .. } = seg.kind {
            // A title is zero-length in the source: only the exact instant
            // lands on it, and everything else reads straight through.
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

/// Map an output time back to the source frame that is on screen. Inside an
/// overdub that is always the frozen first frame.
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
                    let Edit::Caption { start, end, .. } = e else {
                        return None;
                    };
                    let cap = Range::new(*start, *end);
                    match seg.kind {
                        SegmentKind::Title { .. } => None,
                        SegmentKind::Overdub { .. } => (cap.start < seg.source.end
                            && cap.end > seg.source.start)
                            .then_some(CaptionWindow {
                                index,
                                start: 0.0,
                                end: seg.output.len(),
                            }),
                        SegmentKind::Source => {
                            let a = cap.start.max(seg.source.start);
                            let b = cap.end.min(seg.source.end);
                            (b > a + EPS).then_some(CaptionWindow {
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
                        Edit::Cut {
                            start,
                            transition: Some(t),
                            ..
                        } if (start - a.source.end).abs() < EPS => Some(*t),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CaptionPos, TitleStyle, Transition};

    fn cut(start: f64, end: f64) -> Edit {
        Edit::Cut {
            start,
            end,
            transition: None,
        }
    }

    fn overdub(start: f64, end: f64, audio_duration: f64) -> Edit {
        Edit::Overdub {
            start,
            end,
            text: "replacement".into(),
            audio_url: "/data/x/od.wav".into(),
            audio_duration,
        }
    }

    fn r(start: f64, end: f64) -> Range {
        Range::new(start, end)
    }

    fn title(at: f64, duration: f64) -> Edit {
        Edit::Title {
            at,
            duration,
            text: "T".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }
    }

    fn caption(start: f64, end: f64) -> Edit {
        Edit::Caption {
            start,
            end,
            text: "c".into(),
            position: CaptionPos::BottomLeft,
        }
    }

    fn kinds(tl: &[Segment]) -> Vec<&SegmentKind> {
        tl.iter().map(|s| &s.kind).collect()
    }

    #[test]
    fn word_status_mirrors_the_client() {
        let w = crate::types::Word {
            id: "w".into(),
            text: "x".into(),
            start: 1.0,
            end: 1.5,
        };
        assert_eq!(word_status(&w, &[]), WordStatus::Kept);
        assert_eq!(word_status(&w, &[cut(0.5, 2.0)]), WordStatus::Cut);
        assert_eq!(
            word_status(&w, &[cut(0.5, 2.0), overdub(1.0, 1.5, 1.0)]),
            WordStatus::Overdub
        );
        assert_eq!(
            word_status(&w, &[cut(1.2, 2.0)]),
            WordStatus::Kept,
            "partial cover is not cut"
        );
    }

    #[test]
    fn normalize_sorts_and_merges_overlapping_and_touching() {
        let cuts = [r(5.0, 6.0), r(1.0, 2.0), r(1.5, 3.0), r(3.0, 4.0)];
        assert_eq!(normalize_cuts(&cuts), vec![r(1.0, 4.0), r(5.0, 6.0)]);
    }

    #[test]
    fn normalize_drops_empty_and_inverted_ranges() {
        assert_eq!(normalize_cuts(&[r(2.0, 2.0), r(3.0, 1.0)]), vec![]);
        assert_eq!(normalize_cuts(&[]), vec![]);
    }

    #[test]
    fn kept_segments_without_edits_is_whole_media() {
        assert_eq!(kept_segments(10.0, &[]), vec![r(0.0, 10.0)]);
    }

    #[test]
    fn kept_segments_are_complement_of_cuts() {
        let edits = [cut(2.0, 3.0), cut(8.0, 10.0), cut(2.5, 4.0)];
        assert_eq!(kept_segments(10.0, &edits), vec![r(0.0, 2.0), r(4.0, 8.0)]);
    }

    #[test]
    fn kept_segments_handles_cut_at_start_and_beyond_end() {
        let edits = [cut(0.0, 1.0), cut(9.0, 12.0)];
        assert_eq!(kept_segments(10.0, &edits), vec![r(1.0, 9.0)]);
    }

    #[test]
    fn kept_segments_everything_cut_is_empty() {
        assert_eq!(kept_segments(10.0, &[cut(0.0, 10.0)]), vec![]);
    }

    #[test]
    fn kept_segments_ignore_overdubs() {
        assert_eq!(
            kept_segments(10.0, &[overdub(2.0, 3.0, 1.5)]),
            vec![r(0.0, 10.0)]
        );
    }

    #[test]
    fn timeline_splits_source_around_overdub_and_stretches_it() {
        let edits = [overdub(2.0, 3.0, 2.5)];
        let tl = timeline(10.0, &edits);
        assert_eq!(
            tl,
            vec![
                Segment {
                    source: r(0.0, 2.0),
                    output: r(0.0, 2.0),
                    kind: SegmentKind::Source
                },
                Segment {
                    source: r(2.0, 3.0),
                    output: r(2.0, 4.5),
                    kind: SegmentKind::Overdub { index: 0 },
                },
                Segment {
                    source: r(3.0, 10.0),
                    output: r(4.5, 11.5),
                    kind: SegmentKind::Source
                },
            ]
        );
        assert_eq!(output_duration(&tl), 11.5);
    }

    #[test]
    fn timeline_with_cut_then_overdub() {
        let edits = [cut(1.0, 2.0), overdub(5.0, 6.0, 0.5)];
        let tl = timeline(8.0, &edits);
        let sources: Vec<Range> = tl.iter().map(|s| s.source).collect();
        let outputs: Vec<Range> = tl.iter().map(|s| s.output).collect();
        assert_eq!(
            sources,
            vec![r(0.0, 1.0), r(2.0, 5.0), r(5.0, 6.0), r(6.0, 8.0)]
        );
        assert_eq!(
            outputs,
            vec![r(0.0, 1.0), r(1.0, 4.0), r(4.0, 4.5), r(4.5, 6.5)]
        );
    }

    #[test]
    fn timeline_overdub_wins_over_overlapping_cut() {
        let edits = [cut(2.5, 4.0), overdub(2.0, 3.0, 1.0)];
        let tl = timeline(6.0, &edits);
        let sources: Vec<Range> = tl.iter().map(|s| s.source).collect();
        assert_eq!(sources, vec![r(0.0, 2.0), r(2.0, 3.0), r(4.0, 6.0)]);
        assert_eq!(tl[1].kind, SegmentKind::Overdub { index: 1 });
    }

    #[test]
    fn source_to_output_skips_cuts() {
        let tl = timeline(10.0, &[cut(2.0, 4.0)]);
        assert_eq!(source_to_output_time(1.0, &tl), 1.0);
        assert_eq!(source_to_output_time(3.0, &tl), 2.0); // inside the cut
        assert_eq!(source_to_output_time(5.0, &tl), 3.0);
        assert_eq!(source_to_output_time(10.0, &tl), 8.0);
    }

    #[test]
    fn output_to_source_reinserts_cuts() {
        let tl = timeline(10.0, &[cut(2.0, 4.0)]);
        assert_eq!(output_to_source_time(1.0, &tl), 1.0);
        assert_eq!(output_to_source_time(2.0, &tl), 4.0);
        assert_eq!(output_to_source_time(7.9, &tl), 9.9);
        assert_eq!(output_to_source_time(8.0, &tl), 10.0);
    }

    #[test]
    fn remaps_through_an_overdub_freeze() {
        let tl = timeline(10.0, &[overdub(2.0, 3.0, 4.0)]);
        assert_eq!(source_to_output_time(2.5, &tl), 2.0);
        assert_eq!(source_to_output_time(3.0, &tl), 6.0);
        assert_eq!(output_to_source_time(4.0, &tl), 2.0); // frozen frame
        assert_eq!(output_to_source_time(6.5, &tl), 3.5);
    }

    #[test]
    fn remap_is_inverse_on_kept_source() {
        let tl = timeline(
            20.0,
            &[cut(1.0, 2.0), overdub(5.0, 7.0, 1.0), cut(15.0, 20.0)],
        );
        for t in [0.0, 0.5, 2.0, 4.9, 7.0, 10.0, 14.99] {
            let back = output_to_source_time(source_to_output_time(t, &tl), &tl);
            assert!((back - t).abs() < 1e-9, "{t} -> {back}");
        }
    }

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
        assert_eq!(
            kinds(&tl),
            vec![
                &SegmentKind::Source,
                &SegmentKind::Title { index: 1 },
                &SegmentKind::Source
            ]
        );
        assert_eq!(tl[2].source, r(4.0, 10.0));
        let tl = timeline(10.0, &[cut(2.0, 4.0), title(3.0, 1.0)]);
        assert_eq!(
            kinds(&tl),
            vec![
                &SegmentKind::Source,
                &SegmentKind::Title { index: 1 },
                &SegmentKind::Source
            ]
        );
        assert_eq!(output_duration(&tl), 9.0);
    }

    #[test]
    fn two_titles_at_one_instant_keep_edit_order_and_a_title_precedes_an_overdub_there() {
        let tl = timeline(
            10.0,
            &[title(5.0, 1.0), title(5.0, 2.0), overdub(5.0, 6.0, 0.5)],
        );
        assert_eq!(
            kinds(&tl),
            vec![
                &SegmentKind::Source,
                &SegmentKind::Title { index: 0 },
                &SegmentKind::Title { index: 1 },
                &SegmentKind::Overdub { index: 2 },
                &SegmentKind::Source
            ]
        );
        assert_eq!(tl[2].output, r(6.0, 8.0));
    }

    #[test]
    fn title_at_start_and_end() {
        let tl = timeline(10.0, &[title(0.0, 1.0), title(10.0, 1.0)]);
        assert_eq!(
            kinds(&tl),
            vec![
                &SegmentKind::Title { index: 0 },
                &SegmentKind::Source,
                &SegmentKind::Title { index: 1 }
            ]
        );
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
        assert_eq!(
            w[0],
            vec![CaptionWindow {
                index: 1,
                start: 2.0,
                end: 3.0
            }]
        );
        assert_eq!(
            w[1],
            vec![CaptionWindow {
                index: 1,
                start: 0.0,
                end: 2.0
            }]
        );
    }

    #[test]
    fn caption_over_an_overdub_covers_its_whole_hold_and_titles_get_none() {
        let edits = [overdub(2.0, 3.0, 4.0), caption(2.5, 2.8), title(6.0, 1.0)];
        let tl = timeline(10.0, &edits);
        let w = caption_windows(&tl, &edits);
        assert_eq!(
            w[1],
            vec![CaptionWindow {
                index: 1,
                start: 0.0,
                end: 4.0
            }]
        );
        let title_i = tl
            .iter()
            .position(|s| matches!(s.kind, SegmentKind::Title { .. }))
            .unwrap();
        assert!(w[title_i].is_empty());
    }

    #[test]
    fn piece_starts_come_from_cuts_and_splits_outside_them() {
        assert_eq!(piece_starts(&[], &[]), vec![0.0]);
        assert_eq!(piece_starts(&[cut(2.0, 3.0)], &[]), vec![0.0, 3.0]);
        assert_eq!(
            piece_starts(&[cut(2.0, 3.0)], &[2.5, 5.0, 3.0, 0.0]),
            vec![0.0, 3.0, 5.0]
        );
        // A cut from zero: no piece starts at zero.
        assert_eq!(piece_starts(&[cut(0.0, 1.0)], &[]), vec![1.0]);
    }

    #[test]
    fn joins_use_cut_override_then_project_and_always_dip_around_titles() {
        let edits = [
            Edit::Cut {
                start: 2.0,
                end: 3.0,
                transition: Some(Transition::None),
            },
            cut(5.0, 6.0),
            title(8.0, 1.0),
            overdub(9.0, 9.5, 1.0),
        ];
        let tl = timeline(10.0, &edits);
        let j = joins(&tl, &edits, Transition::Dip);
        let by_after: Vec<(usize, Transition)> =
            j.iter().map(|j| (j.after, j.transition)).collect();
        // pieces: [0,2) [3,5) [6,8) T [8,9) OD [9.5,10)
        assert_eq!(
            by_after,
            vec![
                (0, Transition::None),
                (1, Transition::Dip),
                (2, Transition::Dip),
                (3, Transition::Dip),
                (4, Transition::None),
                (5, Transition::None)
            ]
        );
        assert_eq!(
            joins(&tl, &edits, Transition::None)[1].transition,
            Transition::None
        );
    }
}
