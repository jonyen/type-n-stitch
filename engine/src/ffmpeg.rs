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

use serde::{Deserialize, Serialize};

use crate::editlist::{caption_windows, joins, timeline, Segment, SegmentKind, FADE};
use crate::types::{CaptionPos, Edit, MediaKind, TitleStyle, Transition};

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

/// Frame size and rate of the source picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// Used when the source's frame size and rate are unknown.
pub const DEFAULT_VIDEO: VideoInfo = VideoInfo {
    width: 1280,
    height: 720,
    fps: 30.0,
};

#[derive(Debug)]
pub struct ExportOptions<'a> {
    pub duration: f64,
    pub kind: MediaKind,
    pub format: OutputFormat,
    pub output: &'a Path,
    /// Local audio file for each overdub edit's `audio_url`.
    pub overdub_audio: &'a HashMap<String, PathBuf>,
    /// Frame size and rate of the source picture; titles must match it.
    pub video: Option<VideoInfo>,
    /// Font file for `drawtext`.
    pub font: &'a Path,
    /// Transition used at every join that does not override it.
    pub transition: Transition,
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

    let video_info = opts.video.unwrap_or(DEFAULT_VIDEO);
    let windows = caption_windows(&segments, edits);
    let joins = joins(&segments, edits, opts.transition);
    // A dip needs room for both halves, so very short pieces stay hard-cut.
    let dips = |i: usize| segments[i].output.len() >= 2.0 * FADE;
    let dip_out = |i: usize| {
        dips(i)
            && joins
                .iter()
                .any(|j| j.after == i && j.transition == Transition::Dip)
    };
    let dip_in = |i: usize| {
        i > 0
            && dips(i)
            && joins
                .iter()
                .any(|j| j.after + 1 == i && j.transition == Transition::Dip)
    };

    let mut graph = String::new();
    let mut concat_inputs = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let hold = seg.output.len();
        let (mut v, mut a) = match seg.kind {
            SegmentKind::Source => (render_video.then(|| video_trim(seg)), audio_trim(seg)),
            SegmentKind::Overdub { index } => (
                render_video.then(|| freeze_frame(seg, opts.duration, hold)),
                overdub_audio(input_index[&index], hold),
            ),
            SegmentKind::Title { index } => (
                render_video.then(|| title_video(&edits[index], video_info, opts.font, hold)),
                silence(hold),
            ),
        };
        if let Some(v) = v.as_mut() {
            // Captions come first so a dip fades the lettering with the picture.
            for w in &windows[i] {
                v.push_str(&caption_filter(
                    &edits[w.index],
                    video_info,
                    opts.font,
                    w.start,
                    w.end,
                ));
            }
            if dip_in(i) {
                let _ = write!(v, ",fade=t=in:st=0:d={}", fmt(FADE));
            }
            if dip_out(i) {
                let _ = write!(v, ",fade=t=out:st={}:d={}", fmt(hold - FADE), fmt(FADE));
            }
        }
        if dip_in(i) {
            let _ = write!(a, ",afade=t=in:st=0:d={}", fmt(FADE));
        }
        if dip_out(i) {
            let _ = write!(a, ",afade=t=out:st={}:d={}", fmt(hold - FADE), fmt(FADE));
        }
        if let Some(v) = v {
            let _ = write!(graph, "{v}[v{i}];");
            let _ = write!(concat_inputs, "[v{i}]");
        }
        let _ = write!(graph, "{a}[a{i}];");
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
        "[0:v]trim=start={}:end={},setpts=PTS-STARTPTS,setsar=1",
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
         tpad=stop_mode=clone:stop_duration={hold},trim=end={hold},setpts=PTS-STARTPTS,setsar=1",
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

/// Make `s` safe inside a single-quoted `drawtext` text with `expansion=none`:
/// apostrophes become typographic, backslashes are doubled.
pub fn drawtext_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\u{2019}")
}

/// Background and foreground for a title card.
fn title_colors(style: TitleStyle) -> (&'static str, &'static str) {
    match style {
        TitleStyle::Dark => ("0x111111", "white"),
        TitleStyle::Light => ("0xf6f6f7", "0x17181a"),
        TitleStyle::Accent => ("0x2563eb", "white"),
    }
}

/// A flat colour card the size of the source picture, with the title drawn on it.
fn title_video(edit: &Edit, video: VideoInfo, font: &Path, hold: f64) -> String {
    let Edit::Title {
        text,
        subtitle,
        style,
        ..
    } = edit
    else {
        unreachable!("title segment points at a non-title edit")
    };
    let (bg, fg) = title_colors(*style);
    let font = font.to_string_lossy();
    let big = video.height / 12;
    let small = video.height / 24;
    let mut s = format!(
        "color=c={bg}:s={w}x{h}:r={fps}:d={d},format=yuv420p,setsar=1",
        w = video.width,
        h = video.height,
        fps = fmt(video.fps),
        d = fmt(hold)
    );
    match subtitle {
        Some(sub) if !sub.trim().is_empty() => {
            let _ = write!(
                s,
                ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={big}:\
                 fontcolor={fg}:x=(w-text_w)/2:y=h*0.42-text_h/2",
                drawtext_escape(text)
            );
            let _ = write!(
                s,
                ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={small}:\
                 fontcolor={fg}:x=(w-text_w)/2:y=h*0.58-text_h/2",
                drawtext_escape(sub)
            );
        }
        _ => {
            let _ = write!(
                s,
                ",drawtext=fontfile={font}:expansion=none:text='{}':fontsize={big}:\
                 fontcolor={fg}:x=(w-text_w)/2:y=(h-text_h)/2",
                drawtext_escape(text)
            );
        }
    }
    s
}

/// Silence under a title card, in the same format as every other piece.
fn silence(hold: f64) -> String {
    format!(
        "anullsrc=r=48000:cl=stereo,atrim=end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}",
        fmt(hold)
    )
}

/// A caption drawn over one piece, visible for `[start, end)` of that piece.
fn caption_filter(edit: &Edit, video: VideoInfo, font: &Path, start: f64, end: f64) -> String {
    let Edit::Caption { text, position, .. } = edit else {
        unreachable!("caption window points at a non-caption edit")
    };
    let (x, y) = match position {
        CaptionPos::BottomLeft => ("w*0.05", "h*0.85-text_h"),
        CaptionPos::BottomCenter => ("(w-text_w)/2", "h*0.85-text_h"),
        CaptionPos::TopLeft => ("w*0.05", "h*0.08"),
    };
    format!(
        ",drawtext=fontfile={}:expansion=none:text='{}':fontsize={}:fontcolor=white:box=1:\
         boxcolor=black@0.55:boxborderw=12:x={x}:y={y}:enable='between(t,{},{})'",
        font.to_string_lossy(),
        drawtext_escape(text),
        video.height / 28,
        fmt(start),
        fmt(end)
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

/// Millisecond precision, without trailing zeros: ffmpeg reads `2.75` and
/// `0` as happily as `2.750`, and the graphs stay readable.
fn fmt(t: f64) -> String {
    if t.abs() < 5e-4 {
        return "0".into();
    }
    let s = format!("{t:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
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
            video: Some(VideoInfo {
                width: 1280,
                height: 720,
                fps: 30.0,
            }),
            font: Path::new("/fonts/F.ttf"),
            transition: Transition::None,
        }
    }

    fn filter_complex(args: &[String]) -> &str {
        let i = args.iter().position(|a| a == "-filter_complex").unwrap();
        &args[i + 1]
    }

    fn cut(start: f64, end: f64) -> Edit {
        Edit::Cut {
            start,
            end,
            transition: None,
        }
    }

    /// Plan `edits` with the default options and return the filter graph.
    fn graph(edits: &[Edit], kind: MediaKind, format: OutputFormat) -> String {
        graph_with(edits, kind, format, |_| {})
    }

    /// Same, with a chance to tweak the options first.
    fn graph_with(
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> String {
        let none = HashMap::new();
        let out = PathBuf::from(format!("out.{}", format.extension()));
        let mut options = opts(kind, format, &out, &none);
        tweak(&mut options);
        let input = if kind == MediaKind::Video {
            "in.mp4"
        } else {
            "in.mp3"
        };
        let args = build_ffmpeg_args(Path::new(input), edits, &options).unwrap();
        filter_complex(&args).to_owned()
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
        let g = filter_complex(&args);
        assert!(g.contains("[0:v]trim=start=0:end=2,setpts=PTS-STARTPTS,setsar=1[v0]"));
        assert!(g.contains("[0:a]atrim=start=4:end=10,asetpts=PTS-STARTPTS"));
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
        let g = filter_complex(&args);
        assert!(g.contains(
            "[0:v]trim=start=3:end=4,setpts=PTS-STARTPTS,select=eq(n\\,0),\
             tpad=stop_mode=clone:stop_duration=2.5,trim=end=2.5,setpts=PTS-STARTPTS,setsar=1[v1]"
        ));
        assert!(g.contains("[1:a]aresample=48000"));
        assert!(g.contains("apad=whole_dur=2.5,atrim=end=2.5,asetpts=PTS-STARTPTS[a1]"));
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
        let g = filter_complex(&args);
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
        assert!(!filter_complex(&args).contains("[0:v]"));
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

    #[test]
    fn drawtext_escape_neutralises_quotes_and_backslashes() {
        assert_eq!(
            drawtext_escape("It's 50% \\ done: yes"),
            "It’s 50% \\\\ done: yes"
        );
    }

    #[test]
    fn title_piece_is_a_color_source_with_text_and_silence() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 3.0,
            text: "Hello".into(),
            subtitle: Some("sub".into()),
            style: TitleStyle::Accent,
        }];
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(g.contains("color=c=0x2563eb:s=1280x720:r=30:d=3"), "{g}");
        assert!(
            g.contains("drawtext=fontfile=/fonts/F.ttf:expansion=none:text='Hello'"),
            "{g}"
        );
        assert!(g.contains("text='sub'"), "{g}");
        assert!(g.contains("anullsrc=r=48000:cl=stereo,atrim=end=3"), "{g}");
        assert!(g.contains("concat=n=3:v=1:a=1"), "{g}");
        assert!(g.contains("setsar=1"), "{g}");
    }

    #[test]
    fn title_without_video_info_uses_the_default_frame() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 1.0,
            text: "T".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.video = None
        });
        assert!(g.contains("s=1280x720:r=30"), "{g}");
        assert!(g.contains("c=0x111111"), "{g}");
    }

    #[test]
    fn audio_only_export_keeps_title_silence_and_skips_captions() {
        let edits = [
            Edit::Title {
                at: 2.0,
                duration: 1.0,
                text: "T".into(),
                subtitle: None,
                style: TitleStyle::Dark,
            },
            Edit::Caption {
                start: 3.0,
                end: 4.0,
                text: "c".into(),
                position: CaptionPos::BottomLeft,
            },
        ];
        let g = graph(&edits, MediaKind::Audio, OutputFormat::Mp3);
        assert!(g.contains("anullsrc"), "{g}");
        assert!(!g.contains("drawtext"), "{g}");
        assert!(g.contains("concat=n=3:v=0:a=1"), "{g}");
    }

    #[test]
    fn caption_is_drawn_on_its_segment_with_an_enable_window() {
        let edits = [
            cut(3.0, 5.0),
            Edit::Caption {
                start: 2.0,
                end: 7.0,
                text: "Ada".into(),
                position: CaptionPos::BottomCenter,
            },
        ];
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(
            g.contains(
                "text='Ada':fontsize=25:fontcolor=white:box=1:boxcolor=black@0.55:\
                 boxborderw=12:x=(w-text_w)/2:y=h*0.85-text_h:enable='between(t,2,3)'"
            ),
            "{g}"
        );
        assert!(g.contains("enable='between(t,0,2)'"), "{g}");
    }

    #[test]
    fn dip_adds_fade_pairs_only_at_dipping_joins() {
        let edits = [cut(3.0, 5.0)];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.transition = Transition::Dip
        });
        assert!(g.contains("fade=t=out:st=2.75:d=0.25"), "{g}");
        assert!(g.contains("afade=t=out:st=2.75:d=0.25"), "{g}");
        assert!(g.contains("fade=t=in:st=0:d=0.25"), "{g}");
        assert!(g.contains("afade=t=in:st=0:d=0.25"), "{g}");
        let g = graph(&edits, MediaKind::Video, OutputFormat::Mp4);
        assert!(!g.contains("fade="), "{g}");
    }

    #[test]
    fn a_piece_shorter_than_half_a_second_is_not_faded() {
        let edits = [cut(0.3, 5.0)];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.transition = Transition::Dip
        });
        assert!(!g.contains("fade=t=out"), "{g}");
        assert!(g.contains("fade=t=in"), "{g}");
    }
}
