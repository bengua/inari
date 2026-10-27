//! Keeping a node's harnesses current. A harness may name the npm package it comes
//! from; the node then reads the installed version, asks the registry for the latest,
//! and on request installs it and runs the harness's restart command (a server such as
//! OpenFox). An ACP agent started per turn needs no restart: the next message runs the
//! new version.

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSpec {
    /// The npm package the harness comes from.
    pub npm: String,
    /// The global prefix it is installed under (default: `~/.local`).
    #[serde(default)]
    pub prefix: Option<PathBuf>,
    /// The npm program (default: `npm` from PATH).
    #[serde(default = "default_npm")]
    pub npm_bin: String,
    /// Run after a successful install, e.g. to restart a server the harness talks to.
    #[serde(default)]
    pub restart: Vec<String>,
}

fn default_npm() -> String {
    "npm".into()
}

impl UpdateSpec {
    fn prefix(&self) -> PathBuf {
        self.prefix
            .clone()
            .unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".local"))
    }

    /// The installed version, from the package's own manifest.
    pub fn current(&self) -> Option<String> {
        let manifest = self
            .prefix()
            .join("lib/node_modules")
            .join(&self.npm)
            .join("package.json");
        let v: Value = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
        v.get("version")?.as_str().map(String::from)
    }

    pub async fn latest(&self) -> Result<String, String> {
        let (code, out) = run(
            &[&self.npm_bin, "view", &self.npm, "version"],
            Duration::from_secs(60),
        )
        .await;
        let version = out
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        if code != 0 || version.is_empty() {
            return Err(tail(&out));
        }
        Ok(version)
    }

    pub async fn install(&self, version: &str) -> Result<(), String> {
        let prefix = self.prefix().display().to_string();
        let target = format!("{}@{version}", self.npm);
        let (code, out) = run(
            &[&self.npm_bin, "install", "-g", "--prefix", &prefix, &target],
            Duration::from_secs(900),
        )
        .await;
        if code != 0 {
            return Err(tail(&out));
        }
        if !self.restart.is_empty() {
            let argv: Vec<&str> = self.restart.iter().map(String::as_str).collect();
            let (code, out) = run(&argv, Duration::from_secs(120)).await;
            if code != 0 {
                return Err(format!("installed, but the restart failed: {}", tail(&out)));
            }
        }
        Ok(())
    }

    /// What the node announces for this harness.
    pub async fn status(&self) -> Value {
        let current = self.current();
        match self.latest().await {
            Ok(latest) => {
                let available = current.as_deref().is_some_and(|c| c != latest);
                json!({"current": current, "latest": latest, "available": available})
            }
            Err(e) => json!({"current": current, "latest": null, "available": false, "error": e}),
        }
    }
}

fn tail(out: &str) -> String {
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// Run a program (no shell) with the node's environment; exit code and combined output.
pub async fn run(argv: &[&str], timeout: Duration) -> (i32, String) {
    let Some((program, args)) = argv.split_first() else {
        return (-1, "empty command".into());
    };
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => return (-1, format!("cannot start {program}: {e}")),
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            (out.status.code().unwrap_or(-1), text)
        }
        Ok(Err(e)) => (-1, e.to_string()),
        Err(_) => (
            -1,
            format!("{program} did not finish within {}s", timeout.as_secs()),
        ),
    }
}
