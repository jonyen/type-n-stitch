//! Environment-driven settings. Every value has a default that works on a
//! Mac with Homebrew ffmpeg + whisper-cpp and VoiceStudio on localhost.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub data_dir: PathBuf,
    pub samples_dir: PathBuf,
    pub whisper_bin: String,
    pub whisper_model: PathBuf,
    pub tts_base_url: String,
    pub tts_voice: String,
    pub max_upload_bytes: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.into());
        Self {
            port: env("PORT", "5175").parse().unwrap_or(5175),
            data_dir: PathBuf::from(env(
                "DATA_DIR",
                concat!(env!("CARGO_MANIFEST_DIR"), "/data"),
            )),
            samples_dir: PathBuf::from(env(
                "SAMPLES_DIR",
                concat!(env!("CARGO_MANIFEST_DIR"), "/../samples"),
            )),
            whisper_bin: env("WHISPER_BIN", "whisper-cli"),
            whisper_model: PathBuf::from(env(
                "WHISPER_MODEL",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../models/ggml-large-v3-turbo.bin"
                ),
            )),
            tts_base_url: env("TTS_BASE_URL", "http://localhost:3900/v1")
                .trim_end_matches('/')
                .to_owned(),
            tts_voice: env("TTS_VOICE", "513bb606"),
            max_upload_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}
