//! Giving a note its meaning: a local embedding server (llama.cpp, LocalAI, text-embeddings-
//! inference… serving a 384-wide model such as multilingual-e5-small or paraphrase-multilingual-
//! MiniLM-L12-v2), reached over its OpenAI-compatible `POST /embeddings`. The width is the
//! store's (`DIM`); a model that answers another width is refused, not truncated.

use crate::net;
use crate::{Error, Result, DIM};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// How Kitsune reaches the embedding server.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EmbedConfig {
    /// The server's base address, e.g. http://127.0.0.1:8082/v1 (empty: search by words only).
    #[serde(default)]
    pub url: String,
    /// The model name the server expects.
    #[serde(default)]
    pub model: String,
    /// Put before a note's text (e5 models want "passage: ").
    #[serde(default)]
    pub passage_prefix: String,
    /// Put before a search's text (e5 models want "query: ").
    #[serde(default)]
    pub query_prefix: String,
    /// How long an embedding may take, in seconds (0: fifteen seconds).
    #[serde(default)]
    pub timeout: u64,
}

impl EmbedConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// The meaning vector of a text, `DIM` wide.
pub fn embed(cfg: &EmbedConfig, text: &str) -> Result<Vec<f32>> {
    if !cfg.ready() {
        return Err(Error::Invalid(
            "no embedding server set up: Settings → Kitsune → Meaning".into(),
        ));
    }
    let url = format!("{}/embeddings", cfg.url.trim_end_matches('/'));
    let wait = Duration::from_secs(if cfg.timeout == 0 { 15 } else { cfg.timeout });
    let r = net::post_json(&url, &json!({"model": cfg.model, "input": text}), &[], wait)?;
    if !r.ok() {
        return Err(Error::Invalid(format!(
            "the embedding server answered HTTP {}: {}",
            r.status,
            r.text().chars().take(300).collect::<String>()
        )));
    }
    let v: Value = serde_json::from_slice(&r.body)?;
    let vector: Vec<f32> = serde_json::from_value(v["data"][0]["embedding"].clone())
        .map_err(|_| Error::Invalid("the embedding server gave no vector".into()))?;
    if vector.len() != DIM {
        return Err(Error::Invalid(format!(
            "the embedding model gives {} values, the brain keeps {DIM}: pick a {DIM}-wide model",
            vector.len()
        )));
    }
    Ok(vector)
}
