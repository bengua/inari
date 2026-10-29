//! Inari's local engine: the OpenFox core inside the core. A thread's turn is a loop of the
//! model (an OpenAI-compatible server on the machine or the private network: vLLM, Ollama,
//! llama.cpp…) and its tools (read, write, edit a file, run a command), in the thread's
//! folder, journaled as any harness's. No other process than inari-core-rs and the model
//! server. Its settings (providers, model, agent, limits) are the interface's, kept in
//! `<data_dir>/engine/<harness>/`; a provider's key is kept apart (`keys.json`, mode 600) and
//! never shown.

pub mod defs;
pub mod forge;
mod library;
pub mod llm;
mod mcp;
mod panel;
mod project;
pub mod prompts;
mod run;
pub mod state;
pub mod tools;
mod workflow;

use crate::agent::Probe;
use crate::capsule::io;
use crate::openfox::Panel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A model server the engine may use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Its OpenAI-compatible base address (http://127.0.0.1:8000/v1).
    pub url: String,
}

/// The engine's settings, set from the interface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// The model a thread uses when it chose none: "<provider>/<model>".
    #[serde(default)]
    pub model: String,
    /// The agent a thread works as when it chose none (builder, planner).
    #[serde(default = "builder")]
    pub mode: String,
    /// The model's context, in tokens: past 80 % the conversation is summarised.
    #[serde(default = "window")]
    pub context_window: u64,
    /// How many model calls a turn may make.
    #[serde(default = "steps")]
    pub max_steps: u32,
    /// How long a turn may run, in seconds.
    #[serde(default = "limit")]
    pub turn_timeout: u64,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Instructions added to every thread's system prompt.
    #[serde(default)]
    pub instructions: String,
    /// The skills the agents are not offered.
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// A thread's folder stays on disk between its turns (its dev server and processes go on).
    #[serde(default = "yes")]
    pub keep_folder: bool,
}

fn yes() -> bool {
    true
}

fn builder() -> String {
    "builder".into()
}
fn window() -> u64 {
    32_768
}
fn steps() -> u32 {
    100
}
fn limit() -> u64 {
    3600
}

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_value(json!({})).expect("defaults")
    }
}

fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 64
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

impl Settings {
    pub fn problem(&self) -> Option<String> {
        for p in &self.providers {
            if !id_like(&p.id) {
                return Some(format!(
                    "a provider's id is letters, digits, - _ and . only: {:?}",
                    p.id
                ));
            }
            if !p.url.starts_with("http://") {
                return Some(format!(
                    "{}: the local engine speaks plain http (http://host:port/v1)",
                    p.name
                ));
            }
        }
        if !id_like(&self.mode) {
            return Some(format!("unknown agent {:?}", self.mode));
        }
        if !(2048..=2_000_000).contains(&self.context_window) {
            return Some("a context window is between 2,048 and 2,000,000 tokens".into());
        }
        if !(1..=1000).contains(&self.max_steps) {
            return Some("a turn makes between 1 and 1,000 model calls".into());
        }
        if !(30..=86_400).contains(&self.turn_timeout) {
            return Some("a turn's limit is between 30 s and 24 h".into());
        }
        if self.temperature.is_some_and(|t| !(0.0..=2.0).contains(&t)) {
            return Some("a temperature is between 0 and 2".into());
        }
        None
    }
}

pub struct LocalEngine {
    dir: PathBuf,
    settings: Mutex<Settings>,
    /// Run before a turn whose thread chose a model (as an ACP harness's `prepare`).
    pub prepare: Vec<String>,
    pub prepare_timeout: Duration,
    pub extra_env: BTreeMap<String, String>,
    /// How long the model may stay silent in the middle of an answer.
    pub quiet: Duration,
    /// What each thread shows beside its conversation (the same panel as OpenFox's).
    panels: Mutex<HashMap<String, Panel>>,
    /// When each thread's panel last went to the pages.
    shown: Mutex<HashMap<String, Instant>>,
    /// The questions waiting for the person's answer, by call.
    questions: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Option<String>>>>,
    /// Messages sent while a thread works: the model reads them at its next step.
    queues: Mutex<HashMap<String, Vec<String>>>,
    /// The threads working now.
    active: Mutex<HashSet<String>>,
    /// The threads whose next turn starts by summarising the conversation.
    compact_next: Mutex<HashSet<String>>,
    /// The MCP servers' links and tools.
    mcp: mcp::Hub,
    /// The processes the threads' projects run on this core.
    works: project::Works,
}

impl LocalEngine {
    pub fn new(dir: PathBuf) -> Self {
        let settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            dir,
            settings: Mutex::new(settings),
            prepare: Vec::new(),
            prepare_timeout: Duration::from_secs(600),
            extra_env: BTreeMap::new(),
            quiet: Duration::from_secs(300),
            panels: Mutex::default(),
            shown: Mutex::default(),
            questions: Mutex::default(),
            queues: Mutex::default(),
            active: Mutex::default(),
            compact_next: Mutex::default(),
            mcp: mcp::Hub::default(),
            works: project::Works::default(),
        }
    }

    /// The engine's own folder (its settings, agents, skills, commands, workflows).
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("not poisoned").clone()
    }

    pub fn set_settings(&self, s: Settings) -> Result<(), String> {
        if let Some(why) = s.problem() {
            return Err(why);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        io::atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&s).expect("serializes"),
        )
        .map_err(|e| e.to_string())?;
        *self.settings.lock().expect("not poisoned") = s;
        Ok(())
    }

    fn keys(&self) -> HashMap<String, String> {
        std::fs::read(self.dir.join("keys.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Keep a provider's key (empty: none), readable by the core's user only.
    pub fn set_key(&self, provider: &str, key: &str) -> Result<(), String> {
        let mut keys = self.keys();
        if key.is_empty() {
            keys.remove(provider);
        } else {
            keys.insert(provider.into(), key.into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join("keys.json");
        io::atomic_write(&path, &serde_json::to_vec(&keys).expect("serializes"))
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn server(&self, provider: &Provider) -> llm::Server {
        llm::Server {
            url: provider.url.trim_end_matches('/').to_string(),
            key: self.keys().get(&provider.id).cloned().unwrap_or_default(),
        }
    }

    /// The models of every provider, as "<provider>/<model>" (a provider that does not answer
    /// gives none).
    pub async fn models(&self) -> Vec<(String, Result<Vec<String>, String>)> {
        let mut out = Vec::new();
        for p in self.settings().providers {
            let got = llm::models(&self.server(&p)).await;
            out.push((p.id.clone(), got));
        }
        out
    }

    /// Try a server before it is kept: its models, or why it does not answer.
    pub async fn test(url: &str, key: &str) -> Result<Vec<String>, String> {
        llm::models(&llm::Server {
            url: url.trim().trim_end_matches('/').to_string(),
            key: key.into(),
        })
        .await
    }

    /// How the engine is, for its sheet: its settings, its providers (never their keys) with
    /// their models.
    pub async fn status(&self) -> Value {
        let s = self.settings();
        let keys = self.keys();
        let mut providers = Vec::new();
        for (id, got) in self.models().await {
            let p = s.providers.iter().find(|p| p.id == id).cloned();
            let Some(p) = p else { continue };
            providers.push(match got {
                Ok(models) => json!({"id": p.id, "name": p.name, "url": p.url, "key": keys.contains_key(&p.id),
                                     "reachable": true, "models": models}),
                Err(e) => json!({"id": p.id, "name": p.name, "url": p.url, "key": keys.contains_key(&p.id),
                                 "reachable": false, "problem": e, "models": []}),
            });
        }
        json!({"providers": providers, "model": s.model, "mode": s.mode, "context_window": s.context_window,
               "max_steps": s.max_steps, "turn_timeout": s.turn_timeout, "temperature": s.temperature,
               "instructions": s.instructions, "modes": self.modes(None), "keep_folder": s.keep_folder,
               "forges": self.forges_view()})
    }

    pub async fn probe(&self) -> Probe {
        let mut models = Vec::new();
        for (id, got) in self.models().await {
            for m in got.unwrap_or_default() {
                models.push(format!("{id}/{m}"));
            }
        }
        Probe {
            models,
            modes: self.modes(None),
            commands: defs::commands(&self.dir, None)
                .into_iter()
                .map(|c| json!({"name": c.id, "description": c.name}))
                .collect(),
        }
    }

    /// The agents a thread may work as (not the sub-agents), as modes.
    pub fn modes(&self, workdir: Option<&std::path::Path>) -> Vec<Value> {
        defs::agents(&self.dir, workdir)
            .into_iter()
            .filter(|a| !a.subagent)
            .map(|a| json!({"id": a.id, "name": a.name, "description": a.description}))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_checked() {
        let ok = Settings::default();
        assert_eq!(ok.problem(), None);
        assert_eq!((ok.mode.as_str(), ok.context_window), ("builder", 32_768));
        let bad = Settings {
            providers: vec![Provider {
                id: "v".into(),
                name: "vLLM".into(),
                url: "https://x/v1".into(),
            }],
            ..ok.clone()
        };
        assert!(bad.problem().is_some());
        assert!(Settings {
            mode: "../x".into(),
            ..ok
        }
        .problem()
        .is_some());
    }
}
