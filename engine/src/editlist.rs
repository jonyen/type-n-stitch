//! Edit-list math: turning a list of cuts and overdubs into a timeline and
//! mapping times between the source and the rendered output.

use crate::types::{Edit, Range};

/// Two ranges closer than this are treated as touching.
pub const EPS: f64 = 1e-6;

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

/// The rendered output, piece by piece: kept source ranges split around
/// overdubs, and overdub ranges holding their first frame for the length of
/// the synthesized audio. Where an overdub overlaps a cut, the overdub wins.
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

    // Source pieces: kept ranges minus every overdub range.
    let overdub_holes = normalize_cuts(&overdubs.iter().map(|(_, r, _)| *r).collect::<Vec<_>>());
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
                .map(move |r| {
                    (
                        Range::new(r.start.max(kept.start), r.end),
                        SegmentKind::Source,
                    )
                })
        })
        .collect();

    // Overdub pieces sit at their source position but take `audio_duration`.
    pieces.extend(
        overdubs
            .iter()
            .map(|(index, r, _)| (*r, SegmentKind::Overdub { index: *index })),
    );
    pieces.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));

    let mut out_cursor = 0.0;
    pieces
        .into_iter()
        .map(|(source, kind)| {
            let out_len = match kind {
                SegmentKind::Source => source.len(),
                SegmentKind::Overdub { index } => match &edits[index] {
                    Edit::Overdub { audio_duration, .. } => *audio_duration,
                    _ => unreachable!("overdub index points at a cut"),
                },
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

/// Total length of the rendered output.
pub fn output_duration(timeline: &[Segment]) -> f64 {
    timeline.last().map_or(0.0, |s| s.output.end)
}

/// Map a source time to the output. Times inside a cut map to the moment the
/// cut closes; times inside an overdub map to where that overdub begins.
pub fn source_to_output_time(t: f64, timeline: &[Segment]) -> f64 {
    for seg in timeline {
        if seg.source.contains(t) {
            return match seg.kind {
                SegmentKind::Source => seg.output.start + (t - seg.source.start),
                SegmentKind::Overdub { .. } => seg.output.start,
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
                SegmentKind::Overdub { .. } => seg.source.start,
            };
        }
    }
    timeline.last().map_or(0.0, |s| s.source.end)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
