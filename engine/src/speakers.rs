//! Speaker diarization: who is talking when. sherpa-onnx's offline
//! diarizer (pyannote segmentation + a speaker-embedding model) prints one
//! `start -- end speaker_NN` line per turn; we parse those and give every
//! transcript word the speaker whose turns overlap it most.

use serde::{Deserialize, Serialize};

use crate::types::Word;

/// One stretch of speech by one speaker, in source seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerTurn {
    pub start: f64,
    pub end: f64,
    pub speaker: u32,
}

/// Paths and tuning for a diarizer run.
#[derive(Debug, Clone)]
pub struct DiarizeOptions<'a> {
    pub segmentation_model: &'a str,
    pub embedding_model: &'a str,
    /// Cosine-distance cut-off for merging voices; higher means fewer speakers.
    pub cluster_threshold: f64,
    /// Force this many speakers instead of clustering by threshold.
    pub num_speakers: Option<u32>,
}

/// Arguments for `sherpa-onnx-offline-speaker-diarization` on a 16 kHz mono WAV.
pub fn diarize_args(wav: &str, opts: &DiarizeOptions) -> Vec<String> {
    let mut args = vec![
        format!("--segmentation.pyannote-model={}", opts.segmentation_model),
        format!("--embedding.model={}", opts.embedding_model),
    ];
    match opts.num_speakers {
        Some(n) => args.push(format!("--clustering.num-clusters={n}")),
        None => args.push(format!(
            "--clustering.cluster-threshold={}",
            opts.cluster_threshold
        )),
    }
    args.push(wav.to_owned());
    args
}

/// Parse the diarizer's `0.082 -- 0.976 speaker_01` lines, ignoring the
/// progress and config chatter around them. Speakers are renumbered in order
/// of first appearance, so the first voice heard is always speaker 0.
pub fn parse_diarization(output: &str) -> Vec<SpeakerTurn> {
    let mut raw: Vec<(f64, f64, u32)> = output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let start = parts.next()?.parse::<f64>().ok()?;
            if parts.next()? != "--" {
                return None;
            }
            let end = parts.next()?.parse::<f64>().ok()?;
            let speaker = parts.next()?.strip_prefix("speaker_")?.parse().ok()?;
            (end > start && parts.next().is_none()).then_some((start, end, speaker))
        })
        .collect();
    raw.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut order: Vec<u32> = Vec::new();
    raw.into_iter()
        .map(|(start, end, speaker)| {
            let id = match order.iter().position(|&s| s == speaker) {
                Some(i) => i,
                None => {
                    order.push(speaker);
                    order.len() - 1
                }
            };
            SpeakerTurn {
                start,
                end,
                speaker: id as u32,
            }
        })
        .collect()
}

/// Speaker for each word: the one whose turns overlap the word longest. A
/// word in no turn (diarization skips very short or quiet speech) takes the
/// nearest turn's speaker. `None` only when there are no turns at all.
pub fn assign_speakers(words: &[Word], turns: &[SpeakerTurn]) -> Vec<Option<u32>> {
    words
        .iter()
        .map(|word| {
            let mut best: Option<(f64, u32)> = None;
            for turn in turns {
                let overlap = word.end.min(turn.end) - word.start.max(turn.start);
                if overlap > 0.0 && best.is_none_or(|(o, _)| overlap > o) {
                    best = Some((overlap, turn.speaker));
                }
            }
            best.map(|(_, s)| s).or_else(|| {
                let mid = (word.start + word.end) / 2.0;
                turns
                    .iter()
                    .min_by(|a, b| distance(mid, a).total_cmp(&distance(mid, b)))
                    .map(|t| t.speaker)
            })
        })
        .collect()
}

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

fn distance(t: f64, turn: &SpeakerTurn) -> f64 {
    if t < turn.start {
        turn.start - t
    } else if t > turn.end {
        t - turn.end
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start: f64, end: f64) -> Word {
        Word {
            id: format!("w{start}"),
            text: "x".into(),
            start,
            end,
        }
    }

    fn turn(start: f64, end: f64, speaker: u32) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker,
        }
    }

    const OUTPUT: &str = "\
progress 100.00%
Duration : 60.024 s
OfflineSpeakerDiarizationConfig(min_duration_on=0.3)
Started
0.082 -- 0.976 speaker_01
1.769 -- 8.131 speaker_00
9.042 -- 14.830 speaker_01
17.412 -- 18.442 speaker_02
";

    #[test]
    fn parses_turns_and_renumbers_by_first_appearance() {
        let turns = parse_diarization(OUTPUT);
        assert_eq!(
            turns,
            vec![
                turn(0.082, 0.976, 0),
                turn(1.769, 8.131, 1),
                turn(9.042, 14.83, 0),
                turn(17.412, 18.442, 2),
            ]
        );
    }

    #[test]
    fn ignores_malformed_and_empty_lines() {
        let turns = parse_diarization("1 -- 2 speaker_x\n3 - 4 speaker_0\n5 -- 5 speaker_0\n");
        assert!(turns.is_empty());
    }

    #[test]
    fn sorts_out_of_order_turns() {
        let turns = parse_diarization("5 -- 6 speaker_03\n1 -- 2 speaker_07\n");
        assert_eq!(turns, vec![turn(1.0, 2.0, 0), turn(5.0, 6.0, 1)]);
    }

    #[test]
    fn words_take_the_speaker_with_most_overlap() {
        let turns = [turn(0.0, 1.2, 0), turn(1.1, 3.0, 1)];
        let words = [word(0.2, 0.6), word(1.0, 1.6), word(2.0, 2.4)];
        assert_eq!(
            assign_speakers(&words, &turns),
            vec![Some(0), Some(1), Some(1)]
        );
    }

    #[test]
    fn words_outside_every_turn_take_the_nearest() {
        let turns = [turn(0.0, 1.0, 0), turn(5.0, 6.0, 1)];
        let words = [word(1.2, 1.4), word(4.5, 4.8), word(7.0, 7.5)];
        assert_eq!(
            assign_speakers(&words, &turns),
            vec![Some(0), Some(1), Some(1)]
        );
    }

    #[test]
    fn no_turns_means_no_speakers() {
        assert_eq!(assign_speakers(&[word(0.0, 1.0)], &[]), vec![None]);
    }

    #[test]
    fn args_use_threshold_unless_a_count_is_forced() {
        let mut opts = DiarizeOptions {
            segmentation_model: "seg.onnx",
            embedding_model: "emb.onnx",
            cluster_threshold: 0.9,
            num_speakers: None,
        };
        let args = diarize_args("a.wav", &opts);
        assert!(args.contains(&"--clustering.cluster-threshold=0.9".to_owned()));
        assert_eq!(args.last().unwrap(), "a.wav");
        opts.num_speakers = Some(2);
        let args = diarize_args("a.wav", &opts);
        assert!(args.contains(&"--clustering.num-clusters=2".to_owned()));
    }

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
}
