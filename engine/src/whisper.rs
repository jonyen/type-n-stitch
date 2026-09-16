//! Parsing whisper.cpp's `--output-json` format into words.
//!
//! We run `whisper-cli -ml 1 -sow` so every segment is one word. Times come
//! from `offsets` (milliseconds), which is exact where the `timestamps`
//! strings are rounded.

use serde::Deserialize;

use crate::types::Word;

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperSegment>,
}

#[derive(Deserialize)]
struct WhisperSegment {
    offsets: Offsets,
    text: String,
}

#[derive(Deserialize)]
struct Offsets {
    from: u64,
    to: u64,
}

/// Turn whisper.cpp JSON into a list of words. Whitespace-only segments and
/// zero-length segments are dropped; ids are stable positional (`w0`, `w1`…).
pub fn parse_whisper_json(json: &str) -> Result<Vec<Word>, serde_json::Error> {
    let output: WhisperOutput = serde_json::from_str(json)?;
    let words = output
        .transcription
        .into_iter()
        .filter_map(|seg| {
            let text = seg.text.trim();
            if text.is_empty() || seg.offsets.to <= seg.offsets.from {
                return None;
            }
            Some((text.to_owned(), seg.offsets.from, seg.offsets.to))
        })
        .enumerate()
        .map(|(i, (text, from, to))| Word {
            id: format!("w{i}"),
            text,
            start: from as f64 / 1000.0,
            end: to as f64 / 1000.0,
        })
        .collect();
    Ok(words)
}

/// Build a `whisper-cli` argv that writes `<out_base>.json` in the format
/// `parse_whisper_json` expects.
/// Whisper is trained on cleaned-up transcripts and silently drops "um" and
/// "uh" unless the decoder is primed with text that contains them. Without
/// this prompt, filler removal finds nothing on real recordings.
pub const DISFLUENCY_PROMPT: &str = "Um, so, uh, I mean, like, you know, hmm.";

pub fn whisper_args(model: &str, wav: &str, out_base: &str) -> Vec<String> {
    [
        "-m",
        model,
        "-f",
        wav,
        "-l",
        "en",
        "-ml",
        "1",
        "-sow",
        "--prompt",
        DISFLUENCY_PROMPT,
        "-oj",
        "-of",
        out_base,
        "-np",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "systeminfo": "x",
      "model": {"type": "large-v3-turbo"},
      "transcription": [
        {"timestamps": {"from": "00:00:00,000", "to": "00:00:00,000"}, "offsets": {"from": 0, "to": 0}, "text": ""},
        {"timestamps": {"from": "00:00:00,000", "to": "00:00:00,910"}, "offsets": {"from": 0, "to": 910}, "text": " thankful"},
        {"timestamps": {"from": "00:00:00,910", "to": "00:00:01,250"}, "offsets": {"from": 910, "to": 1250}, "text": " for"},
        {"timestamps": {"from": "00:00:01,250", "to": "00:00:01,250"}, "offsets": {"from": 1250, "to": 1250}, "text": "   "},
        {"timestamps": {"from": "00:01:02,500", "to": "00:01:03,000"}, "offsets": {"from": 62500, "to": 63000}, "text": " gospel."}
      ]
    }"#;

    #[test]
    fn primes_the_decoder_so_fillers_survive() {
        let args = whisper_args("m.bin", "in.wav", "out");
        let at = args.iter().position(|a| a == "--prompt").expect("--prompt");
        let prompt = &args[at + 1];
        assert_eq!(prompt, DISFLUENCY_PROMPT);
        for filler in ["Um", "uh", "hmm"] {
            assert!(prompt.contains(filler), "prompt should contain {filler}");
        }
        assert!(args.windows(2).any(|w| w == ["-ml", "1"]));
    }

    #[test]
    fn parses_words_and_drops_blanks() {
        let words = parse_whisper_json(SAMPLE).unwrap();
        assert_eq!(words.len(), 3);
        assert_eq!(
            words[0],
            Word {
                id: "w0".into(),
                text: "thankful".into(),
                start: 0.0,
                end: 0.91
            }
        );
        assert_eq!(words[1].text, "for");
        assert_eq!(words[2].id, "w2");
        assert_eq!(words[2].start, 62.5);
        assert_eq!(words[2].end, 63.0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_whisper_json("{\"nope\": 1}").is_err());
    }

    #[test]
    fn whisper_args_request_one_word_per_segment_json() {
        let args = whisper_args("m.bin", "in.wav", "out");
        assert!(args.windows(2).any(|w| w == ["-ml", "1"]));
        assert!(args.contains(&"-sow".to_owned()));
        assert!(args.contains(&"-oj".to_owned()));
        assert!(args.windows(2).any(|w| w == ["-of", "out"]));
    }
}
