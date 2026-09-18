//! Shelling out to ffprobe, ffmpeg and whisper-cli.

use std::path::{Path, PathBuf};

use std::process::Stdio;

use anyhow::{anyhow, Context};
use engine::{
    diarize_args, parse_diarization, parse_progress_line, parse_silencedetect, parse_whisper_json,
    progress_fraction, silencedetect_args, whisper_args, DiarizeOptions, MediaKind, ProgressEvent,
    Range, SpeakerTurn, VideoInfo, Word, PROGRESS_ARGS,
};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Last few lines of a tool's stderr, for error messages.
fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let tail: Vec<&str> = text.lines().rev().take(8).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn check_status(program: &str, output: &std::process::Output) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "{program} exited with {}: {}",
        output.status,
        stderr_tail(&output.stderr).trim()
    ))
}

fn start_error(program: &str) -> String {
    format!("failed to start {program} (is it installed and on PATH?)")
}

/// Run a tool, returning stdout or an error that includes stderr.
async fn run(program: &str, args: &[String]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| start_error(program))?;
    check_status(program, &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub duration: f64,
    pub kind: MediaKind,
    /// Frame size and rate of the picture track, when there is one.
    pub video: Option<VideoInfo>,
}

#[derive(Deserialize)]
struct ProbeOutput {
    format: ProbeFormat,
    #[serde(default)]
    streams: Vec<ProbeStream>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: String,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: String,
    #[serde(default)]
    disposition: ProbeDisposition,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    r_frame_rate: Option<String>,
}

/// ffprobe reports the frame rate as a rational, e.g. `"30000/1001"`.
fn parse_frame_rate(rate: Option<&str>) -> f64 {
    let fallback = 30.0;
    let Some((num, den)) = rate.and_then(|r| r.split_once('/')) else {
        return rate.and_then(|r| r.parse().ok()).unwrap_or(fallback);
    };
    match (num.parse::<f64>(), den.parse::<f64>()) {
        (Ok(num), Ok(den)) if den > 0.0 && num > 0.0 => num / den,
        _ => fallback,
    }
}

#[derive(Deserialize, Default)]
struct ProbeDisposition {
    #[serde(default)]
    attached_pic: u8,
}

/// Duration and whether there is a real picture track (cover art doesn't count).
pub async fn probe(path: &Path) -> anyhow::Result<Probe> {
    let args = [
        "-v",
        "error",
        "-show_entries",
        "format=duration:stream=codec_type,width,height,r_frame_rate:stream_disposition=attached_pic",
        "-of",
        "json",
    ]
    .map(String::from)
    .into_iter()
    .chain([path.to_string_lossy().into_owned()])
    .collect::<Vec<_>>();
    let json = run("ffprobe", &args).await?;
    let out: ProbeOutput = serde_json::from_str(&json).context("unexpected ffprobe output")?;
    let duration: f64 = out.format.duration.parse().context("ffprobe duration")?;
    let picture = out
        .streams
        .iter()
        .find(|s| s.codec_type == "video" && s.disposition.attached_pic == 0);
    let has_audio = out.streams.iter().any(|s| s.codec_type == "audio");
    if !has_audio {
        return Err(anyhow!("the file has no audio track to transcribe"));
    }
    let video = picture.and_then(|s| {
        Some(VideoInfo {
            width: s.width?,
            height: s.height?,
            fps: parse_frame_rate(s.r_frame_rate.as_deref()),
        })
    });
    Ok(Probe {
        duration,
        kind: if picture.is_some() {
            MediaKind::Video
        } else {
            MediaKind::Audio
        },
        video,
    })
}

/// Duration only; used for synthesized overdub WAVs and rendered exports.
pub async fn duration(path: &Path) -> anyhow::Result<f64> {
    Ok(probe(path).await?.duration)
}

/// Run ffmpeg to completion, with stderr in the error on failure.
pub async fn ffmpeg(args: &[String]) -> anyhow::Result<()> {
    run("ffmpeg", args).await.map(drop)
}

/// Whisper wants 16 kHz mono PCM.
pub async fn to_whisper_wav(input: &Path, output: &Path) -> anyhow::Result<()> {
    let args = ["-y", "-loglevel", "error", "-i"]
        .map(String::from)
        .into_iter()
        .chain([input.to_string_lossy().into_owned()])
        .chain(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"].map(String::from))
        .chain([output.to_string_lossy().into_owned()])
        .collect::<Vec<_>>();
    run("ffmpeg", &args).await.map(drop)
}

pub async fn transcribe(
    whisper_bin: &str,
    model: &Path,
    wav: &Path,
    out_base: &Path,
) -> anyhow::Result<Vec<Word>> {
    if !model.exists() {
        return Err(anyhow!(
            "whisper model not found at {} (set WHISPER_MODEL)",
            model.display()
        ));
    }
    let args = whisper_args(
        &model.to_string_lossy(),
        &wav.to_string_lossy(),
        &out_base.to_string_lossy(),
    );
    run(whisper_bin, &args).await?;
    let json_path: PathBuf = out_base.with_extension("json");
    let json = tokio::fs::read_to_string(&json_path)
        .await
        .with_context(|| format!("whisper wrote no {}", json_path.display()))?;
    Ok(parse_whisper_json(&json)?)
}

/// Speaker turns in a 16 kHz mono `wav`, from sherpa-onnx's offline diarizer.
pub async fn diarize(
    config: &crate::config::Config,
    wav: &Path,
) -> anyhow::Result<Vec<SpeakerTurn>> {
    for (path, what) in [
        (&config.diarize_bin, "diarization binary"),
        (&config.diarize_segmentation, "segmentation model"),
        (&config.diarize_embedding, "speaker embedding model"),
    ] {
        if !path.is_file() {
            return Err(anyhow!(
                "speaker detection is not set up: no {what} at {} (run scripts/setup-diarization.sh)",
                path.display()
            ));
        }
    }
    let args = diarize_args(
        &wav.to_string_lossy(),
        &DiarizeOptions {
            segmentation_model: &config.diarize_segmentation.to_string_lossy(),
            embedding_model: &config.diarize_embedding.to_string_lossy(),
            cluster_threshold: config.diarize_threshold,
            num_speakers: None,
        },
    );
    let stdout = run(&config.diarize_bin.to_string_lossy(), &args).await?;
    Ok(parse_diarization(&stdout))
}

/// Silent stretches in `wav`, from ffmpeg's `silencedetect` (logged on stderr).
pub async fn silences(wav: &Path, duration: f64) -> anyhow::Result<Vec<Range>> {
    let output = Command::new("ffmpeg")
        .args(silencedetect_args(&wav.to_string_lossy()))
        .output()
        .await
        .with_context(|| start_error("ffmpeg"))?;
    check_status("ffmpeg", &output)?;
    Ok(parse_silencedetect(
        &String::from_utf8_lossy(&output.stderr),
        duration,
    ))
}

/// Run ffmpeg with `-progress pipe:1`, calling `on_progress` with the
/// fraction of `planned` output seconds written so far (0..=1).
pub async fn ffmpeg_with_progress(
    args: &[String],
    planned: f64,
    mut on_progress: impl FnMut(f64),
) -> anyhow::Result<()> {
    let mut child = Command::new("ffmpeg")
        .args(PROGRESS_ARGS)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| start_error("ffmpeg"))?;
    let stdout = child.stdout.take().context("ffmpeg stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        match parse_progress_line(&line) {
            Some(ProgressEvent::OutTime(t)) => on_progress(progress_fraction(t, planned)),
            Some(ProgressEvent::End) => on_progress(1.0),
            None => {}
        }
    }
    let output = child
        .wait_with_output()
        .await
        .context("waiting for ffmpeg")?;
    check_status("ffmpeg", &output)
}
