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

/// A video on the main track, placed at a fixed stitched offset. Source 0 is
/// the project's own media at offset 0; every later one comes from an
/// `Op::AddSource` and starts where the one before it ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub media: String,
    pub offset: f64,
    pub duration: f64,
}

/// Where a layer's picture sits on the canvas: full frame, or a corner
/// picture-in-picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Frame {
    Full,
    PipTopLeft,
    PipTopRight,
    PipBottomLeft,
    PipBottomRight,
}

/// Upper bounds on inserted text so a stored op cannot bloat every fold.
pub const MAX_TITLES: usize = 32;
pub const MAX_CAPTIONS: usize = 32;

/// Upper bounds on splits, track-2 layers (B-roll) and audio inserts, for the same reason.
pub const MAX_SPLITS: usize = 64;
pub const MAX_BROLL: usize = 32;
pub const MAX_AUDIO: usize = 16;

/// Gain range accepted for an `Audio` edit.
pub const MIN_GAIN_DB: f64 = -30.0;
pub const MAX_GAIN_DB: f64 = 12.0;

/// Default for `Edit::Audio::duck` and `Op::AddAudio::duck` on old logs.
pub fn default_duck() -> bool {
    true
}

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
    /// Show `media` from `offset` on video track `track` (2 or 3) over
    /// main-track `[start, end)`, placed by `frame`. Upper tracks cover lower
    /// ones. `audio` is `None` when muted, else the layer's level in dB; the
    /// main audio always continues.
    Layer {
        track: u8,
        start: f64,
        end: f64,
        media: String,
        #[serde(default)]
        offset: f64,
        frame: Frame,
        audio: Option<f64>,
    },
    /// Mix asset `media` from `offset` over `[start, end)` at `gain` dB, ducked under speech when `duck`.
    Audio {
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
}

impl Edit {
    pub fn range(&self) -> Range {
        match *self {
            Edit::Cut { start, end, .. }
            | Edit::Overdub { start, end, .. }
            | Edit::Caption { start, end, .. }
            | Edit::Layer { start, end, .. }
            | Edit::Audio { start, end, .. } => Range::new(start, end),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn layer() -> Edit {
        Edit::Layer {
            track: 3,
            start: 1.0,
            end: 2.5,
            media: "asset-1".into(),
            offset: 0.5,
            frame: Frame::PipTopRight,
            audio: Some(-6.0),
        }
    }

    #[test]
    fn layer_edit_serialises_with_the_layer_tag() {
        let json = serde_json::to_value(layer()).unwrap();
        assert_eq!(
            json,
            json!({
                "kind": "layer", "track": 3, "start": 1.0, "end": 2.5,
                "media": "asset-1", "offset": 0.5, "frame": "pipTopRight", "audio": -6.0
            })
        );
        let back: Edit = serde_json::from_value(json).unwrap();
        assert_eq!(back, layer());
    }

    #[test]
    fn a_muted_layer_writes_audio_null() {
        let muted = Edit::Layer {
            track: 2,
            start: 0.0,
            end: 1.0,
            media: "asset-1".into(),
            offset: 0.0,
            frame: Frame::Full,
            audio: None,
        };
        assert_eq!(
            serde_json::to_value(&muted).unwrap(),
            json!({
                "kind": "layer", "track": 2, "start": 0.0, "end": 1.0,
                "media": "asset-1", "offset": 0.0, "frame": "full", "audio": null
            })
        );
    }

    #[test]
    fn layer_offset_and_audio_default_when_missing() {
        let e: Edit = serde_json::from_str(
            r#"{"kind":"layer","track":2,"start":0,"end":1,"media":"m","frame":"full"}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 1.0,
                media: "m".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            }
        );
    }

    #[test]
    fn frame_names_are_camel_case() {
        for (frame, name) in [
            (Frame::Full, "full"),
            (Frame::PipTopLeft, "pipTopLeft"),
            (Frame::PipTopRight, "pipTopRight"),
            (Frame::PipBottomLeft, "pipBottomLeft"),
            (Frame::PipBottomRight, "pipBottomRight"),
        ] {
            assert_eq!(serde_json::to_value(frame).unwrap(), json!(name));
            let back: Frame = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(back, frame);
        }
    }

    #[test]
    fn source_serialises_flat() {
        let s = Source {
            media: "m2".into(),
            offset: 10.0,
            duration: 5.0,
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(
            json,
            json!({ "media": "m2", "offset": 10.0, "duration": 5.0 })
        );
        assert_eq!(serde_json::from_value::<Source>(json).unwrap(), s);
    }

    #[test]
    fn layer_range_is_its_span_and_broll_edits_are_gone() {
        assert_eq!(layer().range(), Range::new(1.0, 2.5));
        // Folded docs are never persisted, so no stored JSON holds "broll" edits.
        assert!(serde_json::from_str::<Edit>(
            r#"{"kind":"broll","start":0,"end":1,"media":"m","offset":0}"#
        )
        .is_err());
    }
}
