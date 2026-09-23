//! Planning the ffmpeg render for an edit list.
//!
//! The main track is one or more source files laid end to end in stitched
//! time; each is its own input, in order. Every output piece becomes one
//! `trim`/`atrim` chain on the file that holds it (or, for an overdub, a
//! frozen first frame plus the synthesized WAV), fitted onto the canvas, and
//! the pieces are joined with the `concat` filter. Audio is resampled to a
//! common format first because `concat` refuses mismatched streams.
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
    audio_windows, caption_windows, joins, layer_windows, timeline_with, Segment, SegmentKind,
    Window, EPS, FADE,
};
use crate::types::{Edit, Frame, MediaKind, Range, Source, Transition, Word};
use crate::{locate, stitched_duration};

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

/// One main-track file as the planner sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInput {
    /// Where it sits on the stitched timeline.
    pub source: Source,
    /// The local file ffmpeg opens.
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Frame size and rate, when the file has a picture and it was probed.
    pub video: Option<VideoInfo>,
}

/// The frame every source is fitted onto: the first file with a picture
/// (an unprobed one counts as `DEFAULT_VIDEO`), else `DEFAULT_VIDEO`.
pub fn canvas(sources: &[SourceInput]) -> VideoInfo {
    sources
        .iter()
        .find(|s| s.kind == MediaKind::Video)
        .and_then(|s| s.video)
        .unwrap_or(DEFAULT_VIDEO)
}

/// `Video` when any source has a picture: the project renders to mp4.
pub fn sources_kind(sources: &[SourceInput]) -> MediaKind {
    if sources.iter().any(|s| s.kind == MediaKind::Video) {
        MediaKind::Video
    } else {
        MediaKind::Audio
    }
}

#[derive(Debug)]
pub struct ExportOptions<'a> {
    /// `Video` when any source has a picture; see `sources_kind`.
    pub kind: MediaKind,
    pub format: OutputFormat,
    pub output: &'a Path,
    /// Local audio file for each overdub edit's `audio_url`.
    pub overdub_audio: &'a HashMap<String, PathBuf>,
    /// PNG for each `Edit::Title`, by edit index, rendered at the frame size.
    pub title_images: &'a HashMap<usize, PathBuf>,
    /// PNG and its `overlay` placement for each `Edit::Caption`, by edit index.
    pub caption_images: &'a HashMap<usize, (PathBuf, u32, u32)>,
    /// Transition used at every join that does not override it.
    pub transition: Transition,
    pub splits: &'a [f64],
    pub order: &'a [f64],
    /// Local file for each media id a layer or audio edit names.
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
    #[error("the piece at {0} s lies outside every source")]
    OutsideSources(f64),
}

/// Audio format every segment is coerced to before `concat`.
const AUDIO_NORMALIZE: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";

/// Picture-in-picture width as a fraction of the canvas width.
pub const PIP_WIDTH: f64 = 0.3;
/// Picture-in-picture inset from its corner, as a fraction of each canvas side.
pub const PIP_MARGIN: f64 = 0.04;

/// Build the full ffmpeg argv (without the program name) that renders `edits`
/// over the main-track `sources` into `opts.output`. Input `k` is `sources[k]`.
pub fn build_ffmpeg_args(
    sources: &[SourceInput],
    edits: &[Edit],
    opts: &ExportOptions,
) -> Result<Vec<String>, ExportError> {
    if sources.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let placed: Vec<Source> = sources.iter().map(|s| s.source.clone()).collect();
    let segments = split_at_joins(
        timeline_with(stitched_duration(&placed), edits, opts.splits, opts.order),
        &placed,
    );
    if segments.is_empty() {
        return Err(ExportError::NothingToExport);
    }
    let with_video = opts.kind == MediaKind::Video;
    if !with_video && opts.format == OutputFormat::Mp4 {
        return Err(ExportError::FormatMismatch("mp4"));
    }
    let render_video = with_video && opts.format == OutputFormat::Mp4;

    let mut args: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error"]
        .map(String::from)
        .to_vec();
    for s in sources {
        args.push("-i".into());
        args.push(s.path.to_string_lossy().into_owned());
    }

    let video_info = canvas(sources);
    let windows = caption_windows(&segments, edits);

    // Overdub WAVs, title cards, captions and inserts become extra inputs,
    // numbered in the order they are pushed after the sources.
    let mut next_input = sources.len();
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
    let layers = layer_windows(&segments, edits);
    let audios = audio_windows(&segments, edits);
    let asset_path = |id: &str| {
        opts.assets
            .get(id)
            .ok_or_else(|| ExportError::MissingAsset(id.to_owned()))
    };
    // A layer opens its file when its picture is drawn or its sound is mixed;
    // a muted layer in an audio-only render needs nothing.
    let mut layer_input: Vec<Vec<Option<usize>>> = vec![Vec::new(); segments.len()];
    for (i, ws) in layers.iter().enumerate() {
        for w in ws {
            let Edit::Layer { media, audio, .. } = &edits[w.index] else {
                continue;
            };
            let opened = if render_video || audio.is_some() {
                args.push("-i".into());
                args.push(asset_path(media)?.to_string_lossy().into_owned());
                next_input += 1;
                Some(next_input - 1)
            } else {
                None
            };
            layer_input[i].push(opened);
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
        let (k, local) = place(&placed, seg.source)?;
        let file = &sources[k];
        let picture = file.kind == MediaKind::Video;
        let (v, a) = match seg.kind {
            SegmentKind::Source => (
                render_video.then(|| {
                    if picture {
                        video_trim(k, local, &fit(file, video_info))
                    } else {
                        black(video_info, hold)
                    }
                }),
                audio_trim(k, local),
            ),
            SegmentKind::Overdub { index } => (
                render_video.then(|| {
                    if picture {
                        freeze_frame(
                            k,
                            local.start,
                            file.source.duration,
                            hold,
                            &fit(file, video_info),
                        )
                    } else {
                        black(video_info, hold)
                    }
                }),
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
            for (n, w) in layers[i].iter().enumerate() {
                let (
                    Edit::Layer {
                        start,
                        offset,
                        frame,
                        ..
                    },
                    Some(input),
                ) = (&edits[w.index], layer_input[i][n])
                else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let (scale, x, y) = layer_placement(*frame, video_info);
                let _ = write!(
                    graph,
                    "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS+{}/TB,{scale},setsar=1[v{i}b{n}];",
                    fmt(from),
                    fmt(from + (w.end - w.start)),
                    fmt(w.start)
                );
                let _ = write!(graph, "{chain}[v{i}bo{n}];");
                chain = format!(
                    "[v{i}bo{n}][v{i}b{n}]overlay=x={x}:y={y}:eof_action=pass:enable='between(t,{},{})'",
                    fmt(w.start),
                    fmt(w.end)
                );
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
        // Music beds and layers with sound mix under the piece's own audio.
        let sounding: Vec<(usize, &Window, f64, usize)> = layers[i]
            .iter()
            .enumerate()
            .filter_map(|(n, w)| match &edits[w.index] {
                Edit::Layer {
                    audio: Some(db), ..
                } => Some((n, w, *db, layer_input[i][n]?)),
                _ => None,
            })
            .collect();
        let mut mixed = if audios[i].is_empty() && sounding.is_empty() {
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
                let runs = if *duck {
                    duck_runs(seg, w, opts.words)
                } else {
                    Vec::new()
                };
                let from = offset + (w.source_start - start);
                let _ = write!(
                    graph,
                    "{}[a{i}x{n}];",
                    bed(audio_input[i][n], from, w, *gain, &runs)
                );
                let _ = write!(labels, "[a{i}x{n}]");
            }
            // A layer has no duck switch: its sound always dips under speech.
            for (n, w, db, input) in &sounding {
                let Edit::Layer { start, offset, .. } = &edits[w.index] else {
                    continue;
                };
                let from = offset + (w.source_start - start);
                let runs = duck_runs(seg, w, opts.words);
                let _ = write!(graph, "{}[a{i}l{n}];", bed(*input, from, w, *db, &runs));
                let _ = write!(labels, "[a{i}l{n}]");
            }
            format!(
                "{labels}amix=inputs={}:normalize=0:duration=first",
                audios[i].len() + sounding.len() + 1
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

fn video_trim(input: usize, r: Range, fit: &str) -> String {
    format!(
        "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS{fit},setsar=1",
        fmt(r.start),
        fmt(r.end)
    )
}

fn audio_trim(input: usize, r: Range) -> String {
    format!(
        "[{input}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}",
        fmt(r.start),
        fmt(r.end)
    )
}

/// Take the frame at local time `at` of a file `duration` long and clone it
/// for `hold` seconds.
fn freeze_frame(input: usize, at: f64, duration: f64, hold: f64, fit: &str) -> String {
    // A one-second window guarantees at least one frame at any sane frame rate.
    let window_end = (at + 1.0).min(duration);
    format!(
        "[{input}:v]trim=start={}:end={},setpts=PTS-STARTPTS,select=eq(n\\,0),\
         tpad=stop_mode=clone:stop_duration={hold},trim=end={hold},setpts=PTS-STARTPTS{fit},setsar=1",
        fmt(at),
        fmt(window_end),
        hold = fmt(hold),
    )
}

/// Black at the canvas size for `hold` seconds: the picture of an audio-only source.
fn black(canvas: VideoInfo, hold: f64) -> String {
    format!(
        "color=c=black:s={}x{}:r={}:d={},format=yuv420p,setsar=1",
        canvas.width,
        canvas.height,
        rate(canvas.fps),
        fmt(hold)
    )
}

/// Filters that bring `file`'s picture onto the canvas: nothing when it is
/// known to match already, else scale to fit, letterbox and resample the
/// frame rate. An unprobed picture is always fitted: its size is a guess.
fn fit(file: &SourceInput, canvas: VideoInfo) -> String {
    if let Some(v) = file.video {
        if v.width == canvas.width && v.height == canvas.height && (v.fps - canvas.fps).abs() < 1e-3
        {
            return String::new();
        }
    }
    format!(
        ",scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,fps={}",
        rate(canvas.fps),
        w = canvas.width,
        h = canvas.height,
    )
}

/// A frame rate for ffmpeg: NTSC rates as their exact fraction, others as
/// a plain number.
fn rate(fps: f64) -> String {
    let ntsc = fps * 1001.0 / 1000.0;
    if (ntsc - ntsc.round()).abs() < 1e-3 && (fps - fps.round()).abs() > 1e-3 {
        format!("{}000/1001", ntsc.round() as i64)
    } else {
        fmt(fps)
    }
}

/// The input holding a piece, and the piece's range in that file's own time.
/// A piece that starts outside every file is a planner bug, not input 0.
fn place(placed: &[Source], r: Range) -> Result<(usize, Range), ExportError> {
    locate(placed, r.start)
        .map(|(k, local)| (k, Range::new(local, local + r.len())))
        .ok_or(ExportError::OutsideSources(r.start))
}

/// Split every source piece at each join strictly inside it, so each piece
/// reads one file. Holds and titles are placed by their start and left whole.
fn split_at_joins(segments: Vec<Segment>, placed: &[Source]) -> Vec<Segment> {
    let joins: Vec<f64> = placed.iter().skip(1).map(|s| s.offset).collect();
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        if seg.kind != SegmentKind::Source {
            out.push(seg);
            continue;
        }
        let shift = seg.output.start - seg.source.start;
        let mut cursor = seg.source.start;
        for &j in &joins {
            if j > cursor + EPS && j < seg.source.end - EPS {
                out.push(Segment {
                    source: Range::new(cursor, j),
                    output: Range::new(cursor + shift, j + shift),
                    kind: SegmentKind::Source,
                });
                cursor = j;
            }
        }
        out.push(Segment {
            source: Range::new(cursor, seg.source.end),
            output: Range::new(cursor + shift, seg.output.end),
            kind: SegmentKind::Source,
        });
    }
    out
}

/// An inserted sound: `w`'s length of `input` from `from`, placed at
/// `w.start` in the piece, at `gain` dB, dipped under `runs` of speech.
fn bed(input: usize, from: f64, w: &Window, gain: f64, runs: &[Range]) -> String {
    let mut m = format!(
        "[{input}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE},adelay={}:all=1,volume={}dB",
        fmt(from),
        fmt(from + (w.end - w.start)),
        (w.start * 1000.0).round() as i64,
        fmt(gain)
    );
    if !runs.is_empty() {
        let _ = write!(m, ",volume=volume='{}':eval=frame", duck_expr(runs));
    }
    m
}

/// A layer's scale filter and its `overlay` x and y on the canvas. Full frame
/// is fitted and letterboxed; picture-in-picture is `PIP_WIDTH` of the canvas
/// wide and `PIP_MARGIN` of each side in from its corner.
fn layer_placement(frame: Frame, canvas: VideoInfo) -> (String, String, String) {
    let (w, h) = (canvas.width, canvas.height);
    let pip = || format!("scale={}:-2", even(f64::from(w) * PIP_WIDTH));
    let mx = (f64::from(w) * PIP_MARGIN).round() as u32;
    let my = (f64::from(h) * PIP_MARGIN).round() as u32;
    let right = format!("main_w-overlay_w-{mx}");
    let bottom = format!("main_h-overlay_h-{my}");
    match frame {
        Frame::Full => (
            format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"),
            "0".into(),
            "0".into(),
        ),
        Frame::PipTopLeft => (pip(), mx.to_string(), my.to_string()),
        Frame::PipTopRight => (pip(), right, my.to_string()),
        Frame::PipBottomLeft => (pip(), mx.to_string(), bottom),
        Frame::PipBottomRight => (pip(), right, bottom),
    }
}

/// The nearest even whole number; yuv420p needs even sizes.
fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32) * 2
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
    use crate::types::{CaptionPos, Frame, TitleStyle};

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
            kind,
            format,
            output,
            overdub_audio,
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

    const HD: VideoInfo = VideoInfo {
        width: 1280,
        height: 720,
        fps: 30.0,
    };

    /// One main-track file at `offset` for `duration` seconds.
    fn file(
        media: &str,
        path: &str,
        offset: f64,
        duration: f64,
        kind: MediaKind,
        video: Option<VideoInfo>,
    ) -> SourceInput {
        SourceInput {
            source: Source {
                media: media.into(),
                offset,
                duration,
            },
            path: PathBuf::from(path),
            kind,
            video,
        }
    }

    /// The ten-second single file every older test renders.
    fn single(kind: MediaKind) -> Vec<SourceInput> {
        match kind {
            MediaKind::Video => vec![file("m0", "in.mp4", 0.0, 10.0, kind, Some(HD))],
            MediaKind::Audio => vec![file("m0", "in.mp3", 0.0, 10.0, kind, None)],
        }
    }

    /// The whole argv, so tests can look at the inputs as well as the graph.
    fn args_with(
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> Result<Vec<String>, ExportError> {
        args_from(&single(kind), edits, kind, format, tweak)
    }

    /// Same, over an explicit list of main-track files.
    fn args_from(
        sources: &[SourceInput],
        edits: &[Edit],
        kind: MediaKind,
        format: OutputFormat,
        tweak: impl FnOnce(&mut ExportOptions),
    ) -> Result<Vec<String>, ExportError> {
        let none = HashMap::new();
        let out = PathBuf::from(format!("out.{}", format.extension()));
        let mut options = opts(kind, format, &out, &none);
        tweak(&mut options);
        build_ffmpeg_args(sources, edits, &options)
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
            &single(MediaKind::Video),
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
            &single(MediaKind::Video),
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
            &single(MediaKind::Audio),
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
            &single(MediaKind::Video),
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
            &single(MediaKind::Video),
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
            &single(MediaKind::Video),
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
            &single(MediaKind::Audio),
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
        let mut sources = single(MediaKind::Video);
        sources[0].video = None;
        let args = args_from(&sources, &edits, MediaKind::Video, OutputFormat::Mp4, |o| {
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
        let args = build_ffmpeg_args(&single(MediaKind::Video), &edits, &options).unwrap();

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
        let edits = [Edit::Layer {
            track: 2,
            start: 2.0,
            end: 4.0,
            media: "b1".into(),
            offset: 1.5,
            frame: Frame::Full,
            audio: None,
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
        let edits = [Edit::Layer {
            track: 2,
            start: 4.0,
            end: 6.0,
            media: "b1".into(),
            offset: 0.0,
            frame: Frame::Full,
            audio: None,
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
    fn music_plays_over_a_title_card_instead_of_being_silenced() {
        // Pieces: [0,5) T(1 s) [5,10). The card's base stays silence, with the
        // bed mixed on top of it, so the music does not drop out under an intro.
        let edits = [
            Edit::Audio {
                start: 0.0,
                end: 10.0,
                media: "m1".into(),
                offset: 0.0,
                gain: 0.0,
                duck: true,
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
            o.words = leak([word(1.0, 2.0)]);
        });
        // Segment 1 is the card: silence as the base, music mixed onto it.
        assert!(g.contains(&format!("anullsrc=r=48000:cl=stereo,atrim=end=1,asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE}[a1m];")), "{g}");
        assert!(
            g.contains(&format!("atrim=start=5:end=6,asetpts=PTS-STARTPTS,{AUDIO_NORMALIZE},adelay=0:all=1,volume=0dB[a1x0];")),
            "{g}"
        );
        assert!(
            g.contains("[a1m][a1x0]amix=inputs=2:normalize=0:duration=first,afade=t=in"),
            "{g}"
        );
        // A title has no words, so nothing ducks on the card.
        assert!(!g.contains("[a1x0];volume=volume="), "{g}");
    }

    #[test]
    fn a_caption_or_broll_spanning_a_title_draws_nothing_on_the_card() {
        let edits = [
            Edit::Title {
                at: 5.0,
                duration: 1.0,
                text: "T".into(),
                subtitle: None,
                style: TitleStyle::Dark,
            },
            Edit::Caption {
                start: 0.0,
                end: 10.0,
                text: "c".into(),
                position: CaptionPos::BottomLeft,
            },
            Edit::Layer {
                track: 2,
                start: 0.0,
                end: 10.0,
                media: "b1".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
            },
        ];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4")]);
            o.title_images = titles(&[(0, "/imgs/t.png")]);
            o.caption_images = captions(&[(1, "/imgs/c.png", 10, 20)]);
        });
        // The card's video chain goes straight from the still to [v1].
        assert!(!g.contains("[v1c0]"), "{g}");
        assert!(!g.contains("[v1b0]"), "{g}");
        assert!(
            g.contains("trim=end=1,setpts=PTS-STARTPTS,fade=t=in"),
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
        let args = build_ffmpeg_args(&single(MediaKind::Video), &edits, &o).unwrap();
        let g = filter_complex(&args);
        // Segment 1 is the 2 s hold: the envelope covers [0,2].
        assert!(g.contains("(t--0.12)/0.12,(2.12-t)/0.12"), "{g}");
    }

    #[test]
    fn audio_only_export_mixes_music_and_ignores_broll() {
        let edits = [
            Edit::Layer {
                track: 2,
                start: 1.0,
                end: 2.0,
                media: "b1".into(),
                offset: 0.0,
                frame: Frame::Full,
                audio: None,
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

    const FHD: VideoInfo = VideoInfo {
        width: 1920,
        height: 1080,
        fps: 30.0,
    };
    const PAL: VideoInfo = VideoInfo {
        width: 1280,
        height: 720,
        fps: 25.0,
    };
    const AN: &str = AUDIO_NORMALIZE;
    /// What a 720p25 file gets to sit on a 1080p30 canvas.
    const FIT: &str = ",scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,fps=30";

    /// A 1080p30 file for 4 s, then a 720p25 file for 6 s.
    fn two_files() -> Vec<SourceInput> {
        vec![
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(FHD)),
            file("m1", "b.mov", 4.0, 6.0, MediaKind::Video, Some(PAL)),
        ]
    }

    #[test]
    fn one_source_graph_is_exactly_what_it_was() {
        let g = graph(&[cut(2.0, 4.0)], MediaKind::Video, OutputFormat::Mp4);
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=2,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [0:v]trim=start=4:end=10,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [0:a]atrim=start=4:end=10,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn two_sources_are_trimmed_from_their_own_files_and_fitted_to_the_first() {
        // The fold's permanent split at the join is in `splits`, as AddSource leaves it.
        let args = args_from(
            &two_files(),
            &[cut(1.0, 2.0)],
            MediaKind::Video,
            OutputFormat::Mp4,
            |o| o.splits = &[4.0],
        )
        .unwrap();
        assert_eq!(
            &args[..8],
            &[
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                "a.mp4",
                "-i",
                "b.mov"
            ]
        );
        assert_eq!(
            filter_complex(&args),
            format!(
                "[0:v]trim=start=0:end=1,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=1,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [0:v]trim=start=2:end=4,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [0:a]atrim=start=2:end=4,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [1:v]trim=start=0:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v2];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a2];\
                 [v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[outv][outa]"
            )
        );
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
    }

    #[test]
    fn a_piece_that_crosses_a_join_is_split_there() {
        // No splits at all: the planner still reads each file for its own part.
        let g = filter_complex(
            &args_from(
                &two_files(),
                &[],
                MediaKind::Video,
                OutputFormat::Mp4,
                |_| {},
            )
            .unwrap(),
        )
        .to_owned();
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [1:v]trim=start=0:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v1];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn an_overdub_on_the_second_file_freezes_that_files_frame_on_the_canvas() {
        let edits = [Edit::Overdub {
            start: 5.0,
            end: 6.0,
            text: "hi".into(),
            audio_url: "/data/m/od.wav".into(),
            audio_duration: 2.0,
        }];
        let od = leak(HashMap::from([(
            "/data/m/od.wav".to_owned(),
            PathBuf::from("/srv/od.wav"),
        )]));
        let args = args_from(
            &two_files(),
            &edits,
            MediaKind::Video,
            OutputFormat::Mp4,
            |o| {
                o.overdub_audio = od;
            },
        )
        .unwrap();
        // Pieces: [0,4) file 0, [4,5) file 1, the 2 s hold, [6,10) file 1.
        assert!(has_run(&args, &["-i", "/srv/od.wav"]), "{args:?}");
        let g = filter_complex(&args);
        assert!(
            g.contains(&format!(
                "[1:v]trim=start=1:end=2,setpts=PTS-STARTPTS,select=eq(n\\,0),\
                 tpad=stop_mode=clone:stop_duration=2,trim=end=2,setpts=PTS-STARTPTS{FIT},setsar=1[v2];"
            )),
            "{g}"
        );
        assert!(g.contains("[2:a]aresample=48000"), "{g}");
        assert!(
            g.contains(&format!(
                "[1:v]trim=start=2:end=6,setpts=PTS-STARTPTS{FIT},setsar=1[v3];"
            )),
            "{g}"
        );
        assert!(g.ends_with("concat=n=4:v=1:a=1[outv][outa]"), "{g}");
    }

    #[test]
    fn an_audio_only_source_shows_black_on_the_canvas() {
        let sources = [
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(FHD)),
            file("m1", "b.m4a", 4.0, 6.0, MediaKind::Audio, None),
        ];
        let g = filter_complex(
            &args_from(&sources, &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap(),
        )
        .to_owned();
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a0];\
                 color=c=black:s=1920x1080:r=30:d=6,format=yuv420p,setsar=1[v1];\
                 [1:a]atrim=start=0:end=6,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [v0][a0][v1][a1]concat=n=2:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn the_canvas_is_the_first_file_with_a_picture() {
        assert_eq!(canvas(&two_files()), FHD);
        assert_eq!(sources_kind(&two_files()), MediaKind::Video);
        let audio_first = [
            file("m0", "a.m4a", 0.0, 4.0, MediaKind::Audio, None),
            file("m1", "b.mov", 4.0, 6.0, MediaKind::Video, Some(PAL)),
        ];
        assert_eq!(canvas(&audio_first), PAL);
        assert_eq!(sources_kind(&audio_first), MediaKind::Video);
        let audio_only = [file("m0", "a.m4a", 0.0, 4.0, MediaKind::Audio, None)];
        assert_eq!(canvas(&audio_only), DEFAULT_VIDEO);
        assert_eq!(sources_kind(&audio_only), MediaKind::Audio);
        let unprobed = [file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, None)];
        assert_eq!(canvas(&unprobed), DEFAULT_VIDEO);
    }

    #[test]
    fn rate_writes_ntsc_rates_as_fractions() {
        assert_eq!(rate(30.0), "30");
        assert_eq!(rate(25.0), "25");
        assert_eq!(rate(30000.0 / 1001.0), "30000/1001");
        assert_eq!(rate(24000.0 / 1001.0), "24000/1001");
        assert_eq!(rate(60000.0 / 1001.0), "60000/1001");
    }

    #[test]
    fn no_sources_is_nothing_to_export() {
        assert_eq!(
            args_from(&[], &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap_err(),
            ExportError::NothingToExport
        );
    }

    fn layer(
        track: u8,
        start: f64,
        end: f64,
        media: &str,
        offset: f64,
        frame: Frame,
        audio: Option<f64>,
    ) -> Edit {
        Edit::Layer {
            track,
            start,
            end,
            media: media.into(),
            offset,
            frame,
            audio,
        }
    }

    /// The files given to `-i`, in argv order.
    fn inputs(args: &[String]) -> Vec<&str> {
        args.iter()
            .enumerate()
            .filter(|(i, a)| *a == "-i" && *i + 1 < args.len())
            .map(|(i, _)| args[i + 1].as_str())
            .collect()
    }

    #[test]
    fn v2_full_frame_then_v3_pip_with_sound_are_stacked_and_mixed() {
        // V3 is listed first on purpose: stacking follows the track, not the log.
        let edits = [
            layer(3, 2.0, 5.0, "p1", 1.0, Frame::PipTopRight, Some(-6.0)),
            layer(2, 1.0, 6.0, "b1", 0.0, Frame::Full, None),
        ];
        let args = args_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4"), ("p1", "/assets/p1.mp4")]);
            o.words = leak([word(2.5, 3.0)]);
        })
        .unwrap();
        assert_eq!(
            inputs(&args),
            ["in.mp4", "/assets/b1.mp4", "/assets/p1.mp4"]
        );
        assert_eq!(
            filter_complex(&args),
            format!(
                "[1:v]trim=start=0:end=5,setpts=PTS-STARTPTS+1/TB,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1[v0b0];\
                 [0:v]trim=start=0:end=10,setpts=PTS-STARTPTS,setsar=1[v0bo0];\
                 [2:v]trim=start=1:end=4,setpts=PTS-STARTPTS+2/TB,scale=384:-2,setsar=1[v0b1];\
                 [v0bo0][v0b0]overlay=x=0:y=0:eof_action=pass:enable='between(t,1,6)'[v0bo1];\
                 [v0bo1][v0b1]overlay=x=main_w-overlay_w-51:y=29:eof_action=pass:enable='between(t,2,5)'[v0];\
                 [0:a]atrim=start=0:end=10,asetpts=PTS-STARTPTS,{AN}[a0m];\
                 [2:a]atrim=start=1:end=4,asetpts=PTS-STARTPTS,{AN},adelay=2000:all=1,volume=-6dB,volume=volume='1-0.749*(max(0,min(1,min((t-2.38)/0.12,(3.12-t)/0.12))))':eval=frame[a0l1];\
                 [a0m][a0l1]amix=inputs=2:normalize=0:duration=first[a0];\
                 [v0][a0]concat=n=1:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn a_sounding_layer_mixes_beside_music() {
        let edits = [
            Edit::Audio {
                start: 0.0,
                end: 10.0,
                media: "m1".into(),
                offset: 0.0,
                gain: -12.0,
                duck: false,
            },
            layer(2, 4.0, 6.0, "b1", 0.0, Frame::Full, Some(3.0)),
        ];
        let g = graph_with(&edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.assets = assets(&[("m1", "/assets/m1.mp3"), ("b1", "/assets/b1.mp4")]);
        });
        // Inputs: source 0, layer 1, music 2.
        assert!(g.contains(&format!("[1:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN},adelay=4000:all=1,volume=3dB[a0l0];")), "{g}");
        assert!(
            g.contains("[a0m][a0x0][a0l0]amix=inputs=3:normalize=0:duration=first[a0];"),
            "{g}"
        );
    }

    #[test]
    fn audio_only_export_mixes_a_sounding_layer_and_skips_muted_ones() {
        let edits = [
            layer(2, 1.0, 3.0, "b1", 0.0, Frame::Full, None),
            layer(3, 4.0, 6.0, "p1", 0.0, Frame::PipBottomLeft, Some(0.0)),
        ];
        let args = args_with(&edits, MediaKind::Audio, OutputFormat::Mp3, |o| {
            o.assets = assets(&[("b1", "/assets/b1.mp4"), ("p1", "/assets/p1.mp4")]);
        })
        .unwrap();
        assert_eq!(inputs(&args), ["in.mp3", "/assets/p1.mp4"]);
        let g = filter_complex(&args);
        assert!(!g.contains("overlay"), "{g}");
        assert!(g.contains(&format!("[1:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN},adelay=4000:all=1,volume=0dB[a0l1];")), "{g}");
        assert!(
            g.contains("[a0m][a0l1]amix=inputs=2:normalize=0:duration=first[a0];"),
            "{g}"
        );
    }

    #[test]
    fn pip_sits_four_percent_in_from_its_corner_at_thirty_percent_width() {
        let c = VideoInfo {
            width: 1920,
            height: 1080,
            fps: 30.0,
        };
        let s = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
        assert_eq!(
            layer_placement(Frame::Full, c),
            s("scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2", "0", "0")
        );
        assert_eq!(
            layer_placement(Frame::PipTopLeft, c),
            s("scale=576:-2", "77", "43")
        );
        assert_eq!(
            layer_placement(Frame::PipTopRight, c),
            s("scale=576:-2", "main_w-overlay_w-77", "43")
        );
        assert_eq!(
            layer_placement(Frame::PipBottomLeft, c),
            s("scale=576:-2", "77", "main_h-overlay_h-43")
        );
        assert_eq!(
            layer_placement(Frame::PipBottomRight, c),
            s("scale=576:-2", "main_w-overlay_w-77", "main_h-overlay_h-43")
        );
    }

    #[test]
    fn a_layer_on_a_later_piece_is_numbered_after_the_sources() {
        // Two files: layer inputs start at 2, and the window is relative to its piece.
        let sources = [
            file("m0", "a.mp4", 0.0, 4.0, MediaKind::Video, Some(HD)),
            file("m1", "b.mp4", 4.0, 6.0, MediaKind::Video, Some(HD)),
        ];
        let edits = [layer(3, 5.0, 7.0, "m0", 1.0, Frame::PipBottomRight, None)];
        let args = args_from(&sources, &edits, MediaKind::Video, OutputFormat::Mp4, |o| {
            o.splits = &[4.0];
            o.assets = assets(&[("m0", "/data/m0/source.mp4")]);
        })
        .unwrap();
        assert_eq!(inputs(&args), ["a.mp4", "b.mp4", "/data/m0/source.mp4"]);
        let g = filter_complex(&args);
        assert!(
            g.contains(
                "[2:v]trim=start=1:end=3,setpts=PTS-STARTPTS+1/TB,scale=384:-2,setsar=1[v1b0];"
            ),
            "{g}"
        );
        assert!(g.contains("[v1bo0][v1b0]overlay=x=main_w-overlay_w-51:y=main_h-overlay_h-29:eof_action=pass:enable='between(t,1,3)'[v1];"), "{g}");
    }

    // Task 5 follow-ups: unprobed pictures, pieces outside every file, joins.

    #[test]
    fn an_unprobed_video_is_always_fitted_even_on_the_default_canvas() {
        let mut sources = single(MediaKind::Video);
        sources[0].video = None;
        assert_eq!(canvas(&sources), DEFAULT_VIDEO);
        let g = filter_complex(
            &args_from(&sources, &[], MediaKind::Video, OutputFormat::Mp4, |_| {}).unwrap(),
        )
        .to_owned();
        assert!(
            g.starts_with("[0:v]trim=start=0:end=10,setpts=PTS-STARTPTS,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,fps=30,setsar=1[v0];"),
            "{g}"
        );
    }

    #[test]
    fn a_piece_outside_every_file_is_an_error_not_input_zero() {
        let placed = [Source {
            media: "m0".into(),
            offset: 0.0,
            duration: 4.0,
        }];
        assert_eq!(
            place(&placed, Range::new(6.0, 8.0)),
            Err(ExportError::OutsideSources(6.0))
        );
        assert_eq!(
            place(&placed, Range::new(1.0, 2.0)),
            Ok((0, Range::new(1.0, 2.0)))
        );
    }

    /// Three HD files: [0,3), [3,6), [6,10).
    fn three_files() -> Vec<SourceInput> {
        vec![
            file("m0", "a.mp4", 0.0, 3.0, MediaKind::Video, Some(HD)),
            file("m1", "b.mp4", 3.0, 3.0, MediaKind::Video, Some(HD)),
            file("m2", "c.mp4", 6.0, 4.0, MediaKind::Video, Some(HD)),
        ]
    }

    #[test]
    fn a_piece_spanning_two_joins_reads_each_file_for_its_part() {
        let g = filter_complex(
            &args_from(
                &three_files(),
                &[],
                MediaKind::Video,
                OutputFormat::Mp4,
                |_| {},
            )
            .unwrap(),
        )
        .to_owned();
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=3,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=3,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [1:v]trim=start=0:end=3,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [1:a]atrim=start=0:end=3,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [2:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v2];\
                 [2:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a2];\
                 [v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn a_cut_across_a_join_keeps_each_side_in_its_own_file() {
        // Cut [2,5) removes the end of file 0 and the start of file 1.
        let g = filter_complex(
            &args_from(
                &three_files(),
                &[cut(2.0, 5.0)],
                MediaKind::Video,
                OutputFormat::Mp4,
                |o| o.splits = &[3.0, 6.0],
            )
            .unwrap(),
        )
        .to_owned();
        // File 0 keeps [0,2); file 1 resumes at its own second 2.
        assert_eq!(
            g,
            format!(
                "[0:v]trim=start=0:end=2,setpts=PTS-STARTPTS,setsar=1[v0];\
                 [0:a]atrim=start=0:end=2,asetpts=PTS-STARTPTS,{AN}[a0];\
                 [1:v]trim=start=2:end=3,setpts=PTS-STARTPTS,setsar=1[v1];\
                 [1:a]atrim=start=2:end=3,asetpts=PTS-STARTPTS,{AN}[a1];\
                 [2:v]trim=start=0:end=4,setpts=PTS-STARTPTS,setsar=1[v2];\
                 [2:a]atrim=start=0:end=4,asetpts=PTS-STARTPTS,{AN}[a2];\
                 [v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[outv][outa]"
            )
        );
    }

    #[test]
    fn an_instant_within_eps_of_a_join_belongs_to_the_later_file() {
        let placed: Vec<Source> = three_files().into_iter().map(|s| s.source).collect();
        let near = 3.0 - EPS / 2.0;
        // A piece starting just before the join reads file 1 from its start.
        let (k, local) = place(&placed, Range::new(near, 5.0)).unwrap();
        assert_eq!(k, 1);
        assert!(local.start.abs() < EPS, "{local:?}");
        // A piece ending within EPS of the join is not split into a sliver.
        let seg = Segment {
            source: Range::new(1.0, 3.0 + EPS / 2.0),
            output: Range::new(1.0, 3.0 + EPS / 2.0),
            kind: SegmentKind::Source,
        };
        assert_eq!(split_at_joins(vec![seg.clone()], &placed), vec![seg]);
    }
}
