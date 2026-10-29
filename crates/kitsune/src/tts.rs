//! Speaking a note aloud with a local text-to-speech server (Qwen3-TTS, CSM, Kokoro… served over
//! an OpenAI-compatible `POST /audio/speech`): the core sends the text and the chosen voice, gets
//! the audio back. Which server, which voice, which model and which format are configuration, set
//! in the GUI, never hard-coded; the server runs on the machine or the private network, so the
//! voice never leaves the house. The engine's fidelity and the blind listening test are the
//! person's to settle where the model runs — this is only the wire to it.

use crate::net;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

/// How Kitsune reaches the voice server.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TtsConfig {
    /// The server's base address, e.g. http://127.0.0.1:8880/v1 (empty: no voice set up).
    #[serde(default)]
    pub url: String,
    /// The model the server expects (tts-1, kokoro, qwen3-tts…); empty when it ignores it.
    #[serde(default)]
    pub model: String,
    /// The voice to speak in (a named voice, or a cloned one the server knows).
    #[serde(default)]
    pub voice: String,
    /// The audio format asked of the server: wav (default), mp3, opus, flac…
    #[serde(default)]
    pub format: String,
    /// How long a synthesis may take, in seconds (0: three minutes).
    #[serde(default)]
    pub timeout: u64,
}

impl TtsConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty()
    }

    fn endpoint(&self) -> String {
        format!("{}/audio/speech", self.url.trim_end_matches('/'))
    }

    fn fmt(&self) -> String {
        Some(self.format.trim())
            .filter(|f| !f.is_empty())
            .unwrap_or("wav")
            .to_string()
    }

    fn wait(&self) -> Duration {
        Duration::from_secs(if self.timeout == 0 { 180 } else { self.timeout })
    }
}

/// The audio and its media type for a format.
pub fn mime_of(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "pcm" => "audio/pcm",
        _ => "audio/wav",
    }
}

/// Speak `text`; the audio bytes and their format (the extension to keep).
pub fn synthesize(cfg: &TtsConfig, text: &str) -> Result<(Vec<u8>, String)> {
    if !cfg.ready() {
        return Err(Error::Invalid(
            "no voice server set up: Settings → Kitsune → Voice".into(),
        ));
    }
    if text.trim().is_empty() {
        return Err(Error::Invalid("nothing to say".into()));
    }
    let format = cfg.fmt();
    let mut body = json!({"input": text, "response_format": format});
    if !cfg.model.trim().is_empty() {
        body["model"] = json!(cfg.model.trim());
    }
    if !cfg.voice.trim().is_empty() {
        body["voice"] = json!(cfg.voice.trim());
    }
    let r = net::post_json(&cfg.endpoint(), &body, &[], cfg.wait())?;
    if !r.ok() {
        return Err(Error::Invalid(format!(
            "the voice server answered HTTP {}: {}",
            r.status,
            r.text().chars().take(300).collect::<String>()
        )));
    }
    if r.body.is_empty() {
        return Err(Error::Invalid("the voice server returned no audio".into()));
    }
    Ok((r.body, format))
}
