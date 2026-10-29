//! MCP servers for the local engine: a command the core runs (stdio) or an address on the
//! machine or the private network (HTTP), set from the interface. Their tools are offered to
//! the agents as `<server>_<tool>`. How each is reached is kept in `<engine>/mcp.json`; its
//! secrets (environment, headers) apart in `mcp_secrets.json` (mode 600), never shown again:
//! only their names are.

use super::{llm, LocalEngine};
use crate::capsule::io;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Stdio as Piped;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// How long a server may take to start and list its tools.
const CONNECT: Duration = Duration::from_secs(30);

/// An MCP server as the settings keep it (no secret here).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    /// stdio or http.
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: String,
    /// How long a tool call may take, in seconds (0: two minutes).
    #[serde(default)]
    pub timeout: u64,
    #[serde(default)]
    pub disabled: bool,
    /// The tools the agents are not offered.
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Secret {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct ToolInfo {
    name: String,
    description: String,
    schema: Value,
}

struct Stdio {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Drop for Stdio {
    fn drop(&mut self) {
        if let Some(pid) = self.child.id() {
            // SAFETY: signalling the process group this module started.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
}

/// A live link to a server (held in an Arc).
#[allow(clippy::large_enum_variant)]
enum Link {
    Stdio(tokio::sync::Mutex<Stdio>),
    Http {
        url: String,
        headers: Vec<(String, String)>,
        session: Mutex<Option<String>>,
    },
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl Link {
    async fn open(s: &McpServer, secret: &Secret, dir: &Path) -> Result<Link, String> {
        let link = if s.transport == "http" {
            Link::Http {
                url: s.url.clone(),
                headers: secret
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                session: Mutex::new(None),
            }
        } else {
            let mut cmd = Command::new(&s.command);
            cmd.args(&s.args)
                .envs(&secret.env)
                .current_dir(if dir.is_dir() { dir } else { Path::new("/") })
                .stdin(Piped::piped())
                .stdout(Piped::piped())
                .stderr(Piped::null())
                .process_group(0)
                .kill_on_drop(true);
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("cannot start {}: {e}", s.command))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let out = BufReader::new(child.stdout.take().ok_or("no stdout")?);
            Link::Stdio(tokio::sync::Mutex::new(Stdio { child, stdin, out }))
        };
        tokio::time::timeout(CONNECT, async {
            link.request(
                "initialize",
                json!({"protocolVersion": "2025-03-26", "capabilities": {},
                       "clientInfo": {"name": "inari", "version": env!("CARGO_PKG_VERSION")}}),
            )
            .await?;
            link.notify("notifications/initialized").await;
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| format!("{} did not start within {} s", s.name, CONNECT.as_secs()))??;
        Ok(link)
    }

    async fn notify(&self, method: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        match self {
            Link::Stdio(io) => {
                let mut io = io.lock().await;
                let _ = io.stdin.write_all(format!("{msg}\n").as_bytes()).await;
                let _ = io.stdin.flush().await;
            }
            Link::Http {
                url,
                headers,
                session,
            } => {
                let mut h = headers.clone();
                h.push((
                    "Accept".into(),
                    "application/json, text/event-stream".into(),
                ));
                if let Some(id) = session.lock().expect("not poisoned").clone() {
                    h.push(("Mcp-Session-Id".into(), id));
                }
                let _ = llm::exchange(url, &h, &msg, Duration::from_secs(10)).await;
            }
        }
    }

    /// A request and its result (an error from the server is an Err).
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let answer = match self {
            Link::Stdio(io) => {
                let mut io = io.lock().await;
                io.stdin
                    .write_all(format!("{msg}\n").as_bytes())
                    .await
                    .map_err(|e| format!("the server is gone ({e})"))?;
                io.stdin.flush().await.map_err(|e| e.to_string())?;
                loop {
                    let mut line = String::new();
                    if io
                        .out
                        .read_line(&mut line)
                        .await
                        .map_err(|e| e.to_string())?
                        == 0
                    {
                        return Err("the server stopped".into());
                    }
                    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                        continue;
                    };
                    if v.get("id") == Some(&json!(id)) && v.get("method").is_none() {
                        break v;
                    }
                    // a request from the server (roots, sampling): Inari offers none
                    if let (Some(rid), Some(_)) = (v.get("id"), v.get("method")) {
                        let no = json!({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "not supported"}});
                        let _ = io.stdin.write_all(format!("{no}\n").as_bytes()).await;
                    }
                }
            }
            Link::Http {
                url,
                headers,
                session,
            } => {
                let mut h = headers.clone();
                h.push((
                    "Accept".into(),
                    "application/json, text/event-stream".into(),
                ));
                if let Some(sid) = session.lock().expect("not poisoned").clone() {
                    h.push(("Mcp-Session-Id".into(), sid));
                }
                let (status, got, raw) =
                    llm::exchange(url, &h, &msg, Duration::from_secs(600)).await?;
                if let Some((_, sid)) = got.iter().find(|(k, _)| k == "mcp-session-id") {
                    *session.lock().expect("not poisoned") = Some(sid.clone());
                }
                if !(200..300).contains(&status) {
                    return Err(format!(
                        "HTTP {status}: {}",
                        String::from_utf8_lossy(&raw[..raw.len().min(300)])
                    ));
                }
                let text = String::from_utf8_lossy(&raw);
                let sse = got
                    .iter()
                    .any(|(k, v)| k == "content-type" && v.contains("text/event-stream"));
                let mut found = None;
                let candidates: Vec<Value> = if sse {
                    text.lines()
                        .filter_map(|l| l.strip_prefix("data:"))
                        .filter_map(|d| serde_json::from_str(d.trim()).ok())
                        .collect()
                } else {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(Value::Array(a)) => a,
                        Ok(v) => vec![v],
                        Err(e) => return Err(format!("not JSON: {e}")),
                    }
                };
                for v in candidates {
                    if v.get("id") == Some(&json!(id)) {
                        found = Some(v);
                    }
                }
                found.ok_or("the server gave no answer")?
            }
        };
        if let Some(e) = answer.get("error") {
            return Err(e
                .get("message")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| e.to_string()));
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn tools(&self) -> Result<Vec<ToolInfo>, String> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.request("tools/list", params).await?;
            for t in r
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = t.get("name").and_then(Value::as_str) else {
                    continue;
                };
                out.push(ToolInfo {
                    name: name.to_string(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                });
            }
            cursor = r
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

/// The servers' links and what they offer, kept while the core runs.
#[derive(Default)]
pub(crate) struct Hub {
    links: tokio::sync::Mutex<HashMap<String, Arc<Link>>>,
    tools: Mutex<BTreeMap<String, Vec<ToolInfo>>>,
    errors: Mutex<BTreeMap<String, String>>,
}

/// A tool's name as the model sees it: `<server>_<tool>`, letters, digits, _ and - only.
fn exposed(server: &str, tool: &str) -> String {
    format!("{server}_{tool}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 64
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// "KEY=value" lines (the environment) or "Key: value" lines (headers).
fn pairs(text: &str, sep: char) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (k, v) = line
            .split_once(sep)
            .ok_or_else(|| format!("{line:?} is not NAME{sep}value"))?;
        let k = k.trim();
        if k.is_empty() || k.len() > 128 || k.contains(['\r', '\n']) {
            return Err(format!("{line:?} has no name"));
        }
        out.insert(k.to_string(), v.trim().to_string());
    }
    Ok(out)
}

/// A server as the form gives it, and its secret.
fn from_form(p: &Value) -> Result<(McpServer, Secret), String> {
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
    let mut s = McpServer {
        name,
        transport: if text("transport") == "http" {
            "http"
        } else {
            "stdio"
        }
        .into(),
        timeout: p
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(3600),
        ..Default::default()
    };
    let mut secret = Secret::default();
    if s.transport == "http" {
        s.url = text("url");
        if !s.url.starts_with("http://") {
            return Err(
                "the local engine reaches an MCP server over plain http, on the machine or the private network (http://host:port/mcp)"
                    .into(),
            );
        }
        secret.headers = pairs(&text("headers"), ':')?;
    } else {
        let line = text("command");
        let mut words = line.split_whitespace().map(String::from);
        s.command = words.next().ok_or("a local MCP server needs its command")?;
        s.args = words.collect();
        secret.env = pairs(&text("env"), '=')?;
    }
    Ok((s, secret))
}

impl LocalEngine {
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        std::fs::read(self.dir.join("mcp.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_mcp(&self, list: &[McpServer]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        io::atomic_write(
            &self.dir.join("mcp.json"),
            &serde_json::to_vec_pretty(list).expect("serializes"),
        )
        .map_err(|e| e.to_string())
    }

    fn mcp_secrets(&self) -> BTreeMap<String, Secret> {
        std::fs::read(self.dir.join("mcp_secrets.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_secrets(&self, all: &BTreeMap<String, Secret>) -> Result<(), String> {
        let path = self.dir.join("mcp_secrets.json");
        io::atomic_write(&path, &serde_json::to_vec(all).expect("serializes"))
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Start the servers that are on and not linked yet, list their tools; stop the others.
    pub(crate) async fn mcp_ready(&self) {
        let servers = self.mcp_servers();
        let secrets = self.mcp_secrets();
        let mut links = self.mcp.links.lock().await;
        links.retain(|name, _| servers.iter().any(|s| &s.name == name && !s.disabled));
        self.mcp
            .tools
            .lock()
            .expect("not poisoned")
            .retain(|name, _| links.contains_key(name));
        for s in servers.iter().filter(|s| !s.disabled) {
            if links.contains_key(&s.name) {
                continue;
            }
            let secret = secrets.get(&s.name).cloned().unwrap_or_default();
            let got = match Link::open(s, &secret, &self.dir).await {
                Ok(link) => match tokio::time::timeout(CONNECT, link.tools()).await {
                    Ok(Ok(tools)) => Ok((link, tools)),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(format!("{} did not list its tools in time", s.name)),
                },
                Err(e) => Err(e),
            };
            match got {
                Ok((link, tools)) => {
                    links.insert(s.name.clone(), Arc::new(link));
                    self.mcp
                        .tools
                        .lock()
                        .expect("not poisoned")
                        .insert(s.name.clone(), tools);
                    self.mcp
                        .errors
                        .lock()
                        .expect("not poisoned")
                        .remove(&s.name);
                }
                Err(e) => {
                    self.mcp
                        .errors
                        .lock()
                        .expect("not poisoned")
                        .insert(s.name.clone(), e);
                }
            }
        }
    }

    /// Forget a server's link (it is started again when next needed).
    async fn mcp_drop(&self, name: &str) {
        self.mcp.links.lock().await.remove(name);
        self.mcp.tools.lock().expect("not poisoned").remove(name);
        self.mcp.errors.lock().expect("not poisoned").remove(name);
    }

    /// The tools the servers offer now, as the model names them, with their server and own name.
    fn mcp_offered(&self) -> Vec<(String, String, ToolInfo)> {
        let servers = self.mcp_servers();
        let tools = self.mcp.tools.lock().expect("not poisoned");
        let mut out = Vec::new();
        for s in servers.iter().filter(|s| !s.disabled) {
            for t in tools.get(&s.name).into_iter().flatten() {
                if !s.disabled_tools.contains(&t.name) {
                    out.push((exposed(&s.name, &t.name), s.name.clone(), t.clone()));
                }
            }
        }
        out
    }

    pub(crate) fn mcp_tool_names(&self) -> Vec<String> {
        self.mcp_offered().into_iter().map(|(n, _, _)| n).collect()
    }

    pub(crate) fn mcp_definitions(&self, allowed: &[String]) -> Vec<Value> {
        self.mcp_offered()
            .into_iter()
            .filter(|(n, _, _)| allowed.contains(n))
            .map(|(n, server, t)| {
                json!({"type": "function", "function": {"name": n,
                    "description": format!("[MCP {server}] {}", t.description),
                    "parameters": t.schema}})
            })
            .collect()
    }

    pub(crate) async fn mcp_call(&self, name: &str, args: &Value) -> Result<String, String> {
        let Some((_, server, tool)) = self.mcp_offered().into_iter().find(|(n, _, _)| n == name)
        else {
            return Err(format!("unknown tool {name}"));
        };
        let link = self.mcp.links.lock().await.get(&server).cloned();
        let link = link.ok_or_else(|| format!("the MCP server {server} is not running"))?;
        let wait = self
            .mcp_servers()
            .iter()
            .find(|s| s.name == server)
            .map(|s| s.timeout)
            .filter(|t| *t > 0)
            .unwrap_or(120);
        let args = if args.is_object() {
            args.clone()
        } else {
            json!({})
        };
        let r = tokio::time::timeout(
            Duration::from_secs(wait),
            link.request("tools/call", json!({"name": tool.name, "arguments": args})),
        )
        .await;
        let r = match r {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                if e.contains("stopped") || e.contains("gone") {
                    self.mcp_drop(&server).await;
                }
                return Err(format!("{server}: {e}"));
            }
            Err(_) => return Err(format!("{server} did not answer within {wait} s")),
        };
        let mut out = Vec::new();
        for c in r
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match c.get("type").and_then(Value::as_str) {
                Some("text") => out.push(
                    c.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                Some("resource") => out.push(
                    c.pointer("/resource/text")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| c["resource"].to_string()),
                ),
                Some(other) => out.push(format!("[{other} content]")),
                None => {}
            }
        }
        if out.is_empty() {
            if let Some(s) = r.get("structuredContent") {
                out.push(s.to_string());
            }
        }
        let text = out.join("\n");
        if r.get("isError").and_then(Value::as_bool) == Some(true) {
            Err(text)
        } else {
            Ok(text)
        }
    }

    /// The servers as the page shows them: no secret, only their names.
    pub(crate) async fn mcp_view(&self) -> Value {
        // the page waits: a server slow to start says so next time
        let _ = tokio::time::timeout(Duration::from_secs(10), self.mcp_ready()).await;
        let secrets = self.mcp_secrets();
        let tools = self.mcp.tools.lock().expect("not poisoned").clone();
        let errors = self.mcp.errors.lock().expect("not poisoned").clone();
        let servers: Vec<Value> = self
            .mcp_servers()
            .into_iter()
            .map(|s| {
                let secret = secrets.get(&s.name).cloned().unwrap_or_default();
                let listed: Vec<Value> = tools
                    .get(&s.name)
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        json!({"name": t.name, "description": t.description,
                               "enabled": !s.disabled_tools.contains(&t.name),
                               "estimatedTokens": (t.description.len() + t.schema.to_string().len()) / 4})
                    })
                    .collect();
                let tokens: u64 = listed
                    .iter()
                    .filter(|t| t["enabled"] == true)
                    .map(|t| t["estimatedTokens"].as_u64().unwrap_or(0))
                    .sum();
                let status = if s.disabled {
                    "disabled"
                } else if errors.contains_key(&s.name) {
                    "error"
                } else if tools.contains_key(&s.name) {
                    "connected"
                } else {
                    "disconnected"
                };
                json!({"name": s.name, "status": status, "error": errors.get(&s.name),
                       "transport": s.transport,
                       "command": format!("{} {}", s.command, s.args.join(" ")).trim(),
                       "url": s.url, "env": secret.env.keys().collect::<Vec<_>>(),
                       "headers": secret.headers.keys().collect::<Vec<_>>(),
                       "disabled": s.disabled, "tokens": tokens, "tools": listed})
            })
            .collect();
        json!({"servers": servers})
    }

    /// Change the servers from the interface; a test's detail says what it offers.
    pub(crate) async fn mcp_change(&self, action: &str, p: &Value) -> Result<String, String> {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut list = self.mcp_servers();
        let at = list.iter().position(|s| s.name == name);
        match action {
            "mcp_add" => {
                let (s, secret) = from_form(p)?;
                let mut secrets = self.mcp_secrets();
                secrets.insert(s.name.clone(), secret);
                match list.iter().position(|x| x.name == s.name) {
                    Some(i) => list[i] = s.clone(),
                    None => list.push(s.clone()),
                }
                self.save_mcp(&list)?;
                self.save_secrets(&secrets)?;
                self.mcp_drop(&s.name).await;
                Ok(String::new())
            }
            "mcp_test" => {
                let (s, secret) = from_form(p)?;
                let link = Link::open(&s, &secret, &self.dir).await?;
                let tools = tokio::time::timeout(CONNECT, link.tools())
                    .await
                    .map_err(|_| "it did not list its tools in time".to_string())??;
                Ok(format!(
                    "Inari reaches it: {}",
                    tools
                        .iter()
                        .map(|t| t.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
            "mcp_toggle" => {
                let i = at.ok_or_else(|| format!("no MCP server {name:?}"))?;
                list[i].disabled = p.get("disabled").and_then(Value::as_bool).unwrap_or(false);
                self.save_mcp(&list)?;
                self.mcp_drop(&name).await;
                Ok(String::new())
            }
            "mcp_tool" => {
                let i = at.ok_or_else(|| format!("no MCP server {name:?}"))?;
                let tool = p
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let on = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                list[i].disabled_tools.retain(|t| *t != tool);
                if !on {
                    list[i].disabled_tools.push(tool);
                }
                self.save_mcp(&list).map(|_| String::new())
            }
            "mcp_remove" => {
                let i = at.ok_or_else(|| format!("no MCP server {name:?}"))?;
                list.remove(i);
                self.save_mcp(&list)?;
                let mut secrets = self.mcp_secrets();
                secrets.remove(&name);
                self.save_secrets(&secrets)?;
                self.mcp_drop(&name).await;
                Ok(String::new())
            }
            other => Err(format!("unknown action {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_gives_a_server_and_its_secret() {
        let (s, secret) = from_form(&json!({"name": "gh", "transport": "stdio",
            "command": "npx -y gh-mcp", "env": "TOKEN=abc\n"}))
        .unwrap();
        assert_eq!((s.command.as_str(), s.args.len()), ("npx", 2));
        assert_eq!(secret.env["TOKEN"], "abc");
        assert!(
            from_form(&json!({"name": "x", "transport": "http", "url": "https://far/mcp"}))
                .is_err()
        );
        assert!(from_form(&json!({"name": "../x", "command": "a"})).is_err());
        assert_eq!(exposed("my.server", "do it"), "my_server_do_it");
    }
}
