//! Runs the planned ffmpeg command against `samples/sample.mp4` and checks
//! the rendered duration. Skips itself when the sample or ffmpeg is missing
//! so `cargo test` still passes on a fresh clone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use engine::text;
use engine::{
    build_ffmpeg_args, oriented, CaptionPos, Edit, ExportOptions, MediaKind, OutputFormat,
    TitleStyle, Transition, VideoInfo,
};

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
            video: None,
            title_images: &HashMap::new(),
            caption_images: &HashMap::new(),
            transition: Transition::None,
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
            video: None,
            title_images: &HashMap::new(),
            caption_images: &HashMap::new(),
            transition: Transition::None,
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

/// Is ffmpeg on the PATH at all?
fn have_ffmpeg() -> bool {
    let ok = Command::new("which")
        .arg("ffmpeg")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: ffmpeg not on PATH");
    }
    ok
}

/// A 3 s colour-bar clip with a tone, standing in for a real recording.
fn testsrc_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("clip.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=30:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
            "-shortest",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .status()
        .expect("ffmpeg runs");
    assert!(status.success());
    clip
}

#[test]
fn renders_a_title_card_a_caption_and_a_dip_from_rasterised_pngs() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let clip = testsrc_clip(dir.path());
    let video = VideoInfo {
        width: 320,
        height: 240,
        fps: 30.0,
    };

    let edits = [
        Edit::Title {
            at: 1.0,
            duration: 2.0,
            text: "Chapter one".into(),
            subtitle: Some("a subtitle".into()),
            style: TitleStyle::Accent,
        },
        Edit::Caption {
            start: 0.5,
            end: 2.5,
            text: "Ada Lovelace".into(),
            position: CaptionPos::BottomLeft,
        },
        Edit::Cut {
            start: 2.0,
            end: 2.5,
            transition: None,
        },
    ];

    // The engine draws the text; ffmpeg only composites the PNGs.
    let card = dir.path().join("title-0.png");
    std::fs::write(
        &card,
        text::render_title("Chapter one", Some("a subtitle"), TitleStyle::Accent, video).to_png(),
    )
    .unwrap();
    let mut title_images = HashMap::new();
    title_images.insert(0, card);

    let boxed = text::render_caption("Ada Lovelace", CaptionPos::BottomLeft, video);
    let cap = dir.path().join("caption-1.png");
    std::fs::write(&cap, boxed.raster.to_png()).unwrap();
    let mut caption_images = HashMap::new();
    caption_images.insert(1, (cap, boxed.x, boxed.y));

    let output = dir.path().join("out.mp4");
    let none = HashMap::new();
    let args = build_ffmpeg_args(
        &clip,
        &edits,
        &ExportOptions {
            duration: 3.0,
            kind: MediaKind::Video,
            format: OutputFormat::Mp4,
            output: &output,
            overdub_audio: &none,
            video: Some(video),
            title_images: &title_images,
            caption_images: &caption_images,
            transition: Transition::Dip,
        },
    )
    .unwrap();
    run(&args);

    // 3 s, less the 0.5 s cut, plus the 2 s title card.
    let rendered = probe_duration(&output);
    assert!(
        (rendered - 4.5).abs() < 0.2,
        "rendered {rendered} s, expected 4.5 s"
    );
    assert!(probe_video_stream(&output));
}

/// The picture's coded size, as ffprobe reports it (before autorotation).
fn probe_size(path: &Path) -> (u32, u32) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()
        .expect("ffprobe runs");
    // csv prints a trailing separator, so read the first two fields.
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fields = text.trim().split('x').filter(|f| !f.is_empty());
    let w = fields.next().expect("width").parse().expect("width");
    let h = fields.next().expect("height").parse().expect("height");
    (w, h)
}

/// Copy `clip` with a 90° display matrix, so ffmpeg autorotates it on decode.
fn rotated_clip(dir: &Path, clip: &Path, out_name: &str) -> PathBuf {
    let rotated = dir.join(out_name);
    // Newer builds take `-display_rotation` on the input; older ones only
    // understand the `rotate` stream metadata.
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-display_rotation", "90", "-i"])
        .arg(clip)
        .args(["-c", "copy"])
        .arg(&rotated)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(clip)
            .args(["-c", "copy", "-metadata:s:v", "rotate=90"])
            .arg(&rotated)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success(), "could not make a rotated source");
    }
    rotated
}

/// Plan a title-card render of `clip` with the card rasterised at `video`,
/// and return whether ffmpeg accepted it.
fn render_title_at(clip: &Path, dir: &Path, video: VideoInfo, tag: &str) -> (bool, PathBuf) {
    let edits = [Edit::Title {
        at: 1.0,
        duration: 1.0,
        text: "Rotated".into(),
        subtitle: None,
        style: TitleStyle::Dark,
    }];
    let card = dir.join(format!("title-{tag}.png"));
    std::fs::write(
        &card,
        text::render_title("Rotated", None, TitleStyle::Dark, video).to_png(),
    )
    .unwrap();
    let mut title_images = HashMap::new();
    title_images.insert(0, card);

    let output = dir.join(format!("out-{tag}.mp4"));
    let none = HashMap::new();
    let args = build_ffmpeg_args(
        clip,
        &edits,
        &ExportOptions {
            duration: 3.0,
            kind: MediaKind::Video,
            format: OutputFormat::Mp4,
            output: &output,
            overdub_audio: &none,
            video: Some(video),
            title_images: &title_images,
            caption_images: &HashMap::new(),
            transition: Transition::None,
        },
    )
    .unwrap();
    let ok = Command::new("ffmpeg")
        .args(&args)
        .output()
        .expect("ffmpeg runs")
        .status
        .success();
    (ok, output)
}

/// A 90°-rotated source decodes transposed, so the title card has to be
/// rasterised at the *oriented* size — which is what the server computes from
/// the probed size plus the display matrix — or `concat` rejects it.
#[test]
fn a_rotated_source_needs_its_title_card_at_the_oriented_size() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let clip = testsrc_clip(dir.path());
    let rotated = rotated_clip(dir.path(), &clip, "rot.mp4");

    // What the server's probe sees: the coded size, 320x240 here.
    let (w, h) = probe_size(&rotated);
    let probed = VideoInfo {
        width: w,
        height: h,
        fps: 30.0,
    };
    let card_size = oriented(probed, 90);
    assert_eq!(
        (card_size.width, card_size.height),
        (probed.height, probed.width),
        "a quarter turn swaps the frame size"
    );

    let (ok, output) = render_title_at(&rotated, dir.path(), card_size, "oriented");
    assert!(ok, "ffmpeg rejected a card at the oriented size");
    assert_eq!(
        probe_size(&output),
        (card_size.width, card_size.height),
        "the render keeps the rotated frame size"
    );

    // The negative control: at the probed (unswapped) size, concat refuses.
    let (ok, _) = render_title_at(&rotated, dir.path(), probed, "coded");
    assert!(!ok, "a card at the unswapped size should have failed");
}
