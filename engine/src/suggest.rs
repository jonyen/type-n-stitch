//! Suggested edits: filler words and long pauses, computed from the word
//! timestamps alone. The client mirrors these rules in `suggest.ts` so it can
//! show counts instantly; the server exposes them at `POST /media/:id/suggest`.
//!
//! Every suggestion is a plain `Edit::Cut`, so applying a batch is just
//! appending them to the edit list (one undo step) and the usual cut
//! normalization merges neighbours.

use crate::types::{Edit, Word};

/// Tolerance for comparing gaps against thresholds (timestamps are ms).
const EPS: f64 = 1e-6;

/// Single words that are cut when "Remove fillers" runs. Matching is
/// case-insensitive and ignores surrounding punctuation.
pub const FILLERS: &[&str] = &["um", "uh", "umm", "uhh", "hmm", "mm", "er", "ah", "erm"];

/// Two-word fillers, only removed when `SuggestOptions::two_word_fillers` is on.
pub const TWO_WORD_FILLERS: &[(&str, &str)] = &[("you", "know"), ("i", "mean")];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuggestOptions {
    /// Also cut "you know" and "I mean". Off by default: they are often real
    /// speech.
    pub two_word_fillers: bool,
    /// A gap between consecutive words longer than this is a pause worth
    /// tightening.
    pub pause_threshold: f64,
    /// How much of a tightened pause is left in the output.
    pub pause_keep: f64,
    /// Silence before the first word longer than this is trimmed.
    pub leading_threshold: f64,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            two_word_fillers: false,
            pause_threshold: 0.6,
            pause_keep: 0.25,
            leading_threshold: 0.5,
        }
    }
}

/// Lower-case a word and strip leading/trailing punctuation so `"Um,"`
/// and `"um"` compare equal.
pub fn normalize_word(text: &str) -> String {
    text.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// End of the span a word "owns": the next word's start, or the end of the
/// media for the last word, so the pause after it goes with the cut.
fn owned_end(words: &[Word], index: usize, duration: f64) -> f64 {
    words.get(index + 1).map_or(duration, |w| w.start)
}

/// One cut per filler occurrence, in word order. A two-word filler yields a
/// single cut covering both words.
pub fn filler_cuts(words: &[Word], duration: f64, opts: &SuggestOptions) -> Vec<Edit> {
    let norm: Vec<String> = words.iter().map(|w| normalize_word(&w.text)).collect();
    let mut cuts = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let two = opts.two_word_fillers
            && i + 1 < words.len()
            && TWO_WORD_FILLERS
                .iter()
                .any(|(a, b)| norm[i] == *a && norm[i + 1] == *b);
        if two {
            cuts.push(Edit::Cut {
                start: words[i].start,
                end: owned_end(words, i + 1, duration),
            });
            i += 2;
            continue;
        }
        if FILLERS.contains(&norm[i].as_str()) {
            cuts.push(Edit::Cut {
                start: words[i].start,
                end: owned_end(words, i, duration),
            });
        }
        i += 1;
    }
    cuts
}

/// One cut per pause longer than `pause_threshold`, leaving `pause_keep`
/// after the preceding word. Leading silence longer than `leading_threshold`
/// is trimmed the same way (keeping `pause_keep` before the first word).
pub fn pause_cuts(words: &[Word], opts: &SuggestOptions) -> Vec<Edit> {
    let mut cuts = Vec::new();
    if let Some(first) = words.first() {
        if first.start > opts.leading_threshold + EPS {
            cuts.push(Edit::Cut {
                start: 0.0,
                end: first.start - opts.pause_keep,
            });
        }
    }
    for pair in words.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        if next.start - prev.end > opts.pause_threshold + EPS {
            cuts.push(Edit::Cut {
                start: prev.end + opts.pause_keep,
                end: next.start,
            });
        }
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(id: usize, text: &str, start: f64, end: f64) -> Word {
        Word {
            id: format!("w{id}"),
            text: text.into(),
            start,
            end,
        }
    }

    fn cut(start: f64, end: f64) -> Edit {
        Edit::Cut { start, end }
    }

    fn sentence() -> Vec<Word> {
        vec![
            w(0, "So,", 0.0, 0.3),
            w(1, "um,", 0.4, 0.6),
            w(2, "I", 0.7, 0.8),
            w(3, "think", 0.8, 1.1),
            w(4, "UH", 1.2, 1.4),
            w(5, "you", 1.5, 1.6),
            w(6, "know", 1.6, 1.8),
            w(7, "it's", 1.9, 2.0),
            w(8, "fine.", 2.0, 2.4),
            w(9, "Hmm.", 3.0, 3.3),
        ]
    }

    #[test]
    fn normalize_word_strips_case_and_punctuation() {
        assert_eq!(normalize_word("Um,"), "um");
        assert_eq!(normalize_word("\"Uh...\""), "uh");
        assert_eq!(normalize_word("it's"), "it's");
        assert_eq!(normalize_word("..."), "");
    }

    #[test]
    fn filler_cuts_match_case_insensitively_and_own_the_following_gap() {
        let cuts = filler_cuts(&sentence(), 10.0, &SuggestOptions::default());
        assert_eq!(
            cuts,
            vec![cut(0.4, 0.7), cut(1.2, 1.5), cut(3.0, 10.0)],
            "um, UH and the trailing Hmm."
        );
    }

    #[test]
    fn two_word_fillers_are_off_by_default_and_cut_as_one_when_on() {
        let opts = SuggestOptions {
            two_word_fillers: true,
            ..SuggestOptions::default()
        };
        let cuts = filler_cuts(&sentence(), 10.0, &opts);
        assert_eq!(
            cuts,
            vec![cut(0.4, 0.7), cut(1.2, 1.5), cut(1.5, 1.9), cut(3.0, 10.0)]
        );
    }

    #[test]
    fn two_word_filler_needs_both_words_in_order() {
        let words = vec![
            w(0, "I", 0.0, 0.1),
            w(1, "know", 0.1, 0.3),
            w(2, "mean", 0.3, 0.5),
            w(3, "I", 0.5, 0.6),
        ];
        let opts = SuggestOptions {
            two_word_fillers: true,
            ..SuggestOptions::default()
        };
        assert!(filler_cuts(&words, 1.0, &opts).is_empty());
    }

    #[test]
    fn filler_cuts_on_empty_or_clean_speech_are_empty() {
        assert!(filler_cuts(&[], 5.0, &SuggestOptions::default()).is_empty());
        let clean = vec![w(0, "hello", 0.0, 0.5), w(1, "there", 0.5, 1.0)];
        assert!(filler_cuts(&clean, 5.0, &SuggestOptions::default()).is_empty());
    }

    #[test]
    fn pause_cuts_tighten_long_gaps_to_a_quarter_second() {
        let words = vec![
            w(0, "one", 0.2, 0.5),
            w(1, "two", 0.9, 1.2),   // 0.4 s gap: kept
            w(2, "three", 2.0, 2.3), // 0.8 s gap: tightened
            w(3, "four", 2.9, 3.1),  // exactly 0.6: kept
            w(4, "five", 5.0, 5.4),  // 1.9 s gap: tightened
        ];
        let cuts = pause_cuts(&words, &SuggestOptions::default());
        assert_eq!(cuts, vec![cut(1.45, 2.0), cut(3.35, 5.0)]);
    }

    #[test]
    fn pause_cuts_trim_leading_silence_beyond_half_a_second() {
        let late = vec![w(0, "hi", 1.5, 1.8), w(1, "there", 1.9, 2.2)];
        assert_eq!(
            pause_cuts(&late, &SuggestOptions::default()),
            vec![cut(0.0, 1.25)]
        );
        let prompt = vec![w(0, "hi", 0.5, 0.8)];
        assert!(pause_cuts(&prompt, &SuggestOptions::default()).is_empty());
        assert!(pause_cuts(&[], &SuggestOptions::default()).is_empty());
    }
}
