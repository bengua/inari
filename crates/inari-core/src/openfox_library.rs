//! What OpenFox keeps beside its sessions, set from Inari's interface: its agents, skills,
//! commands and workflows, its MCP servers, its tools, and a few of its settings (instructions
//! for every project, the patterns that make a turn try again). Read on request (`library`),
//! changed by the node's OpenFox actions (`change`). Secrets (an MCP server's environment and
//! headers) go to OpenFox and are never read back: only their names are.

use crate::openfox::{enc, OpenFoxHarness};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// What the library holds, as OpenFox's own routes name it.
pub const KINDS: [&str; 4] = ["agents", "skills", "commands", "workflows"];

/// The OpenFox settings Inari shows and changes (none of them a secret).
pub const SETTINGS: [&str; 7] = [
    "global_instructions",
    "agent.language",
    "agent.retryPatterns",
    "agent.allowParallelSubAgents",
    "agent.autoContinueOnBoot",
    "llm.dynamicSystemPrompt",
    "tools.confirmOnWorkspaceActions",
];

/// OpenFox's types for what it keeps.
pub mod proto {
    use serde::de::IgnoredAny;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    /// `GET /api/agents|skills|commands|workflows`: OpenFox's own, the person's, the project's.
    #[derive(Deserialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct Listing {
        #[serde(default)]
        pub defaults: Vec<Item>,
        #[serde(default)]
        pub user_items: Vec<Item>,
        #[serde(default)]
        pub project_items: Vec<Item>,
        /// Skills only: every skill found, with where it comes from and whether it is on.
        #[serde(default)]
        pub items: Option<Vec<Item>>,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Item {
        pub id: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub description: Option<String>,
        #[serde(default)]
        pub enabled: Option<bool>,
        #[serde(default)]
        pub read_only: Option<bool>,
        #[serde(default)]
        pub source: Option<String>,
        /// Agents: a sub-agent is called by another, not chosen for a thread.
        #[serde(default)]
        pub subagent: Option<bool>,
    }

    /// `GET /api/agents|skills|commands/:id`.
    #[derive(Deserialize, Debug)]
    pub struct Definition {
        pub metadata: Item,
        #[serde(default)]
        pub prompt: String,
    }

    /// `GET /api/mcp/servers`.
    #[derive(Deserialize, Debug, Default)]
    pub struct McpServers {
        #[serde(default)]
        pub servers: Vec<McpServer>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct McpServer {
        pub name: String,
        pub config: McpConfig,
        #[serde(default)]
        pub status: String,
        #[serde(default)]
        pub tools: Vec<McpTool>,
        #[serde(default)]
        pub estimated_tokens: u64,
        #[serde(default)]
        pub error: Option<String>,
    }

    /// How OpenFox reaches an MCP server. Its environment and headers are secrets: their
    /// values are skipped as they are read, only their names are kept.
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct McpConfig {
        #[serde(default)]
        pub transport: String,
        #[serde(default)]
        pub command: Option<String>,
        #[serde(default)]
        pub args: Option<Vec<String>>,
        #[serde(default)]
        pub url: Option<String>,
        #[serde(default)]
        pub env: Option<BTreeMap<String, IgnoredAny>>,
        #[serde(default)]
        pub headers: Option<BTreeMap<String, IgnoredAny>>,
        #[serde(default)]
        pub oauth: Option<bool>,
        #[serde(default)]
        pub timeout: Option<u64>,
        #[serde(default)]
        pub disabled: Option<bool>,
    }

    #[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct McpTool {
        pub name: String,
        #[serde(default)]
        pub description: Option<String>,
        #[serde(default)]
        pub enabled: bool,
        #[serde(default)]
        pub estimated_tokens: u64,
    }

    /// `POST /api/mcp/servers/test`.
    #[derive(Deserialize, Debug)]
    pub struct McpTest {
        #[serde(default)]
        pub success: bool,
        #[serde(default)]
        pub tools: Vec<String>,
        #[serde(default)]
        pub error: Option<String>,
    }

    /// `GET /api/tools`.
    #[derive(Deserialize, Debug, Default)]
    pub struct Tools {
        #[serde(default)]
        pub tools: Vec<Tool>,
    }

    #[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct Tool {
        pub name: String,
        #[serde(default)]
        pub actions: Vec<String>,
        #[serde(default)]
        pub always_allowed: bool,
        #[serde(default)]
        pub top_level_only: bool,
        #[serde(default)]
        pub is_mcp: bool,
        #[serde(default)]
        pub mcp_server: Option<String>,
    }

    /// `GET /api/settings/:key`.
    #[derive(Deserialize, Debug)]
    pub struct Setting {
        #[serde(default)]
        pub value: Option<String>,
    }

    /// `agent.retryPatterns`: what in a model's answer makes OpenFox try the turn again.
    #[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct RetryPatterns {
        pub patterns: Vec<RetryPattern>,
        pub max_retries_per_turn: u32,
    }

    #[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
    pub struct RetryPattern {
        /// thinking, content or both.
        pub field: String,
        pub pattern: String,
        /// retry (the only one OpenFox knows).
        pub action: String,
        pub active: bool,
    }
}

/// One thing of the library as the page shows it.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// builtin (OpenFox's own), user (the person's) or project.
    pub scope: String,
    /// Skills: on or off.
    pub enabled: Option<bool>,
    /// OpenFox's own, or a shared skill: duplicated to be changed, never removed.
    pub read_only: bool,
    pub subagent: bool,
}

/// An MCP server as the page shows it: no secret, only the names of its variables and headers.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct McpView {
    pub name: String,
    pub status: String,
    pub error: Option<String>,
    pub transport: String,
    pub command: String,
    pub url: String,
    pub env: Vec<String>,
    pub headers: Vec<String>,
    pub disabled: bool,
    pub tokens: u64,
    pub tools: Vec<proto::McpTool>,
}

impl From<proto::McpServer> for McpView {
    fn from(s: proto::McpServer) -> Self {
        let c = s.config;
        let names = |m: Option<BTreeMap<String, serde::de::IgnoredAny>>| {
            m.map(|m| m.into_keys().collect()).unwrap_or_default()
        };
        Self {
            name: s.name,
            status: s.status,
            error: s.error,
            transport: c.transport,
            command: [
                c.command.unwrap_or_default(),
                c.args.unwrap_or_default().join(" "),
            ]
            .join(" ")
            .trim()
            .to_string(),
            url: c.url.unwrap_or_default(),
            env: names(c.env),
            headers: names(c.headers),
            disabled: c.disabled.unwrap_or(false),
            tokens: s.estimated_tokens,
            tools: s.tools,
        }
    }
}

fn entry(i: proto::Item, scope: &str) -> Entry {
    let read_only = i.read_only.unwrap_or(scope == "builtin");
    Entry {
        name: if i.name.is_empty() {
            i.id.clone()
        } else {
            i.name
        },
        id: i.id,
        description: i.description.unwrap_or_default(),
        scope: scope.into(),
        enabled: i.enabled,
        read_only,
        subagent: i.subagent.unwrap_or(false),
    }
}

/// An id that goes into one of OpenFox's paths.
fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 128
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// "KEY=value" lines (an MCP server's environment) or "Key: value" lines (its headers).
fn pairs(text: &str, sep: char) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (k, v) = line
            .split_once(sep)
            .ok_or_else(|| format!("{line:?} is not NAME{sep}value"))?;
        let k = k.trim();
        if k.is_empty() || k.len() > 128 {
            return Err(format!("{line:?} has no name"));
        }
        out.insert(k.to_string(), v.trim().to_string());
    }
    Ok(out)
}

/// An MCP server as the form gives it: what OpenFox's `POST /api/mcp/servers` takes.
fn mcp_body(p: &Value) -> Result<Value, String> {
    let text = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let name = text("name");
    if !id_like(&name) {
        return Err("an MCP server's name is letters, digits, - _ and . only".into());
    }
    let transport = if text("transport") == "http" {
        "http"
    } else {
        "stdio"
    };
    let mut body = json!({"name": name, "transport": transport});
    if transport == "http" {
        let url = text("url");
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("an HTTP MCP server's address starts with http:// or https://".into());
        }
        body["url"] = json!(url);
        let headers = pairs(&text("headers"), ':')?;
        if !headers.is_empty() {
            body["headers"] = json!(headers);
        }
    } else {
        let line = text("command");
        let mut words = line.split_whitespace().map(String::from);
        let command = words.next().ok_or("a local MCP server needs its command")?;
        body["command"] = json!(command);
        let args: Vec<String> = words.collect();
        if !args.is_empty() {
            body["args"] = json!(args);
        }
        let env = pairs(&text("env"), '=')?;
        if !env.is_empty() {
            body["env"] = json!(env);
        }
    }
    if let Some(t) = p.get("timeout").and_then(Value::as_u64).filter(|t| *t > 0) {
        body["timeout"] = json!(t * 1000);
    }
    Ok(body)
}

/// A setting's value as OpenFox keeps it (a string), checked first.
fn setting_value(key: &str, value: &Value) -> Result<String, String> {
    match key {
        "agent.retryPatterns" => {
            let v: proto::RetryPatterns = serde_json::from_value(value.clone())
                .map_err(|e| format!("the retry patterns: {e}"))?;
            if v.max_retries_per_turn > 100 {
                return Err("100 tries a turn at most".into());
            }
            for p in &v.patterns {
                if !["thinking", "content", "both"].contains(&p.field.as_str())
                    || p.action != "retry"
                    || p.pattern.trim().is_empty()
                    || p.pattern.len() > 1000
                {
                    return Err(format!("not a retry pattern: {:?}", p.pattern));
                }
            }
            Ok(serde_json::to_string(&v).expect("serializes"))
        }
        "agent.allowParallelSubAgents"
        | "agent.autoContinueOnBoot"
        | "llm.dynamicSystemPrompt"
        | "tools.confirmOnWorkspaceActions" => match value.as_bool() {
            Some(b) => Ok(b.to_string()),
            None => Err(format!("{key} is on or off")),
        },
        _ => match value.as_str() {
            Some(t) if t.chars().count() <= 20_000 => Ok(t.trim().to_string()),
            _ => Err(format!("{key} is a text of 20,000 characters at most")),
        },
    }
}

impl OpenFoxHarness {
    /// Read part of the library: a kind's list, one item of it (`id`), the MCP servers, the
    /// tools, or the settings Inari shows.
    pub async fn library(&self, kind: &str, id: &str) -> Result<Value, String> {
        match kind {
            k if KINDS.contains(&k) && id.is_empty() => {
                let l: proto::Listing = self.call("GET", &format!("/api/{k}"), None).await?;
                let mut out: Vec<Entry> = Vec::new();
                match l.items {
                    // skills say where each comes from: bundled ones are OpenFox's own
                    Some(items) => {
                        for i in items {
                            let scope = match i.source.as_deref() {
                                Some("bundled") => "builtin",
                                Some(s) if s.starts_with("project") => "project",
                                _ => "user",
                            };
                            out.push(entry(i, scope));
                        }
                    }
                    None => {
                        for (items, scope) in [
                            (l.project_items, "project"),
                            (l.user_items, "user"),
                            (l.defaults, "builtin"),
                        ] {
                            for i in items {
                                if !out.iter().any(|x| x.id == i.id) {
                                    out.push(entry(i, scope));
                                }
                            }
                        }
                    }
                }
                Ok(json!({"items": out}))
            }
            "workflows" => {
                if !id_like(id) {
                    return Err(format!("unknown workflow {id:?}"));
                }
                // a workflow is its whole definition, edited as JSON
                let w: Value = self
                    .call("GET", &format!("/api/workflows/{id}"), None)
                    .await?;
                Ok(json!({"id": id, "json": serde_json::to_string_pretty(&w).expect("serializes")}))
            }
            k if KINDS.contains(&k) => {
                if !id_like(id) {
                    return Err(format!("unknown {k} {id:?}"));
                }
                let d: proto::Definition =
                    self.call("GET", &format!("/api/{k}/{id}"), None).await?;
                Ok(json!({"id": id, "name": d.metadata.name,
                          "description": d.metadata.description.unwrap_or_default(), "prompt": d.prompt}))
            }
            "mcp" => {
                let s: proto::McpServers = self.call("GET", "/api/mcp/servers", None).await?;
                let servers: Vec<McpView> = s.servers.into_iter().map(McpView::from).collect();
                Ok(json!({"servers": servers}))
            }
            "tools" => {
                let t: proto::Tools = self.call("GET", "/api/tools", None).await?;
                Ok(json!({"tools": t.tools}))
            }
            "settings" => {
                let mut out = serde_json::Map::new();
                for key in SETTINGS {
                    let s: proto::Setting = self
                        .call("GET", &format!("/api/settings/{}", enc(key)), None)
                        .await?;
                    let v = s.value.unwrap_or_default();
                    out.insert(
                        key.into(),
                        match key {
                            "agent.retryPatterns" => serde_json::from_str(&v)
                                .unwrap_or(json!({"patterns": [], "maxRetriesPerTurn": 10})),
                            k if k.contains("allow")
                                || k.contains("auto")
                                || k.contains("dynamic")
                                || k.contains("confirm") =>
                            {
                                json!(v == "true")
                            }
                            _ => json!(v),
                        },
                    );
                }
                Ok(Value::Object(out))
            }
            other => Err(format!("OpenFox keeps no {other:?}")),
        }
    }

    /// Change the library (the node's OpenFox actions); the detail says what came of a test.
    pub async fn change(&self, action: &str, p: &Value) -> Result<String, String> {
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let kind = || -> Result<String, String> {
            let k = text("of");
            if KINDS.contains(&k.as_str()) {
                Ok(k)
            } else {
                Err(format!("OpenFox keeps no {k:?}"))
            }
        };
        let id = || -> Result<String, String> {
            let i = text("id");
            if id_like(&i) {
                Ok(i)
            } else {
                Err(format!("unknown item {i:?}"))
            }
        };
        let mcp = || -> Result<String, String> {
            let n = text("name");
            if id_like(&n) {
                Ok(enc(&n))
            } else {
                Err(format!("unknown MCP server {n:?}"))
            }
        };
        match action {
            "item_save" => {
                let (k, i) = (kind()?, id()?);
                let body = if k == "workflows" {
                    let w: Value = serde_json::from_str(&text("json"))
                        .map_err(|e| format!("the workflow is not JSON: {e}"))?;
                    if !w.get("steps").is_some_and(Value::is_array) {
                        return Err("a workflow has its steps".into());
                    }
                    w
                } else {
                    let prompt = text("prompt");
                    if prompt.trim().is_empty() || prompt.len() > 200_000 {
                        return Err(
                            "its prompt is what it does: not empty, 200,000 characters at most"
                                .into(),
                        );
                    }
                    let mut meta = json!({"name": text("name").trim()});
                    if k != "commands" {
                        meta["description"] = json!(text("description").trim());
                    }
                    json!({"metadata": meta, "prompt": prompt})
                };
                self.act("PUT", &format!("/api/{k}/{i}"), body).await?;
                Ok(String::new())
            }
            "item_duplicate" => {
                self.act(
                    "POST",
                    &format!("/api/{}/{}/duplicate", kind()?, id()?),
                    json!({"destination": "user"}),
                )
                .await?;
                Ok(String::new())
            }
            "item_remove" => {
                self.act("DELETE", &format!("/api/{}/{}", kind()?, id()?), json!({}))
                    .await?;
                Ok(String::new())
            }
            "skill_toggle" => {
                self.act("POST", &format!("/api/skills/{}/toggle", id()?), json!({}))
                    .await?;
                Ok(String::new())
            }
            "mcp_add" => {
                self.act("POST", "/api/mcp/servers", mcp_body(p)?).await?;
                Ok(String::new())
            }
            "mcp_test" => {
                let t: proto::McpTest = self
                    .call("POST", "/api/mcp/servers/test", Some(mcp_body(p)?))
                    .await?;
                if t.success {
                    Ok(format!("OpenFox reaches it: {}", t.tools.join(", ")))
                } else {
                    Err(t
                        .error
                        .unwrap_or_else(|| "OpenFox does not reach it".into()))
                }
            }
            "mcp_toggle" => {
                let off = p.get("disabled").and_then(Value::as_bool).unwrap_or(false);
                self.act(
                    "PUT",
                    &format!("/api/mcp/servers/{}", mcp()?),
                    json!({"disabled": off}),
                )
                .await?;
                Ok(String::new())
            }
            "mcp_tool" => {
                let tool = text("tool");
                if tool.is_empty() || tool.len() > 200 {
                    return Err(format!("unknown tool {tool:?}"));
                }
                let on = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                self.act(
                    "PUT",
                    &format!("/api/mcp/servers/{}/tools/{}", mcp()?, enc(&tool)),
                    json!({"enabled": on}),
                )
                .await?;
                Ok(String::new())
            }
            "mcp_remove" => {
                self.act("DELETE", &format!("/api/mcp/servers/{}", mcp()?), json!({}))
                    .await?;
                Ok(String::new())
            }
            "setting" => {
                let key = text("key");
                if !SETTINGS.contains(&key.as_str()) {
                    return Err(format!("Inari does not change OpenFox's {key:?}"));
                }
                let value = setting_value(&key, p.get("value").unwrap_or(&Value::Null))?;
                self.act(
                    "PUT",
                    &format!("/api/settings/{}", enc(&key)),
                    json!({"value": value}),
                )
                .await?;
                Ok(String::new())
            }
            other => Err(format!("unknown OpenFox action {other:?}")),
        }
    }
}

/// The node's OpenFox actions this module does.
pub const ACTIONS: [&str; 10] = [
    "item_save",
    "item_duplicate",
    "item_remove",
    "skill_toggle",
    "mcp_add",
    "mcp_test",
    "mcp_toggle",
    "mcp_tool",
    "mcp_remove",
    "setting",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mcp_server_is_read_without_its_secrets() {
        let raw = json!({"servers": [{"name": "gh", "status": "connected", "estimatedTokens": 900,
            "config": {"transport": "stdio", "command": "npx", "args": ["-y", "gh-mcp"],
                       "env": {"GITHUB_TOKEN": "ghp_secret"}, "disabled": false},
            "tools": [{"name": "issues", "enabled": true, "estimatedTokens": 300, "inputSchema": {}}]}]});
        let s: proto::McpServers = serde_json::from_value(raw).unwrap();
        let v: Vec<McpView> = s.servers.into_iter().map(McpView::from).collect();
        let shown = serde_json::to_string(&v).unwrap();
        assert!(!shown.contains("ghp_secret"));
        assert_eq!(v[0].env, vec!["GITHUB_TOKEN".to_string()]);
        assert_eq!(v[0].command, "npx -y gh-mcp");
    }

    #[test]
    fn an_mcp_form_becomes_what_openfox_takes() {
        let b = mcp_body(&json!({"name": "gh", "command": "npx -y gh-mcp", "env": "A=1\nB = two"}))
            .unwrap();
        assert_eq!(b["args"], json!(["-y", "gh-mcp"]));
        assert_eq!(b["env"], json!({"A": "1", "B": "two"}));
        assert!(mcp_body(&json!({"name": "x y", "command": "a"})).is_err());
        assert!(mcp_body(&json!({"name": "web", "transport": "http", "url": "ftp://x"})).is_err());
        let h = mcp_body(&json!({"name": "web", "transport": "http", "url": "https://x/mcp", "headers": "Authorization: Bearer t"}))
            .unwrap();
        assert_eq!(h["headers"], json!({"Authorization": "Bearer t"}));
    }

    #[test]
    fn settings_are_checked_before_they_go() {
        let ok = json!({"patterns": [{"field": "content", "pattern": "^Error", "action": "retry", "active": true}], "maxRetriesPerTurn": 3});
        assert!(setting_value("agent.retryPatterns", &ok).is_ok());
        let bad = json!({"patterns": [{"field": "elsewhere", "pattern": "x", "action": "retry", "active": true}], "maxRetriesPerTurn": 3});
        assert!(setting_value("agent.retryPatterns", &bad).is_err());
        assert_eq!(
            setting_value("llm.dynamicSystemPrompt", &json!(true)).unwrap(),
            "true"
        );
        assert!(setting_value("llm.dynamicSystemPrompt", &json!("yes")).is_err());
    }
}
