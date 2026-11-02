//! The policy engine: three levels — the server (node configuration), the agent (its
//! harness spec), the session (the thread's labels) — and the strictest verdict wins.
//!
//! A permission request becomes a [`Verdict`]: `Allow` acts, `Ask` goes to the owner's
//! one-gesture signed approval (see `approvals.rs`), `Deny` refuses outright. Budgets
//! (tokens, GPU-seconds for local models) have a soft threshold (`Ask`) and a hard cap
//! (`Deny`). The sudo gate never auto-approves: the exact command reaches the owner,
//! and only the owner's signed verdict lets it through — the model's own judgement is
//! never part of that decision.
//!
//! Every decision is journaled as a record matching `schema/policy-decision.schema.json`.

use serde_json::{json, Map, Value};

/// A command's risk class, from the command alone — never from the model's say-so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Read-only, no shell trickery: `ls`, `cat`, `git status`…
    Benign,
    /// Anything else that is not sudo.
    Regular,
    /// Privilege escalation: the exact command must reach the owner.
    Sudo,
}

/// Read-only first tokens; any shell metacharacter demotes to `Regular` anyway.
const BENIGN: [&str; 24] = [
    "ls", "cat", "echo", "pwd", "date", "sleep", "head", "tail", "grep", "find", "wc", "file",
    "stat", "readlink", "realpath", "which", "whoami", "hostname", "uname", "df", "du", "env",
    "printenv", "true",
];

const SHELL_META: [char; 9] = [';', '&', '|', '>', '<', '`', '$', '(', '\n'];

/// The command part of an ACP tool title (`"bash: rm -rf x"` → `rm -rf x`): the prefix
/// before the first `": "` is a tool kind when it is short and alphanumeric.
fn command_of(title: &str) -> &str {
    if let Some((prefix, rest)) = title.split_once(": ") {
        if prefix.len() <= 12
            && !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return rest.trim();
        }
    }
    title.trim()
}

/// The bare executable name of the first token (`/usr/bin/sudo` → `sudo`).
fn first_token(command: &str) -> &str {
    let first = command.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first)
}

pub fn classify(title: &str) -> Class {
    let command = command_of(title);
    let first = first_token(command);
    if first == "sudo" || first == "doas" {
        return Class::Sudo;
    }
    if first == "git" {
        // git is benign only for its read-only subcommands
        let sub = command.split_whitespace().nth(1).unwrap_or("");
        if ![
            "status", "log", "diff", "show", "branch", "blame", "ls-files",
        ]
        .contains(&sub)
        {
            return Class::Regular;
        }
    } else if !BENIGN.contains(&first) {
        return Class::Regular;
    }
    if command.chars().any(|c| SHELL_META.contains(&c)) {
        return Class::Regular;
    }
    Class::Benign
}

/// One level's limits. `None`/absent means the level constrains nothing; when every
/// level is at its default the engine reproduces the pre-policy behavior exactly
/// (auto approves, everything else asks, nothing is denied).
#[derive(Debug, Clone, Default)]
pub struct Level {
    /// This level permits auto-approval (a session labelled `auto=1` acts without
    /// asking). Auto needs every level's consent.
    pub auto: Option<bool>,
    /// Read-only commands pass without asking.
    pub allow_benign: Option<bool>,
    /// Soft token budget: past it, every action asks.
    pub tokens_ask: Option<u64>,
    /// Hard token budget: past it, every action is denied.
    pub tokens_cap: Option<u64>,
    /// Soft GPU budget in seconds (local models).
    pub gpu_ask: Option<f64>,
    /// Hard GPU budget in seconds.
    pub gpu_cap: Option<f64>,
}

impl Level {
    /// A level from a config table (`{"auto": false, "tokens_cap": 200000, …}`).
    pub fn from_config(v: &Value) -> Self {
        let b = |k: &str| v.get(k).and_then(Value::as_bool);
        let u = |k: &str| v.get(k).and_then(Value::as_u64);
        let f = |k: &str| v.get(k).and_then(Value::as_f64);
        Self {
            auto: b("auto"),
            allow_benign: b("allow_benign"),
            tokens_ask: u("tokens_ask"),
            tokens_cap: u("tokens_cap"),
            gpu_ask: f("gpu_ask"),
            gpu_cap: f("gpu_cap"),
        }
    }

    /// The session level from a thread's labels: `policy.auto=0`, `policy.tokens_cap=…`.
    pub fn from_labels(labels: &Map<String, Value>) -> Self {
        let get = |k: &str| labels.get(&format!("policy.{k}")).and_then(Value::as_str);
        Self {
            auto: get("auto").map(|v| v == "1" || v == "true"),
            allow_benign: get("allow_benign").map(|v| v == "1" || v == "true"),
            tokens_ask: get("tokens_ask").and_then(|v| v.parse().ok()),
            tokens_cap: get("tokens_cap").and_then(|v| v.parse().ok()),
            gpu_ask: get("gpu_ask").and_then(|v| v.parse().ok()),
            gpu_cap: get("gpu_cap").and_then(|v| v.parse().ok()),
        }
    }
}

/// What a session already spent, fed by the node (see `usage.rs`): tokens are an
/// upper-bound estimate (a re-read-context backend reprocesses its context each turn),
/// GPU-seconds are the measured turn time of threads on local models.
#[derive(Debug, Clone, Copy, Default)]
pub struct Spend {
    pub tokens: u64,
    pub gpu_seconds: f64,
}

/// What a permission request is decided into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Ask(String),
    Deny(String),
}

/// The three levels, strictest-first.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub server: Level,
    pub agent: Level,
    pub session: Level,
}

/// What the engine needs to decide one permission request.
pub struct Request<'a> {
    pub command: &'a str,
    /// The session is labelled `auto=1` (and is not under a pass: the caller checks).
    pub auto_requested: bool,
    pub spend: Spend,
}

impl Policy {
    pub fn decide(&self, r: &Request) -> Verdict {
        let class = classify(r.command);
        if class == Class::Sudo {
            // the sudo gate: never auto, never silent — the exact command reaches the
            // owner and only the owner's signed verdict lets it through
            return Verdict::Ask(format!("sudo gate: {}", command_of(r.command)));
        }
        let levels = [&self.server, &self.agent, &self.session];
        // hard caps first: any level over its cap denies
        for l in levels {
            if l.tokens_cap.is_some_and(|cap| r.spend.tokens > cap) {
                return Verdict::Deny(format!(
                    "token budget spent ({} > {})",
                    r.spend.tokens,
                    l.tokens_cap.unwrap_or(0)
                ));
            }
            if l.gpu_cap.is_some_and(|cap| r.spend.gpu_seconds > cap) {
                return Verdict::Deny(format!(
                    "GPU budget spent ({:.0}s > {:.0}s)",
                    r.spend.gpu_seconds,
                    l.gpu_cap.unwrap_or(0.0)
                ));
            }
        }
        // soft thresholds: any level over its threshold asks
        for l in levels {
            if l.tokens_ask.is_some_and(|ask| r.spend.tokens > ask) {
                return Verdict::Ask(format!("token budget past {}", l.tokens_ask.unwrap_or(0)));
            }
            if l.gpu_ask.is_some_and(|ask| r.spend.gpu_seconds > ask) {
                return Verdict::Ask(format!("GPU budget past {:.0}s", l.gpu_ask.unwrap_or(0.0)));
            }
        }
        // auto mode survives unless a level forbids it (yesterday's posture); benign
        // commands pass when a level grants it and none forbids (a loosening nobody
        // granted is not one)
        if r.auto_requested && !levels.iter().any(|l| l.auto == Some(false)) {
            return Verdict::Allow;
        }
        if class == Class::Benign
            && levels.iter().any(|l| l.allow_benign == Some(true))
            && !levels.iter().any(|l| l.allow_benign == Some(false))
        {
            return Verdict::Allow;
        }
        Verdict::Ask("approval required".into())
    }
}

/// The journaled record of a decision (`schema/policy-decision.schema.json`).
pub fn record(
    agent: &str,
    node: &str,
    command: &str,
    verdict: &Verdict,
    spend: &Spend,
    at: f64,
) -> Value {
    let (v, reason) = match verdict {
        Verdict::Allow => ("allow", ""),
        Verdict::Ask(r) => ("ask", r.as_str()),
        Verdict::Deny(r) => ("deny", r.as_str()),
    };
    json!({
        "agent_id": agent,
        "node_id": node,
        "command": command,
        "class": match classify(command) {
            Class::Benign => "benign",
            Class::Regular => "regular",
            Class::Sudo => "sudo",
        },
        "verdict": v,
        "reason": reason,
        "spend": {"tokens": spend.tokens, "gpu_seconds": spend.gpu_seconds},
        "at": at,
    })
}
