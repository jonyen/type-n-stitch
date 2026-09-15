//! Core data model shared between the server API and the client.
//!
//! All times are seconds from the start of the source media. Ranges are
//! half-open: `[start, end)`.

use serde::{Deserialize, Serialize};

/// One transcribed word with its time span in the source media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// A half-open time range `[start, end)` in source seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl Range {
    pub const fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn contains(&self, t: f64) -> bool {
        t >= self.start && t < self.end
    }
}

/// An edit applied to the source media. The edit list is the whole project:
/// the source file plus a `Vec<Edit>` fully describes the output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Edit {
    /// Remove `[start, end)` from the output entirely.
    Cut { start: f64, end: f64 },
    /// Replace the audio of `[start, end)` with synthesized speech. The
    /// output holds the first frame of the range for `audio_duration`.
    #[serde(rename_all = "camelCase")]
    Overdub {
        start: f64,
        end: f64,
        text: String,
        audio_url: String,
        audio_duration: f64,
    },
}

impl Edit {
    pub fn range(&self) -> Range {
        match *self {
            Edit::Cut { start, end } | Edit::Overdub { start, end, .. } => Range::new(start, end),
        }
    }
}

/// Whether the source has a picture track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}
