//! Shelling out to ffprobe, ffmpeg and whisper-cli.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use engine::{parse_whisper_json, whisper_args, MediaKind, Word};
use serde::Deserialize;
use tokio::process::Command;

/// Run a tool, returning stdout or an error that includes stderr.
async fn run(program: &str, args: &[String]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to start {program} (is it installed and on PATH?)"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(anyhow!(
            "{program} exited with {}: {}",
            output.status,
            tail.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub duration: f64,
    pub kind: MediaKind,
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
        "format=duration:stream=codec_type:stream_disposition=attached_pic",
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
    let has_video = out
        .streams
        .iter()
        .any(|s| s.codec_type == "video" && s.disposition.attached_pic == 0);
    let has_audio = out.streams.iter().any(|s| s.codec_type == "audio");
    if !has_audio {
        return Err(anyhow!("the file has no audio track to transcribe"));
    }
    Ok(Probe {
        duration,
        kind: if has_video {
            MediaKind::Video
        } else {
            MediaKind::Audio
        },
    })
}

/// Duration only; used for synthesized overdub WAVs and rendered exports.
pub async fn duration(path: &Path) -> anyhow::Result<f64> {
    Ok(probe(path).await?.duration)
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

pub async fn ffmpeg(args: &[String]) -> anyhow::Result<()> {
    run("ffmpeg", args).await.map(drop)
}
