//! Pure helpers behind the MCP tools: the transcript shape an agent reads,
//! the source range a span of words owns, and whole-word search. Nothing
//! here touches the database, the bus or the filesystem, so it is all
//! directly unit-testable.

use engine::editlist::{word_status, WordStatus};
use engine::types::{Edit, Range, Source, Word};
use engine::{locate, stitched_duration};
use serde::Serialize;

/// One transcript word as an agent sees it: its index is the handle every
/// editing tool takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptWord {
    pub i: usize,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub speaker: Option<u32>,
    pub status: WordStatus,
}

/// The transcript after `edits`, with speaker labels where diarization ran.
pub fn transcript(
    words: &[Word],
    speakers: Option<&[Option<u32>]>,
    edits: &[Edit],
) -> Vec<TranscriptWord> {
    words
        .iter()
        .enumerate()
        .map(|(i, word)| TranscriptWord {
            i,
            text: word.text.clone(),
            start: word.start,
            end: word.end,
            speaker: speakers.and_then(|s| s.get(i).copied().flatten()),
            status: word_status(word, edits),
        })
        .collect()
}

/// The source range words `from..=to` own: from the first word's start to the
/// next word's start, or to the end of the media for the last word, and
/// never past the end of the last word's own source. Within one file this is
/// the same rule the client's `rangeForWords` uses, so an agent's cut lands
/// exactly where a person's Delete would.
pub fn word_range(words: &[Word], from: usize, to: usize, sources: &[Source]) -> Option<Range> {
    if from > to || to >= words.len() {
        return None;
    }
    let start = words[from].start;
    let next = words
        .get(to + 1)
        .map_or(stitched_duration(sources), |w| w.start);
    Some(Range::new(
        start,
        next.min(own_source(sources, words[to].start).end),
    ))
}

/// The stretch of the main track held by the source `t` falls in: a word
/// never owns time in another file, so cutting the last word before a join
/// can never take a later video with it, transcribed or not. The whole
/// track when `t` is outside it (or there are no sources).
pub fn own_source(sources: &[Source], t: f64) -> Range {
    match locate(sources, t) {
        Some((k, _)) => Range::new(sources[k].offset, sources[k].offset + sources[k].duration),
        None => Range::new(0.0, stitched_duration(sources)),
    }
}

/// Every whole-word, case- and punctuation-insensitive occurrence of `text`,
/// as inclusive `(from, to)` word index pairs.
pub fn find_ranges(words: &[Word], text: &str) -> Vec<(usize, usize)> {
    let needle: Vec<String> = text
        .split_whitespace()
        .map(normalize)
        .filter(|w| !w.is_empty())
        .collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<String> = words.iter().map(|w| normalize(&w.text)).collect();
    let mut out = Vec::new();
    // Non-overlapping, left to right: a match consumes the words it covers.
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len() - 1));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// A word reduced to what matching cares about: lowercase alphanumerics.
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words at `i.0 .. i.0 + 0.5`, one per second.
    fn w(texts: &[&str]) -> Vec<Word> {
        texts
            .iter()
            .enumerate()
            .map(|(i, text)| Word {
                id: format!("w{i}"),
                text: (*text).to_owned(),
                start: i as f64,
                end: i as f64 + 0.5,
            })
            .collect()
    }

    #[test]
    fn normalize_and_find_are_case_and_punctuation_insensitive() {
        let words = w(&["Hi,", "and", "welcome", "to", "Type", "and", "Stitch."]);
        assert_eq!(normalize("Stitch."), "stitch");
        assert_eq!(find_ranges(&words, "type and stitch"), vec![(4, 6)]);
        assert_eq!(find_ranges(&words, "AND"), vec![(1, 1), (5, 5)]);
        assert!(find_ranges(&words, "elcome").is_empty(), "whole words only");
        assert!(find_ranges(&words, "").is_empty());
    }

    fn src(offset: f64, duration: f64) -> engine::Source {
        engine::Source {
            media: format!("m{offset}"),
            offset,
            duration,
        }
    }

    #[test]
    fn word_range_owns_the_gap_after_the_last_word() {
        let words = w(&["a", "b", "c"]); // starts 0,1,2; ends 0.5,1.5,2.5
        let one = [src(0.0, 10.0)];
        assert_eq!(word_range(&words, 1, 1, &one), Some(Range::new(1.0, 2.0)));
        assert_eq!(word_range(&words, 2, 2, &one), Some(Range::new(2.0, 10.0)));
        assert_eq!(word_range(&words, 2, 1, &one), None);
        assert_eq!(word_range(&words, 0, 3, &one), None);
    }

    #[test]
    fn a_words_owned_stretch_stops_at_the_end_of_its_own_source() {
        // Source 1 (10..14 s) has no words yet: the last word of source 0
        // owns up to 10 s, not the stitched end.
        let words = w(&["a", "b", "c"]);
        let two = [src(0.0, 10.0), src(10.0, 4.0)];
        assert_eq!(word_range(&words, 2, 2, &two), Some(Range::new(2.0, 10.0)));
        // With a word in source 1, the gap before it is still source 0's.
        let mut words = words;
        words.push(Word {
            id: "1:w0".into(),
            text: "d".into(),
            start: 11.0,
            end: 11.5,
        });
        assert_eq!(word_range(&words, 2, 2, &two), Some(Range::new(2.0, 10.0)));
        assert_eq!(word_range(&words, 3, 3, &two), Some(Range::new(11.0, 14.0)));
        assert_eq!(word_range(&words, 1, 3, &two), Some(Range::new(1.0, 14.0)));
    }

    #[test]
    fn transcript_carries_index_speaker_and_status() {
        let words = w(&["a", "b"]);
        let t = transcript(
            &words,
            Some(&[Some(0), Some(1)]),
            &[Edit::Cut {
                start: 1.0,
                end: 2.0,
                transition: None,
            }],
        );
        assert_eq!(t[0].i, 0);
        assert_eq!(t[0].speaker, Some(0));
        assert_eq!(t[0].status, WordStatus::Kept);
        assert_eq!(t[1].status, WordStatus::Cut);
        assert_eq!(transcript(&words, None, &[])[1].speaker, None);
    }
}
