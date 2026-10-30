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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// A capture whose mean segment `avg_logprob` is below this is refused as inaudible.
    #[serde(default = "min_avg_logprob")]
    pub min_avg_logprob: f64,
    /// A capture whose mean `no_speech_prob` is above this is refused as inaudible.
    #[serde(default = "max_no_speech_prob")]
    pub max_no_speech_prob: f64,
}

fn min_avg_logprob() -> f64 {
    -1.0
}

fn max_no_speech_prob() -> f64 {
    0.6
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            model: String::new(),
            language: String::new(),
            timeout: 0,
            min_avg_logprob: min_avg_logprob(),
            max_no_speech_prob: max_no_speech_prob(),
        }
    }
}

/// What the server heard, and how sure it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub text: String,
    /// The language the server detected (or was told), when it says.
    pub language: String,
    /// The mean of the segments' `avg_logprob`, when the server gives segments.
    pub avg_logprob: Option<f64>,
    /// The mean of the segments' `no_speech_prob`, when the server gives segments.
    pub no_speech_prob: Option<f64>,
}

#[derive(Deserialize)]
struct Verbose {
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    segments: Vec<Segment>,
}

#[derive(Deserialize)]
struct Segment {
    avg_logprob: Option<f64>,
    no_speech_prob: Option<f64>,
}

fn mean(xs: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = xs.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| sum / n as f64)
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

/// Transcribe a WAV/PCM capture (asking for `verbose_json`, so the server says how sure it is).
/// A capture heard too poorly is refused with [`Error::Inaudible`]; a server that cannot be
/// reached or answers wrong is an ordinary error, and the caller keeps the audio for later.
pub fn transcribe(cfg: &AsrConfig, wav: &[u8]) -> Result<Transcript> {
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
        Part::Field("response_format", "verbose_json"),
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
    // verbose_json is {"text", "language", "segments": [{avg_logprob, no_speech_prob, …}]};
    // a server without it answers {"text"} or plain text, and is then taken at its word
    let body = r.text();
    let heard = match serde_json::from_str::<Value>(&body) {
        Ok(v) => {
            let v: Verbose = serde_json::from_value(v)?;
            Transcript {
                text: v.text.trim().to_string(),
                language: v.language,
                avg_logprob: mean(v.segments.iter().filter_map(|s| s.avg_logprob)),
                no_speech_prob: mean(v.segments.iter().filter_map(|s| s.no_speech_prob)),
            }
        }
        Err(_) => Transcript {
            text: body.trim().to_string(),
            language: String::new(),
            avg_logprob: None,
            no_speech_prob: None,
        },
    };
    if let Some(lp) = heard.avg_logprob.filter(|lp| *lp < cfg.min_avg_logprob) {
        return Err(Error::Inaudible(format!(
            "I could not hear that clearly (confidence {lp:.2}); nothing was kept, say it again"
        )));
    }
    if let Some(ns) = heard
        .no_speech_prob
        .filter(|ns| *ns > cfg.max_no_speech_prob)
    {
        return Err(Error::Inaudible(format!(
            "that sounded like no speech ({:.0} %); nothing was kept, say it again",
            ns * 100.0
        )));
    }
    if heard.text.is_empty() {
        return Err(Error::Inaudible(
            "nothing was said in that capture; nothing was kept".into(),
        ));
    }
    Ok(heard)
}
