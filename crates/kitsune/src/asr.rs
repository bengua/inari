//! Turning a voice capture into text with a local speech-to-text server (whisper.cpp,
//! faster-whisper, LocalAI…), reached over its OpenAI-compatible `POST /audio/transcriptions`
//! multipart endpoint. Which server, which model and which language are configuration, set in
//! the GUI, never hard-coded; the server runs on the machine or the private network, so nothing
//! leaves the house.

use crate::net::{self, Part};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// How Kitsune reaches the transcription server.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AsrConfig {
    /// The server's base address, e.g. http://127.0.0.1:8080/v1 (empty: no transcription set up).
    #[serde(default)]
    pub url: String,
    /// The model name the server expects (whisper-1, large-v3…); empty when the server ignores it.
    #[serde(default)]
    pub model: String,
    /// The spoken language as a hint (fr, en…); empty lets the server detect it.
    #[serde(default)]
    pub language: String,
    /// How long a transcription may take, in seconds (0: three minutes).
    #[serde(default)]
    pub timeout: u64,
}

impl AsrConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty()
    }

    fn endpoint(&self) -> String {
        format!("{}/audio/transcriptions", self.url.trim_end_matches('/'))
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(if self.timeout == 0 { 180 } else { self.timeout })
    }
}

/// Transcribe a WAV/PCM capture; the recognised text (trimmed), or why it failed.
pub fn transcribe(cfg: &AsrConfig, wav: &[u8]) -> Result<String> {
    if !cfg.ready() {
        return Err(Error::Invalid(
            "no transcription server set up: Settings → Kitsune → Voice-to-text".into(),
        ));
    }
    let mut parts = vec![
        Part::File {
            field: "file",
            filename: "capture.wav",
            mime: "audio/wav",
            bytes: wav,
        },
        Part::Field("response_format", "json"),
    ];
    if !cfg.model.trim().is_empty() {
        parts.push(Part::Field("model", cfg.model.trim()));
    }
    if !cfg.language.trim().is_empty() {
        parts.push(Part::Field("language", cfg.language.trim()));
    }
    let r = net::post_multipart(&cfg.endpoint(), &parts, &[], cfg.wait())?;
    if !r.ok() {
        return Err(Error::Invalid(format!(
            "the transcription server answered HTTP {}: {}",
            r.status,
            r.text().chars().take(300).collect::<String>()
        )));
    }
    // the OpenAI shape is {"text": "..."}; some servers answer plain text
    let body = r.text();
    let text = match serde_json::from_str::<Value>(&body) {
        Ok(v) => v
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Err(_) => body,
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(Error::Invalid("the transcription came back empty".into()));
    }
    Ok(text)
}
