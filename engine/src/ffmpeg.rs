//! Planning the ffmpeg render for an edit list.
//!
//! Every output piece becomes one `trim`/`atrim` chain on the source (or, for
//! an overdub, a frozen first frame plus the synthesized WAV), and the pieces
//! are joined with the `concat` filter. Audio is resampled to a common
//! format first because `concat` refuses mismatched streams.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::editlist::{timeline, Segment, SegmentKind};
use crate::types::{Edit, MediaKind};

/// Container/codec for the rendered file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Mp4,
    Mp3,
    Wav,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Mp4 => "mp4",
            OutputFormat::Mp3 => "mp3",
            OutputFormat::Wav => "wav",
        }
    }

    /// The natural format for a source kind: video stays mp4, audio becomes mp3.
    pub fn for_kind(kind: MediaKind) -> Self {
        match kind {
            MediaKind::Video => OutputFormat::Mp4,
            MediaKind::Audio => OutputFormat::Mp3,
        }
    }
}

#[derive(Debug)]
pub struct ExportOptions<'a> {
    pub duration: f64,
    pub kind: MediaKind,
    pub format: OutputFormat,
    pub output: &'a Path,
    /// Local audio file for each overdub edit's `audio_url`.
    pub overdub_audio: &'a HashMap<String, PathBuf>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ExportError {
    #[error("every word was cut; nothing left to export")]
    NothingToExport,
    #[error("no audio file for overdub {0}")]
    MissingOverdubAudio(String),
    #[error("a video source cannot be exported as {0}")]
    FormatMismatch(&'static str),
}

/// Audio format every segment is coerced to before `concat`.
const AUDIO_NORMALIZE: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";

/// Build the full ffmpeg argv (without the program name) that renders `edits`
/// applied to `input` into `opts.output`.
pub fn build_ffmpeg_args(
    input: &Path,
    edits: &[Edit],
    opts: &ExportOptions,
) -> Result<Vec<String>, ExportError> {
    let segments = timeline(opts.duration, edits);
    if segments.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let with_video = opts.kind == MediaKind::Video;
    if !with_video && opts.format == OutputFormat::Mp4 {
        return Err(ExportError::FormatMismatch("mp4"));
    }
    let render_video = with_video && opts.format == OutputFormat::Mp4;

    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error", "-i"]
        .map(String::from)
        .to_vec();
    args.push(input.to_string_lossy().into_owned());

    // Each overdub WAV is an extra input; remember its index.
    let mut input_index: HashMap<usize, usize> = HashMap::new();
    for seg in &segments {
        if let SegmentKind::Overdub { index } = seg.kind {
            let Edit::Overdub { audio_url, .. } = &edits[index] else {
                continue;
            };
            let path = opts
                .overdub_audio
                .get(audio_url)
                .ok_or_else(|| ExportError::MissingOverdubAudio(audio_url.clone()))?;
            input_index.insert(index, input_index.len() + 1);
            args.push("-i".into());
            args.push(path.to_string_lossy().into_owned());
        }
    }

    let mut graph = String::new();
    let mut concat_inputs = String::new();
    for (i, seg) in segments.iter().enumerate() {
        match seg.kind {
            SegmentKind::Source => {
                if render_video {
                    let _ = write!(graph, "{}[v{i}];", video_trim(seg));
                }
                let _ = write!(graph, "{}[a{i}];", audio_trim(seg));
            }
            SegmentKind::Overdub { index } => {
                let hold = seg.output.len();
                if render_video {
                    let _ = write!(graph, "{}[v{i}];", freeze_frame(seg, opts.duration, hold));
                }
                let _ = write!(graph, "{}[a{i}];", overdub_audio(input_index[&index], hold));
            }
            // Task 3 renders title cards; the planner does not emit them yet.
            SegmentKind::Title { .. } => unreachable!("title rendering lands in task 3"),
        }
        if render_video {
            let _ = write!(concat_inputs, "[v{i}]");
        }
        let _ = write!(concat_inputs, "[a{i}]");
    }
    let _ = write!(
        graph,
        "{concat_inputs}concat=n={}:v={}:a=1{}[outa]",
        segments.len(),
        u8::from(render_video),
        if render_video { "[outv]" } else { "" },
    );

    args.push("-filter_complex".into());
    args.push(graph);
    if render_video {
        args.extend(["-map", "[outv]"].map(String::from));
    }
    args.extend(["-map", "[outa]"].map(String::from));
    args.extend(codec_args(opts.format).into_iter().map(String::from));
    args.push(opts.output.to_string_lossy().into_owned());
    Ok(args)
}

fn video_trim(seg: &Segment) -> String {
    format!(
        "[0:v]trim=start={}:end={},setpts=PTS-STARTPTS",
        fmt(seg.source.start),
        fmt(seg.source.end)
    )
}

fn audio_trim(seg: &Segment) -> String {
    format!(
        "[0:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}",
        fmt(seg.source.start),
        fmt(seg.source.end)
    )
}

/// Take the first frame of the range and clone it for `hold` seconds.
fn freeze_frame(seg: &Segment, duration: f64, hold: f64) -> String {
    // A one-second window guarantees at least one frame at any sane frame rate.
    let window_end = (seg.source.start + 1.0).min(duration);
    format!(
        "[0:v]trim=start={}:end={},setpts=PTS-STARTPTS,select=eq(n\\,0),\
         tpad=stop_mode=clone:stop_duration={hold},trim=end={hold},setpts=PTS-STARTPTS",
        fmt(seg.source.start),
        fmt(window_end),
        hold = fmt(hold),
    )
}

/// The synthesized WAV, padded or trimmed to exactly `hold` seconds.
fn overdub_audio(input: usize, hold: f64) -> String {
    format!(
        "[{input}:a]{AUDIO_NORMALIZE},apad=whole_dur={hold},atrim=end={hold},asetpts=PTS-STARTPTS",
        hold = fmt(hold)
    )
}

fn codec_args(format: OutputFormat) -> Vec<&'static str> {
    match format {
        OutputFormat::Mp4 => vec![
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
        OutputFormat::Mp3 => vec!["-c:a", "libmp3lame", "-q:a", "2"],
        OutputFormat::Wav => vec!["-c:a", "pcm_s16le"],
    }
}

fn fmt(t: f64) -> String {
    format!("{t:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(
        kind: MediaKind,
        format: OutputFormat,
        output: &'a Path,
        overdub_audio: &'a HashMap<String, PathBuf>,
    ) -> ExportOptions<'a> {
        ExportOptions {
            duration: 10.0,
            kind,
            format,
            output,
            overdub_audio,
        }
    }

    fn graph(args: &[String]) -> &str {
        let i = args.iter().position(|a| a == "-filter_complex").unwrap();
        &args[i + 1]
    }

    #[test]
    fn video_with_one_cut_trims_and_concats_two_pieces() {
        let none = HashMap::new();
        let out = Path::new("out.mp4");
        let edits = [Edit::Cut {
            start: 2.0,
            end: 4.0,
            transition: None,
        }];
        let args = build_ffmpeg_args(
            Path::new("in.mp4"),
            &edits,
            &opts(MediaKind::Video, OutputFormat::Mp4, out, &none),
        )
        .unwrap();

        assert_eq!(
            &args[..6],
            &["-y", "-hide_banner", "-loglevel", "error", "-i", "in.mp4"]
        );
        let g = graph(&args);
        assert!(g.contains("[0:v]trim=start=0.000:end=2.000,setpts=PTS-STARTPTS[v0]"));
        assert!(g.contains("[0:a]atrim=start=4.000:end=10.000,asetpts=PTS-STARTPTS"));
        assert!(g.ends_with("[v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"));
        assert!(args.windows(2).any(|w| w == ["-map", "[outv]"]));
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn overdub_adds_input_freeze_frame_and_padded_audio() {
        let mut audio = HashMap::new();
        audio.insert(
            "/data/m1/od-0.wav".to_owned(),
            PathBuf::from("/srv/od-0.wav"),
        );
        let out = Path::new("out.mp4");
        let edits = [Edit::Overdub {
            start: 3.0,
            end: 4.0,
            text: "hi".into(),
            audio_url: "/data/m1/od-0.wav".into(),
            audio_duration: 2.5,
        }];
        let args = build_ffmpeg_args(
            Path::new("in.mp4"),
            &edits,
            &opts(MediaKind::Video, OutputFormat::Mp4, out, &audio),
        )
        .unwrap();

        assert!(args.windows(2).any(|w| w == ["-i", "/srv/od-0.wav"]));
        let g = graph(&args);
        assert!(g.contains(
            "[0:v]trim=start=3.000:end=4.000,setpts=PTS-STARTPTS,select=eq(n\\,0),\
             tpad=stop_mode=clone:stop_duration=2.500,trim=end=2.500,setpts=PTS-STARTPTS[v1]"
        ));
        assert!(g.contains("[1:a]aresample=48000"));
        assert!(g.contains("apad=whole_dur=2.500,atrim=end=2.500,asetpts=PTS-STARTPTS[a1]"));
        assert!(g.contains("concat=n=3:v=1:a=1[outv][outa]"));
    }

    #[test]
    fn audio_only_has_no_video_chains() {
        let none = HashMap::new();
        let out = Path::new("out.mp3");
        let edits = [Edit::Cut {
            start: 0.0,
            end: 1.0,
            transition: None,
        }];
        let args = build_ffmpeg_args(
            Path::new("in.mp3"),
            &edits,
            &opts(MediaKind::Audio, OutputFormat::Mp3, out, &none),
        )
        .unwrap();
        let g = graph(&args);
        assert!(!g.contains("[0:v]"));
        assert!(g.ends_with("[a0]concat=n=1:v=0:a=1[outa]"));
        assert!(!args.iter().any(|a| a == "[outv]"));
        assert!(args.windows(2).any(|w| w == ["-c:a", "libmp3lame"]));
    }

    #[test]
    fn video_source_can_render_audio_only_wav() {
        let none = HashMap::new();
        let out = Path::new("out.wav");
        let args = build_ffmpeg_args(
            Path::new("in.mp4"),
            &[],
            &opts(MediaKind::Video, OutputFormat::Wav, out, &none),
        )
        .unwrap();
        assert!(!graph(&args).contains("[0:v]"));
        assert!(args.windows(2).any(|w| w == ["-c:a", "pcm_s16le"]));
    }

    #[test]
    fn errors_when_everything_is_cut() {
        let none = HashMap::new();
        let out = Path::new("out.mp4");
        let edits = [Edit::Cut {
            start: 0.0,
            end: 10.0,
            transition: None,
        }];
        let err = build_ffmpeg_args(
            Path::new("in.mp4"),
            &edits,
            &opts(MediaKind::Video, OutputFormat::Mp4, out, &none),
        )
        .unwrap_err();
        assert_eq!(err, ExportError::NothingToExport);
    }

    #[test]
    fn errors_when_overdub_audio_is_unknown() {
        let none = HashMap::new();
        let out = Path::new("out.mp4");
        let edits = [Edit::Overdub {
            start: 1.0,
            end: 2.0,
            text: "x".into(),
            audio_url: "/data/nope.wav".into(),
            audio_duration: 1.0,
        }];
        let err = build_ffmpeg_args(
            Path::new("in.mp4"),
            &edits,
            &opts(MediaKind::Video, OutputFormat::Mp4, out, &none),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ExportError::MissingOverdubAudio("/data/nope.wav".into())
        );
    }

    #[test]
    fn audio_source_cannot_become_mp4() {
        let none = HashMap::new();
        let out = Path::new("out.mp4");
        let err = build_ffmpeg_args(
            Path::new("in.mp3"),
            &[],
            &opts(MediaKind::Audio, OutputFormat::Mp4, out, &none),
        )
        .unwrap_err();
        assert_eq!(err, ExportError::FormatMismatch("mp4"));
    }
}
