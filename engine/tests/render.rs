//! Runs the planned ffmpeg command against `samples/sample.mp4` and checks
//! the rendered duration. Skips itself when the sample or ffmpeg is missing
//! so `cargo test` still passes on a fresh clone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use engine::{build_ffmpeg_args, Edit, ExportOptions, MediaKind, OutputFormat};

fn sample() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples/sample.mp4");
    let have_ffmpeg = Command::new("ffmpeg").arg("-version").output().is_ok();
    if path.exists() && have_ffmpeg {
        Some(path)
    } else {
        eprintln!("skipping: samples/sample.mp4 or ffmpeg not available");
        None
    }
}

fn probe_duration(path: &Path) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("duration")
}

fn probe_video_stream(path: &Path) -> bool {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout).contains("video")
}

/// A 1.5 s tone standing in for VoiceStudio output.
fn fake_overdub_wav(dir: &Path) -> PathBuf {
    let wav = dir.join("overdub.wav");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1.5",
        ])
        .arg(&wav)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    wav
}

fn run(args: &[String]) {
    let status = Command::new("ffmpeg")
        .args(args)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success(), "ffmpeg failed: ffmpeg {}", args.join(" "));
}

#[test]
fn renders_cuts_and_an_overdub_to_the_expected_length() {
    let Some(input) = sample() else { return };
    let dir = std::env::temp_dir().join("type-n-stitch-render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let source_duration = probe_duration(&input);
    assert!(
        (source_duration - 20.0).abs() < 0.5,
        "sample should be ~20 s"
    );

    let wav = fake_overdub_wav(&dir);
    let mut overdub_audio = HashMap::new();
    overdub_audio.insert("/data/test/overdub.wav".to_owned(), wav);

    // Cut 2 s, then 3 s, and replace a 1 s range with a 1.5 s overdub:
    // 20 - 2 - 3 - 1 + 1.5 = 15.5 s.
    let edits = [
        Edit::Cut {
            start: 1.0,
            end: 3.0,
            transition: None,
        },
        Edit::Overdub {
            start: 5.0,
            end: 6.0,
            text: "tone".into(),
            audio_url: "/data/test/overdub.wav".into(),
            audio_duration: 1.5,
        },
        Edit::Cut {
            start: 10.0,
            end: 13.0,
            transition: None,
        },
    ];

    let output = dir.join("out.mp4");
    let args = build_ffmpeg_args(
        &input,
        &edits,
        &ExportOptions {
            duration: source_duration,
            kind: MediaKind::Video,
            format: OutputFormat::Mp4,
            output: &output,
            overdub_audio: &overdub_audio,
        },
    )
    .unwrap();
    run(&args);

    let rendered = probe_duration(&output);
    assert!(
        (rendered - 15.5).abs() < 0.15,
        "rendered {rendered} s, expected 15.5 s"
    );
    assert!(probe_video_stream(&output));
}

#[test]
fn renders_audio_only_export_from_a_video_source() {
    let Some(input) = sample() else { return };
    let dir = std::env::temp_dir().join("type-n-stitch-render-test");
    std::fs::create_dir_all(&dir).unwrap();
    let source_duration = probe_duration(&input);

    let edits = [Edit::Cut {
        start: 0.0,
        end: 10.0,
        transition: None,
    }];
    let output = dir.join("out.mp3");
    let none = HashMap::new();
    let args = build_ffmpeg_args(
        &input,
        &edits,
        &ExportOptions {
            duration: source_duration,
            kind: MediaKind::Video,
            format: OutputFormat::Mp3,
            output: &output,
            overdub_audio: &none,
        },
    )
    .unwrap();
    run(&args);

    let rendered = probe_duration(&output);
    assert!(
        (rendered - 10.0).abs() < 0.15,
        "rendered {rendered} s, expected 10 s"
    );
    assert!(!probe_video_stream(&output));
}
