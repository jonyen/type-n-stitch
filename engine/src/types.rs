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

/// An edit applied to the source media. The edit list is the whole project:
/// the source file plus a `Vec<Edit>` fully describes the output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Edit {
    /// Remove `[start, end)` from the output entirely. `transition` overrides
    /// the project setting where this cut joins its neighbours.
    Cut {
        start: f64,
        end: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition: Option<Transition>,
    },
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

    pub fn is_cut(&self) -> bool {
        matches!(self, Edit::Cut { .. })
    }
}

/// Whether the source has a picture track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}
