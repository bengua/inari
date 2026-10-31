//! AI-Memory (akitaonrails/ai-memory, MIT): the agents' shared long-term memory, a service of its
//! own that the core starts, checks and wires to the agents, never linked into the core. Its
//! binary, its arguments, its address, its embedding provider are settings (`ai-memory.json`), its
//! token a secret apart (`ai-memory.token`, mode 600), never shown again.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// The agents it knows how to wire (`install-mcp --client`, `install-hooks --agent`).
pub const CLIENTS: [&str; 8] = [
    "claude-code",
    "codex",
    "gemini-cli",
    "opencode",
    "qwen-code",
    "cline",
    "copilot-cli",
    "kiro-cli",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Started with the core, and again when it stops.
    pub enabled: bool,
    /// Its command (a name on the PATH, or a path).
    pub binary: String,
    /// What it is started with.
    pub args: String,
    /// Where it answers (loopback by default, as AI-Memory itself).
    pub url: String,
    /// Its own data folder (empty: `<data_dir>/ai-memory`).
    pub data_dir: String,
    /// AI_MEMORY_EMBEDDING_PROVIDER (empty: its default).
    pub embedding_provider: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: "ai-memory".into(),
            args: "serve --transport http".into(),
            url: "http://127.0.0.1:49374".into(),
            data_dir: String::new(),
            embedding_provider: String::new(),
        }
    }
}

impl Config {
    pub fn problem(&self) -> Option<String> {
        let bin_ok = !self.binary.trim().is_empty()
            && self
                .binary
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c));
        if !bin_ok {
            return Some("its command is a name or a path (letters, digits, / . _ -)".into());
        }
        if !self.url.starts_with("http://") {
            return Some("AI-Memory answers on http://127.0.0.1:port (loopback)".into());
        }
        None
    }
}

pub struct AiMemory {
    dir: PathBuf,
    child: Mutex<Option<std::process::Child>>,
}

impl AiMemory {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.to_path_buf(),
            child: Mutex::new(None),
        }
    }

    pub fn config(&self) -> Config {
        std::fs::read(self.dir.join("ai-memory.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn set_config(&self, c: &Config) -> Result<(), String> {
        if let Some(p) = c.problem() {
            return Err(p);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(
            self.dir.join("ai-memory.json"),
            serde_json::to_vec_pretty(c).expect("serializes"),
        )
        .map_err(|e| e.to_string())
    }

    /// Its token: made once, kept mode 600, handed to the service and to the check, never shown.
    fn token(&self) -> Result<String, String> {
        let path = self.dir.join("ai-memory.token");
        if let Ok(t) = std::fs::read_to_string(&path) {
            if !t.trim().is_empty() {
                return Ok(t.trim().to_string());
            }
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let t = crate::wire::random_hex(24);
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        std::io::Write::write_all(&mut o.open(&path).map_err(|e| e.to_string())?, t.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(t)
    }

    fn data_dir(&self, c: &Config) -> PathBuf {
        if c.data_dir.trim().is_empty() {
            self.dir.join("ai-memory")
        } else {
            PathBuf::from(c.data_dir.trim())
        }
    }

    fn running(&self) -> Option<u32> {
        let mut child = self.child.lock().expect("not poisoned");
        match child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => child.as_ref().map(|c| c.id()),
            _ => {
                *child = None;
                None
            }
        }
    }

    /// Start it (again): its output goes to `ai-memory.log` in the core's folder.
    pub fn start(&self) -> Result<u32, String> {
        self.stop();
        let c = self.config();
        if let Some(p) = c.problem() {
            return Err(p);
        }
        let data = self.data_dir(&c);
        std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
        let log =
            std::fs::File::create(self.dir.join("ai-memory.log")).map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(c.binary.trim());
        cmd.args(c.args.split_whitespace())
            .current_dir(&data)
            .env("AI_MEMORY_AUTH_TOKEN", self.token()?)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        if !c.embedding_provider.trim().is_empty() {
            cmd.env("AI_MEMORY_EMBEDDING_PROVIDER", c.embedding_provider.trim());
        }
        let child = cmd.spawn().map_err(|e| {
            format!(
                "{} does not start ({e}): is AI-Memory installed on this core?",
                c.binary
            )
        })?;
        let pid = child.id();
        *self.child.lock().expect("not poisoned") = Some(child);
        Ok(pid)
    }

    pub fn stop(&self) {
        if let Some(mut c) = self.child.lock().expect("not poisoned").take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Does it answer? Its JSON API, with its token.
    pub async fn check(&self) -> Result<String, String> {
        let c = self.config();
        let token = self.token()?;
        let auth = format!("Bearer {token}");
        let (status, _) = crate::usage::http_with(
            "GET",
            &format!("{}/api/v1", c.url.trim_end_matches('/')),
            None,
            &[("Authorization", &auth)],
            Duration::from_secs(5),
        )
        .await
        .map_err(|e| format!("{} does not answer: {e}", c.url))?;
        if (200..400).contains(&status) {
            Ok(format!("It answers at {}.", c.url))
        } else {
            Err(format!("{} answered HTTP {status}", c.url))
        }
    }

    /// Wire an agent to the shared memory, as AI-Memory does it: its MCP server, then its hooks.
    pub async fn wire(&self, client: &str) -> Result<String, String> {
        if !CLIENTS.contains(&client) {
            return Err(format!(
                "unknown agent {client:?}; there are: {}",
                CLIENTS.join(", ")
            ));
        }
        let c = self.config();
        let mut said = Vec::new();
        for args in [
            ["install-mcp", "--client", client, "--apply"],
            ["install-hooks", "--agent", client, "--apply"],
        ] {
            let out = tokio::time::timeout(
                Duration::from_secs(60),
                tokio::process::Command::new(c.binary.trim())
                    .args(args)
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .map_err(|_| format!("{} {} took too long", c.binary, args[0]))?
            .map_err(|e| format!("{} does not start ({e})", c.binary))?;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !out.status.success() {
                return Err(format!(
                    "{} {}: {}",
                    c.binary,
                    args[0],
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            said.push(format!(
                "{}: {}",
                args[0],
                if text.is_empty() { "done" } else { &text }
            ));
        }
        Ok(said.join("\n"))
    }

    /// How it is, for its sheet (never the token).
    pub fn status(&self) -> Value {
        let log = std::fs::read_to_string(self.dir.join("ai-memory.log")).unwrap_or_default();
        let tail: String = log
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        json!({"config": self.config(), "running": self.running().is_some(), "pid": self.running(),
               "clients": CLIENTS, "log": tail})
    }
}

impl Drop for AiMemory {
    fn drop(&mut self) {
        self.stop();
    }
}
