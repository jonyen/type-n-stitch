//! Planning the ffmpeg render for an edit list.
//!
//! Every output piece becomes one `trim`/`atrim` chain on the source (or, for
//! an overdub, a frozen first frame plus the synthesized WAV), and the pieces
//! are joined with the `concat` filter. Audio is resampled to a common
//! format first because `concat` refuses mismatched streams.
//!
//! Text is not drawn by ffmpeg: the installed builds have no `drawtext`, so
//! `crate::text` rasterises title cards and caption boxes into PNGs and this
//! planner brings them in as extra inputs — a looped still for a title piece,
//! an `overlay` for a caption.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

use serde::{Deserialize, Serialize};

use crate::editlist::{
    audio_windows, broll_windows, caption_windows, joins, timeline_with, Segment, SegmentKind,
    Window, FADE,
};
use crate::types::{Edit, MediaKind, Range, Transition, Word};

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

/// The frame size ffmpeg actually decodes, given a display-matrix `rotation`
/// (degrees, as ffprobe reports it — possibly negative).
///
/// ffmpeg autorotates the picture inside the filtergraph, so a quarter-turn
/// source decodes transposed: title cards and caption placements have to be
/// rasterised at the swapped size or `concat` rejects them.
pub fn oriented(video: VideoInfo, rotation: i32) -> VideoInfo {
    if rotation.rem_euclid(180) == 90 {
        VideoInfo {
            width: video.height,
            height: video.width,
            ..video
        }
    } else {
        video
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
    /// Frame size and rate of the source picture; titles must match it.
    pub video: Option<VideoInfo>,
    /// PNG for each `Edit::Title`, by edit index, rendered at the frame size.
    pub title_images: &'a HashMap<usize, PathBuf>,
    /// PNG and its `overlay` placement for each `Edit::Caption`, by edit index.
    pub caption_images: &'a HashMap<usize, (PathBuf, u32, u32)>,
    /// Transition used at every join that does not override it.
    pub transition: Transition,
    pub splits: &'a [f64],
    pub order: &'a [f64],
    /// Local file for each asset id a B-roll or audio edit names.
    pub assets: &'a HashMap<String, PathBuf>,
    /// Transcript words, for ducking. Empty means no ducking.
    pub words: &'a [Word],
}

#[derive(Debug, Error, PartialEq)]
pub enum ExportError {
    #[error("every word was cut; nothing left to export")]
    NothingToExport,
    #[error("no audio file for overdub {0}")]
    MissingOverdubAudio(String),
    #[error("a video source cannot be exported as {0}")]
    FormatMismatch(&'static str),
    #[error("no rendered image for title {0}")]
    MissingTitleImage(usize),
    #[error("no rendered image for caption {0}")]
    MissingCaptionImage(usize),
    #[error("no file for asset {0}")]
    MissingAsset(String),
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
    let segments = timeline_with(opts.duration, edits, opts.splits, opts.order);
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

    let video_info = opts.video.unwrap_or(DEFAULT_VIDEO);
    let windows = caption_windows(&segments, edits);

    // Overdub WAVs and title cards become extra inputs, numbered in the order
    // they are pushed; the source is always input 0.
    let mut next_input = 1;
    let mut overdub_input: HashMap<usize, usize> = HashMap::new();
    let mut title_input: HashMap<usize, usize> = HashMap::new();
    for seg in &segments {
        match seg.kind {
            SegmentKind::Overdub { index } => {
                let Edit::Overdub { audio_url, .. } = &edits[index] else {
                    continue;
                };
                let path = opts
                    .overdub_audio
                    .get(audio_url)
                    .ok_or_else(|| ExportError::MissingOverdubAudio(audio_url.clone()))?;
                overdub_input.insert(index, next_input);
                next_input += 1;
                args.push("-i".into());
                args.push(path.to_string_lossy().into_owned());
            }
            // An audio-only export renders no picture, so it needs no card.
            SegmentKind::Title { index } if render_video => {
                let path = opts
                    .title_images
                    .get(&index)
                    .ok_or(ExportError::MissingTitleImage(index))?;
                title_input.insert(index, next_input);
                next_input += 1;
                args.extend(["-loop", "1", "-framerate"].map(String::from));
                args.push(fmt(video_info.fps));
                args.push("-t".into());
                args.push(fmt(seg.output.len()));
                args.push("-i".into());
                args.push(path.to_string_lossy().into_owned());
            }
            _ => {}
        }
    }
    // One input per caption edit, however many pieces it is drawn on.
    let mut caption_input: HashMap<usize, usize> = HashMap::new();
    if render_video {
        for w in windows.iter().flatten() {
            if caption_input.contains_key(&w.index) {
                continue;
            }
            let (path, ..) = opts
                .caption_images
                .get(&w.index)
                .ok_or(ExportError::MissingCaptionImage(w.index))?;
            caption_input.insert(w.index, next_input);
            next_input += 1;
            args.push("-i".into());
            args.push(path.to_string_lossy().into_owned());
        }
    }
    let brolls = broll_windows(&segments, edits);
    let audios = audio_windows(&segments, edits);
    let asset_path = |id: &str| {
        opts.assets
            .get(id)
            .ok_or_else(|| ExportError::MissingAsset(id.to_owned()))
    };
    let mut broll_input: Vec<Vec<usize>> = vec![Vec::new(); segments.len()];
    if render_video {
        for (i, ws) in brolls.iter().enumerate() {
            for w in ws {
                let Edit::Broll { media, .. } = &edits[w.index] else {
                    continue;
                };
                args.push("-i".into());
                args.push(asset_path(media)?.to_string_lossy().into_owned());
                broll_input[i].push(next_input);
                next_input += 1;
            }
        }
    }
    let mut audio_input: Vec<Vec<usize>> = vec![Vec::new(); segments.len()];
    for (i, ws) in audios.iter().enumerate() {
        for w in ws {
            let Edit::Audio { media, .. } = &edits[w.index] else {
                continue;
            };
            args.push("-i".into());
            args.push(asset_path(media)?.to_string_lossy().into_owned());
            audio_input[i].push(next_input);
            next_input += 1;
        }
    }

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
        let (v, a) = match seg.kind {
            SegmentKind::Source => (render_video.then(|| video_trim(seg)), audio_trim(seg)),
            SegmentKind::Overdub { index } => (
                render_video.then(|| freeze_frame(seg, opts.duration, hold)),
                overdub_audio(overdub_input[&index], hold),
            ),
            SegmentKind::Title { index } => (
                render_video.then(|| title_still(title_input[&index], hold)),
                silence(hold),
            ),
        };
        if let Some(base) = v {
            // `overlay` takes two inputs, so each caption closes the chain so
            // far under a temporary label and starts a new one from it.
            let mut chain = base;
            for (n, w) in brolls[i].iter().enumerate() {
                let Edit::Broll { start, offset, .. } = &edits[w.index] else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let _ = write!(graph,
                    "[{}:v]trim=start={}:end={},setpts=PTS-STARTPTS+{}/TB,scale={W}:{H}:force_original_aspect_ratio=decrease,pad={W}:{H}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}b{n}];",
                    broll_input[i][n], fmt(from), fmt(from + (w.end - w.start)), fmt(w.start),
                    W = video_info.width, H = video_info.height);
                let _ = write!(graph, "{chain}[v{i}bo{n}];");
                chain = format!("[v{i}bo{n}][v{i}b{n}]overlay=x=0:y=0:eof_action=pass:enable='between(t,{},{})'", fmt(w.start), fmt(w.end));
            }
            for (n, w) in windows[i].iter().enumerate() {
                let label = format!("v{i}c{n}");
                let _ = write!(graph, "{chain}[{label}];");
                let (_, x, y) = &opts.caption_images[&w.index];
                chain = format!(
                    "[{label}][{}:v]overlay=x={x}:y={y}:enable='between(t,{},{})'",
                    caption_input[&w.index],
                    fmt(w.start),
                    fmt(w.end)
                );
            }
            // Fades come last so a dip takes the captions with the picture.
            if dip_in(i) {
                let _ = write!(chain, ",fade=t=in:st=0:d={}", fmt(FADE));
            }
            if dip_out(i) {
                let _ = write!(chain, ",fade=t=out:st={}:d={}", fmt(hold - FADE), fmt(FADE));
            }
            let _ = write!(graph, "{chain}[v{i}];");
            let _ = write!(concat_inputs, "[v{i}]");
        }
        let mut mixed = if audios[i].is_empty() {
            a
        } else {
            let _ = write!(graph, "{a}[a{i}m];");
            let mut labels = format!("[a{i}m]");
            for (n, w) in audios[i].iter().enumerate() {
                let Edit::Audio {
                    start,
                    offset,
                    gain,
                    duck,
                    ..
                } = &edits[w.index]
                else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let mut m = format!(
                    "[{}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE},adelay={}:all=1,volume={}dB",
                    audio_input[i][n], fmt(from), fmt(from + (w.end - w.start)),
                    (w.start * 1000.0).round() as i64, fmt(*gain));
                let runs = if *duck {
                    duck_runs(seg, w, opts.words)
                } else {
                    Vec::new()
                };
                if !runs.is_empty() {
                    let _ = write!(m, ",volume=volume='{}':eval=frame", duck_expr(&runs));
                }
                let _ = write!(graph, "{m}[a{i}x{n}];");
                let _ = write!(labels, "[a{i}x{n}]");
            }
            format!(
                "{labels}amix=inputs={}:normalize=0:duration=first",
                audios[i].len() + 1
            )
        };
        if dip_in(i) {
            let _ = write!(mixed, ",afade=t=in:st=0:d={}", fmt(FADE));
        }
        if dip_out(i) {
            let _ = write!(
                mixed,
                ",afade=t=out:st={}:d={}",
                fmt(hold - FADE),
                fmt(FADE)
            );
        }
        let _ = write!(graph, "{mixed}[a{i}];");
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

/// The title card PNG, held for `hold` seconds by its looped input.
fn title_still(input: usize, hold: f64) -> String {
    format!(
        "[{input}:v]format=yuv420p,setsar=1,trim=end={hold},setpts=PTS-STARTPTS",
        hold = fmt(hold)
    )
}

/// Silence under a title card, in the same format as every other piece.
fn silence(hold: f64) -> String {
    format!(
        "anullsrc=r=48000:cl=stereo,atrim=end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}",
        fmt(hold)
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

/// Volume multiplier applied under speech while ducking music.
pub const DUCK_GAIN: f64 = 0.251;
/// Length of the ramp in and out of a duck.
pub const DUCK_RAMP: f64 = 0.12;
/// Word gaps closer than this are merged into one speech run.
pub const SPEECH_GAP: f64 = 0.3;

/// Merge word spans closer than `gap` into speech runs, sorted.
pub fn speech_runs(words: &[Range], gap: f64) -> Vec<Range> {
    let mut sorted: Vec<Range> = words.iter().copied().filter(|r| !r.is_empty()).collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut runs: Vec<Range> = Vec::new();
    for r in sorted {
        match runs.last_mut() {
            Some(last) if r.start <= last.end + gap => last.end = last.end.max(r.end),
            _ => runs.push(r),
        }
    }
    runs
}

/// A `volume` expression in `t`: 1 outside speech, `DUCK_GAIN` inside, linear
/// ramps of `DUCK_RAMP`. "1" when empty.
pub fn duck_expr(runs: &[Range]) -> String {
    if runs.is_empty() {
        return "1".into();
    }
    let term = |r: &Range| {
        format!(
            "max(0,min(1,min((t-{})/{ramp},({}-t)/{ramp})))",
            fmt(r.start - DUCK_RAMP),
            fmt(r.end + DUCK_RAMP),
            ramp = fmt(DUCK_RAMP)
        )
    };
    let mut expr = term(&runs[0]);
    for r in &runs[1..] {
        expr = format!("max({expr},{})", term(r));
    }
    format!("1-{}*({expr})", fmt(1.0 - DUCK_GAIN))
}

/// Speech inside a music window, in segment time: the words the window
/// covers, or the whole hold of an overdub.
fn duck_runs(seg: &Segment, w: &Window, words: &[Word]) -> Vec<Range> {
    match seg.kind {
        SegmentKind::Overdub { .. } => vec![Range::new(w.start, w.end)],
        SegmentKind::Title { .. } => Vec::new(),
        SegmentKind::Source => {
            let spans: Vec<Range> = words
                .iter()
                .filter(|wd| {
                    wd.end > w.source_start && wd.start < w.source_start + (w.end - w.start)
                })
                .map(|wd| {
                    Range::new(
                        (wd.start - seg.source.start).max(w.start),
                        (wd.end - seg.source.start).min(w.end),
                    )
                })
                .collect();
            speech_runs(&spans, SPEECH_GAP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CaptionPos, TitleStyle};

    /// An empty map that lives as long as the test needs it.
    fn leak<T: 'static>(value: T) -> &'static T {
        Box::leak(Box::new(value))
    }

    /// PNG paths keyed by title edit index.
    fn titles(pairs: &[(usize, &str)]) -> &'static HashMap<usize, PathBuf> {
        leak(pairs.iter().map(|(i, p)| (*i, PathBuf::from(p))).collect())
    }

    /// PNG paths and placements keyed by caption edit index.
    fn captions(pairs: &[(usize, &str, u32, u32)]) -> &'static HashMap<usize, (PathBuf, u32, u32)> {
        leak(
            pairs
                .iter()
                .map(|(i, p, x, y)| (*i, (PathBuf::from(p), *x, *y)))
                .collect(),
        )
    }

    /// Local file paths keyed by asset id.
    fn assets(pairs: &[(&str, &str)]) -> &'static HashMap<String, PathBuf> {
        leak(
            pairs
                .iter()
                .map(|(id, p)| ((*id).to_owned(), PathBuf::from(p)))
                .collect(),
        )
    }

    fn word(start: f64, end: f64) -> Word {
        Word {
            id: format!("w{start}"),
            text: "x".into(),
            start,
            end,
        }
    }

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
            title_images: titles(&[]),
            caption_images: captions(&[]),
            transition: Transition::None,
            splits: &[],
            order: &[],
            assets: assets(&[]),
            words: &[],
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
        let args = args_with(edits, kind, format, tweak).unwrap();
        filter_complex(&args).to_owned()
    }

    /// The whole argv, so tests can look at the inputs as well as the graph.
    fn args_with(
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> Result<Vec<String>, ExportError> {
        let none = HashMap::new();
        let out = PathBuf::from(format!("out.{}", format.extension()));
        let mut options = opts(kind, format, &out, &none);
        tweak(&mut options);
        let input = if kind == MediaKind::Video {
            "in.mp4"
        } else {
            "in.mp3"
        };
        build_ffmpeg_args(Path::new(input), edits, &options)
    }

    /// Does `args` contain `needle` as a contiguous run?
    fn has_run(args: &[String], needle: &[&str]) -> bool {
        args.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn speech_runs_merge_close_words() {
        let words = [
            Range::new(1.0, 1.4),
            Range::new(1.5, 2.0),
            Range::new(3.0, 3.2),
        ];
        assert_eq!(
            speech_runs(&words, SPEECH_GAP),
            vec![Range::new(1.0, 2.0), Range::new(3.0, 3.2)]
        );
        assert!(speech_runs(&[], SPEECH_GAP).is_empty());
    }

    #[test]
    fn duck_expr_ramps_around_each_run() {
        assert_eq!(duck_expr(&[]), "1");
        assert_eq!(
            duck_expr(&[Range::new(1.0, 2.0)]),
            "1-0.749*(max(0,min(1,min((t-0.88)/0.12,(2.12-t)/0.12))))"
        );
        assert_eq!(
            duck_expr(&[Range::new(1.0, 2.0), Range::new(3.0, 3.2)]),
            "1-0.749*(max(max(0,min(1,min((t-0.88)/0.12,(2.12-t)/0.12))),max(0,min(1,min((t-2.88)/0.12,(3.32-t)/0.12)))))"
        );
    }

    #[test]
    fn oriented_swaps_only_on_a_quarter_turn() {
        let v = VideoInfo {
            width: 1920,
            height: 1080,
            fps: 30.0,
        };
        let swapped = VideoInfo {
            width: 1080,
            height: 1920,
            fps: 30.0,
        };
        assert_eq!(oriented(v, 0), v);
        assert_eq!(oriented(v, 180), v);
        assert_eq!(oriented(v, -180), v);
        assert_eq!(oriented(v, 360), v);
        assert_eq!(oriented(v, 90), swapped);
        assert_eq!(oriented(v, -90), swapped);
        assert_eq!(oriented(v, 270), swapped);
        assert_eq!(oriented(v, -270), swapped);
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
    fn title_piece_uses_a_looped_png_input() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 3.0,
            text: "Hello".into(),
            subtitle: Some("sub".into()),
            style: TitleStyle::Accent,
        }];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.title_images = titles(&[(0, "/imgs/title-0.png")])
        })
        .unwrap();
        assert!(
            has_run(
                &args,
                &[
                    "-loop",
                    "1",
                    "-framerate",
                    "30",
                    "-t",
                    "3",
                    "-i",
                    "/imgs/title-0.png"
                ]
            ),
            "{args:?}"
        );
        let g = filter_complex(&args);
        assert!(
            g.contains("[1:v]format=yuv420p,setsar=1,trim=end=3,setpts=PTS-STARTPTS"),
            "{g}"
        );
        // A title always dips into and out of its neighbours.
        assert!(
            g.contains("fade=t=in:st=0:d=0.25,fade=t=out:st=2.75:d=0.25[v1]"),
            "{g}"
        );
        assert!(g.contains("anullsrc=r=48000:cl=stereo,atrim=end=3"), "{g}");
        assert!(g.contains("concat=n=3:v=1:a=1"), "{g}");
    }

    #[test]
    fn title_without_video_info_uses_the_default_frame_rate() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 1.0,
            text: "T".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.video = None;
            o.title_images = titles(&[(0, "/imgs/t.png")]);
        })
        .unwrap();
        assert!(has_run(&args, &["-framerate", "30", "-t", "1"]), "{args:?}");
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
        // No images at all: an audio-only export must not need or add them.
        let args = args_with(&edits, MediaKind::Audio, OutputFormat::Mp3, |_| {}).unwrap();
        assert!(!args.iter().any(|a| a == "-loop"), "{args:?}");
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1, "{args:?}");
        let g = filter_complex(&args);
        assert!(g.contains("anullsrc"), "{g}");
        assert!(!g.contains("overlay"), "{g}");
        assert!(g.contains("concat=n=3:v=0:a=1"), "{g}");
    }

    #[test]
    fn caption_overlays_its_segment_with_an_enable_window() {
        let edits = [
            cut(3.0, 5.0),
            Edit::Caption {
                start: 2.0,
                end: 7.0,
                text: "Ada".into(),
                position: CaptionPos::BottomCenter,
            },
        ];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.caption_images = captions(&[(1, "/imgs/cap-1.png", 64, 540)])
        })
        .unwrap();
        assert!(has_run(&args, &["-i", "/imgs/cap-1.png"]), "{args:?}");
        let g = filter_complex(&args);
        assert!(
            g.contains("overlay=x=64:y=540:enable='between(t,2,3)'"),
            "{g}"
        );
        assert!(g.contains("enable='between(t,0,2)'"), "{g}");
        // The overlay is a two-input filter, so the piece is built as a chain
        // and concat still consumes one label per piece.
        assert!(g.contains("[1:v]overlay="), "{g}");
        assert!(g.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1"), "{g}");
    }

    #[test]
    fn captions_fade_with_the_picture() {
        let edits = [
            cut(3.0, 5.0),
            Edit::Caption {
                start: 0.0,
                end: 3.0,
                text: "Ada".into(),
                position: CaptionPos::BottomLeft,
            },
        ];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.caption_images = captions(&[(1, "/imgs/cap-1.png", 64, 540)]);
            o.transition = Transition::Dip;
        });
        assert!(g.contains("enable='between(t,0,3)',fade=t=out"), "{g}");
    }

    #[test]
    fn missing_title_image_is_an_error() {
        let edits = [Edit::Title {
            at: 2.0,
            duration: 1.0,
            text: "T".into(),
            subtitle: None,
            style: TitleStyle::Dark,
        }];
        let err = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap_err();
        assert_eq!(err, ExportError::MissingTitleImage(0));
    }

    #[test]
    fn missing_caption_image_is_an_error() {
        let edits = [
            cut(3.0, 5.0),
            Edit::Caption {
                start: 2.0,
                end: 7.0,
                text: "Ada".into(),
                position: CaptionPos::BottomLeft,
            },
        ];
        let err = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap_err();
        assert_eq!(err, ExportError::MissingCaptionImage(1));
    }

    /// A title, an overdub and a caption together: the inputs are numbered in
    /// the order the planner pushes them (overdubs and titles in timeline
    /// order, then one input per caption), and every `[N:v]`/`[N:a]` label in
    /// the graph names the input at that position.
    #[test]
    fn title_overdub_and_caption_inputs_are_numbered_in_graph_order() {
        let mut audio = HashMap::new();
        audio.insert("/data/m/od.wav".to_owned(), PathBuf::from("/srv/od.wav"));
        let edits = [
            Edit::Title {
                at: 1.0,
                duration: 2.0,
                text: "One".into(),
                subtitle: None,
                style: TitleStyle::Dark,
            },
            Edit::Overdub {
                start: 4.0,
                end: 5.0,
                text: "hi".into(),
                audio_url: "/data/m/od.wav".into(),
                audio_duration: 1.5,
            },
            Edit::Caption {
                start: 6.0,
                end: 8.0,
                text: "Ada".into(),
                position: CaptionPos::BottomLeft,
            },
        ];
        let out = PathBuf::from("out.mp4");
        let mut options = opts(MediaKind::Video, OutputFormat::Mp4, &out, &audio);
        options.title_images = titles(&[(0, "/imgs/title-0.png")]);
        options.caption_images = captions(&[(2, "/imgs/cap-2.png", 64, 540)]);
        let args = build_ffmpeg_args(Path::new("in.mp4"), &edits, &options).unwrap();

        // The files given to `-i`, in argv order: source, title, overdub, caption.
        let inputs: Vec<&str> = args
            .iter()
            .enumerate()
            .filter(|(i, a)| *a == "-i" && *i + 1 < args.len())
            .map(|(i, _)| args[i + 1].as_str())
            .collect();
        assert_eq!(
            inputs,
            [
                "in.mp4",
                "/imgs/title-0.png",
                "/srv/od.wav",
                "/imgs/cap-2.png"
            ],
            "{args:?}"
        );
        // The title's looped still is the input right before its PNG.
        assert!(
            has_run(
                &args,
                &[
                    "-loop",
                    "1",
                    "-framerate",
                    "30",
                    "-t",
                    "2",
                    "-i",
                    "/imgs/title-0.png"
                ]
            ),
            "{args:?}"
        );

        let g = filter_complex(&args);
        // Each label points at the input at that position: 1 is the title
        // still, 2 the overdub WAV, 3 the caption overlay.
        assert!(g.contains("[1:v]format=yuv420p"), "{g}");
        assert!(g.contains("[2:a]aresample=48000"), "{g}");
        assert!(g.contains("[3:v]overlay=x=64:y=540"), "{g}");
        // Nothing refers to an input that was never opened, and no label
        // reads the source's picture as audio or the still as a waveform.
        assert!(!g.contains("[4:"), "{g}");
        assert!(!g.contains("[1:a]"), "{g}");
        assert!(!g.contains("[2:v]"), "{g}");
        assert!(!g.contains("[3:a]"), "{g}");
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

    #[test]
    fn reordered_pieces_concat_in_output_order() {
        let g = graph_with(&[], MediaKind::Video, OutputFormat::Mp4, |o| {
            o.splits = &[5.0];
            o.order = &[5.0, 0.0];
        });
        assert!(g.starts_with("[0:v]trim=start=5:end=10"), "{g}");
        assert!(g.contains("[0:v]trim=start=0:end=5"), "{g}");
        assert!(
            g.ends_with("[v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"),
            "{g}"
        );
    }

    #[test]
    fn broll_is_trimmed_delayed_scaled_and_overlaid_under_captions() {
        let edits = [Edit::Broll {
            start: 2.0,
            end: 4.0,
            media: "b1".into(),
            offset: 1.5,
        }];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4")]);
        })
        .unwrap();
        assert!(has_run(&args, &["-i", "/assets/b1.mp4"]));
        let g = filter_complex(&args);
        assert!(g.contains("[1:v]trim=start=1.5:end=3.5,setpts=PTS-STARTPTS+2/TB,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1[v0b0];"), "{g}");
        assert!(
            g.contains("[v0bo0][v0b0]overlay=x=0:y=0:eof_action=pass:enable='between(t,2,4)'[v0];"),
            "{g}"
        );
    }

    #[test]
    fn broll_across_a_reordered_boundary_offsets_into_the_asset() {
        // Window on the second output piece starts 1 s into the B-roll range.
        let edits = [Edit::Broll {
            start: 4.0,
            end: 6.0,
            media: "b1".into(),
            offset: 0.0,
        }];
        let g = graph_with(
            &[edits[0].clone()],
            MediaKind::Video,
            OutputFormat::Mp4,
            |o| {
                o.splits = &[5.0];
                o.order = &[5.0, 0.0];
                o.assets = assets(&[("b1", "/assets/b1.mp4")]);
            },
        );
        // First output piece [5,10): window [0,1) from asset offset 1.
        assert!(
            g.contains("[1:v]trim=start=1:end=2,setpts=PTS-STARTPTS+0/TB"),
            "{g}"
        );
        // Second output piece [0,5): window [4,5) from asset offset 0.
        assert!(
            g.contains("[2:v]trim=start=0:end=1,setpts=PTS-STARTPTS+4/TB"),
            "{g}"
        );
    }

    #[test]
    fn music_is_delayed_gained_ducked_and_mixed_before_fades() {
        let edits = [Edit::Audio {
            start: 2.0,
            end: 6.0,
            media: "m1".into(),
            offset: 10.0,
            gain: -6.0,
            duck: true,
        }];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("m1", "/assets/m1.mp3")]);
            o.words = leak([word(2.5, 3.0), word(3.1, 3.5)]);
        });
        assert!(g.contains("[a0m];"), "{g}");
        assert!(g.contains(&format!("[1:a]atrim=start=10:end=14,asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE},adelay=2000:all=1,volume=-6dB,volume=volume='1-0.749*(max(0,min(1,min((t-2.38)/0.12,(3.62-t)/0.12))))':eval=frame[a0x0];")), "{g}");
        assert!(
            g.contains("[a0m][a0x0]amix=inputs=2:normalize=0:duration=first[a0];"),
            "{g}"
        );
    }

    #[test]
    fn music_without_duck_or_words_has_no_envelope_and_fades_after_the_mix() {
        let edits = [
            Edit::Audio {
                start: 0.0,
                end: 10.0,
                media: "m1".into(),
                offset: 0.0,
                gain: 0.0,
                duck: false,
            },
            Edit::Title {
                at: 5.0,
                duration: 1.0,
                text: "T".into(),
                subtitle: None,
                style: TitleStyle::Dark,
            },
        ];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("m1", "/assets/m1.mp3")]);
            o.title_images = titles(&[(1, "/imgs/t.png")]);
        });
        assert!(g.contains("volume=0dB[a0x0];"), "{g}");
        assert!(!g.contains("eval=frame"), "{g}");
        assert!(
            g.contains("amix=inputs=2:normalize=0:duration=first,afade=t=out:st=4.75:d=0.25[a0];"),
            "{g}"
        );
    }

    #[test]
    fn overdub_hold_ducks_music_for_its_whole_length() {
        let edits = [
            Edit::Overdub {
                start: 2.0,
                end: 3.0,
                text: "x".into(),
                audio_url: "/data/m/od.wav".into(),
                audio_duration: 2.0,
            },
            Edit::Audio {
                start: 0.0,
                end: 10.0,
                media: "m1".into(),
                offset: 0.0,
                gain: 0.0,
                duck: true,
            },
        ];
        let mut overdub_audio = HashMap::new();
        overdub_audio.insert("/data/m/od.wav".to_owned(), PathBuf::from("/od.wav"));
        let out = PathBuf::from("out.mp4");
        let mut o = opts(MediaKind::Video, OutputFormat::Mp4, &out, &overdub_audio);
        o.assets = assets(&[("m1", "/assets/m1.mp3")]);
        let args = build_ffmpeg_args(Path::new("in.mp4"), &edits, &o).unwrap();
        let g = filter_complex(&args);
        // Segment 1 is the 2 s hold: the envelope covers [0,2].
        assert!(g.contains("(t--0.12)/0.12,(2.12-t)/0.12"), "{g}");
    }

    #[test]
    fn audio_only_export_mixes_music_and_ignores_broll() {
        let edits = [
            Edit::Broll {
                start: 1.0,
                end: 2.0,
                media: "b1".into(),
                offset: 0.0,
            },
            Edit::Audio {
                start: 0.0,
                end: 4.0,
                media: "m1".into(),
                offset: 0.0,
                gain: 0.0,
                duck: false,
            },
        ];
        let args = args_with(&edits, MediaKind::Audio, OutputFormat::Mp3, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4"), ("m1", "/assets/m1.mp3")]);
        })
        .unwrap();
        assert!(!has_run(&args, &["-i", "/assets/b1.mp4"]));
        assert!(has_run(&args, &["-i", "/assets/m1.mp3"]));
        assert!(filter_complex(&args).contains("amix=inputs=2"));
    }

    #[test]
    fn missing_asset_is_an_error() {
        let edits = [Edit::Audio {
            start: 0.0,
            end: 4.0,
            media: "m1".into(),
            offset: 0.0,
            gain: 0.0,
            duck: false,
        }];
        assert_eq!(
            args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap_err(),
            ExportError::MissingAsset("m1".into())
        );
    }
}
