//! VoiceStudio speech synthesis through its OpenAI-compatible endpoint.

use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::error::AppError;

/// Synthesize `text` to a WAV at `output`. Failures (server down, bad voice)
/// come back as a 502 with the upstream message so the UI can show it.
pub async fn synthesize(
    http: &reqwest::Client,
    base_url: &str,
    voice: &str,
    text: &str,
    output: &Path,
) -> Result<(), AppError> {
    let url = format!("{base_url}/audio/speech");
    let response = http
        .post(&url)
        .json(&json!({
            "model": "tts-1",
            "input": text,
            "voice": voice,
            "response_format": "wav",
        }))
        .send()
        .await
        .map_err(|e| {
            AppError::upstream(format!(
                "VoiceStudio is not reachable at {url} ({e}). Start it, or set TTS_BASE_URL."
            ))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::upstream(format!(
            "VoiceStudio returned {status}: {}",
            body.chars().take(300).collect::<String>()
        )));
    }

    let bytes = response.bytes().await.context("reading TTS audio")?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" {
        return Err(AppError::upstream("VoiceStudio did not return a WAV file"));
    }
    tokio::fs::write(output, &bytes)
        .await
        .context("writing overdub WAV")?;
    Ok(())
}
