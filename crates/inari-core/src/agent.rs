//! The agent runner and the ACP harness. Every agent the core drives — Claude Code
//! (through claude-agent-acp), Gemini CLI (`--acp`), OpenFox (openfox-acp) — speaks
//! the Agent Client Protocol, so one harness covers them all. The journal stays the
//! source of truth; attachments reach the model: images as image content when the
//! agent accepts it, other files as links into the workspace, always named in the text.

use crate::capsule::attach::{attach as attach_file, is_image, sniff, Attached};
use crate::capsule::io::{self, safe_relative};
use crate::capsule::{EventKind, TimelineEvent};
use crate::host::CapsuleHost;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

const ENV_KEEP: [&str; 6] = [
    "HOME",
    "PATH",
    "USER",
    "LANG",
    "XDG_CONFIG_HOME",
    "XDG_RUNTIME_DIR",
];

pub type Publisher = Arc<dyn Fn(&str, Value) + Send + Sync>;
pub type Approver =
    Arc<dyn Fn(String, String) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunSummary {
    pub stopped: String, // done | frozen | error
    pub final_text: String,
    pub error: String,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Flatten an ACP content block (or a list of them) to text.
fn content_text(v: &Value) -> String {
    match v {
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("content") => {
            content_text(o.get("content").unwrap_or(&Value::Null))
        }
        Value::Object(o) => o
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Value::Array(a) => a
            .iter()
            .map(content_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// One line per event, as people and priming read it.
pub fn summary(e: &TimelineEvent) -> String {
    let d = &e.data;
    match e.kind {
        EventKind::ToolCall => {
            let name = d.get("name").and_then(Value::as_str).unwrap_or("tool");
            let args = d.get("arguments").cloned().unwrap_or(Value::Null);
            if args.get("by").and_then(Value::as_str) == Some("user") {
                return format!(
                    "$ {}",
                    args.get("cmd").and_then(Value::as_str).unwrap_or("")
                );
            }
            let inner = args.get("input").filter(|v| v.is_object()).unwrap_or(&args);
            let detail = [
                "cmd",
                "command",
                "path",
                "file_path",
                "url",
                "query",
                "pattern",
            ]
            .iter()
            .find_map(|k| inner.get(*k).and_then(Value::as_str));
            match detail {
                Some(x) if !name.contains(':') => format!("{name}: {x}"),
                _ => name.to_string(),
            }
        }
        EventKind::ProviderSwitch => d
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        EventKind::Checkpoint => d
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => ["text", "name", "output"]
            .iter()
            .find_map(|k| d.get(*k).and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
    }
}

/// What a harness needs to run one user turn.
pub struct Turn {
    pub runner: Arc<Runner>,
    pub agent: String,
    pub text: String,
    pub attachments: Vec<Attached>,
    pub workdir: PathBuf,
    pub model: String,
}

impl Turn {
    fn path(&self) -> PathBuf {
        self.runner.host.agents.join(&self.agent)
    }
    fn append(&self, kind: EventKind, data: Value) -> Option<TimelineEvent> {
        let author = format!("{}@{}", self.runner.host.node_id, self.agent);
        io::append_event(&self.path(), &author, kind, data, now())
            .ok()
            .and_then(|c| c.timeline.ordered().last().cloned().cloned())
    }
    fn emit(&self, e: &TimelineEvent) {
        self.runner.emit(&self.agent, e);
    }
    fn phase(&self, phase: &str) {
        self.runner.set_phase(&self.agent, phase);
    }
    fn running(&self) -> bool {
        self.runner.host.status(&self.agent) == "running"
    }
}

pub struct AcpHarness {
    pub command: Vec<String>,
    sessions_file: PathBuf,
    pub timeout: Duration,
    pub prime_chars: usize,
    pub extra_env: BTreeMap<String, String>,
    models_set: Mutex<HashMap<String, String>>,
}

/// Calls awaiting the agent's reply, by request id.
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

struct Rpc {
    stdin: ChildStdin,
    next: u64,
    pending: Pending,
}

impl Rpc {
    async fn send(&mut self, v: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(v).expect("serializes");
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .map_err(|e| format!("the agent closed its input: {e}"))
    }
    async fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
        self.next += 1;
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("not poisoned")
            .insert(self.next, tx);
        self.send(&json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}))
            .await?;
        Ok(rx)
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request(method, params)
            .await?
            .await
            .map_err(|_| "the agent process ended".to_string())?
    }
}

enum Incoming {
    Update(Value),
    Request(u64, String, Value),
}

impl AcpHarness {
    pub fn new(command: Vec<String>, sessions_file: PathBuf) -> Self {
        Self {
            command,
            sessions_file,
            timeout: Duration::from_secs(900),
            prime_chars: 24_000,
            extra_env: BTreeMap::new(),
            models_set: Mutex::default(),
        }
    }

    fn sessions(&self) -> Map<String, Value> {
        std::fs::read(&self.sessions_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn remember(&self, agent: &str, sid: &str) {
        let mut all = self.sessions();
        all.insert(agent.into(), json!(sid));
        let _ = io::atomic_write(
            &self.sessions_file,
            serde_json::to_string(&all).expect("serializes").as_bytes(),
        );
    }

    fn spawn(&self, cwd: &Path) -> std::io::Result<Child> {
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..])
            .current_dir(cwd)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for k in ENV_KEEP {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        cmd.envs(&self.extra_env);
        cmd.spawn()
    }

    /// Read the agent's stdout: replies resolve pending calls, the rest goes to the turn.
    fn read_loop(child: &mut Child, pending: Pending) -> mpsc::UnboundedReceiver<Incoming> {
        let stdout = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                }; // banners
                let method = msg.get("method").and_then(Value::as_str).map(String::from);
                match (method, msg.get("id").and_then(Value::as_u64)) {
                    (Some(m), Some(id)) => {
                        let _ = tx.send(Incoming::Request(
                            id,
                            m,
                            msg.get("params").cloned().unwrap_or(Value::Null),
                        ));
                    }
                    (Some(m), None) if m == "session/update" => {
                        let update = msg
                            .pointer("/params/update")
                            .cloned()
                            .unwrap_or(Value::Null);
                        let _ = tx.send(Incoming::Update(update));
                    }
                    (None, Some(id)) => {
                        if let Some(waiter) = pending.lock().expect("not poisoned").remove(&id) {
                            let reply = match msg.get("error") {
                                Some(e) => Err(e
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("error")
                                    .to_string()),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = waiter.send(reply);
                        }
                    }
                    _ => {}
                }
            }
            for (_, waiter) in pending.lock().expect("not poisoned").drain() {
                let _ = waiter.send(Err("the agent process ended".into()));
            }
        });
        rx
    }

    /// The model ids the agent offers, read from a throw-away session.
    pub async fn list_models(&self, cwd: &Path) -> Vec<String> {
        let Ok(mut child) = self.spawn(cwd) else {
            return vec![];
        };
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let _incoming = Self::read_loop(&mut child, Arc::clone(&pending));
        let mut rpc = Rpc {
            stdin: child.stdin.take().expect("stdin is piped"),
            next: 0,
            pending,
        };
        let probe = async {
            rpc.call(
                "initialize",
                json!({"protocolVersion": 1, "clientCapabilities": {}}),
            )
            .await?;
            let made = rpc
                .call("session/new", json!({"cwd": cwd, "mcpServers": []}))
                .await?;
            Ok::<_, String>(
                made.pointer("/models/availableModels")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                m.get("modelId").and_then(Value::as_str).map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        };
        let out = tokio::time::timeout(Duration::from_secs(60), probe)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let _ = child.kill().await;
        out
    }

    pub async fn turn(&self, t: &Turn) -> RunSummary {
        let mut child = match self.spawn(&t.workdir) {
            Ok(c) => c,
            Err(e) => {
                return RunSummary {
                    stopped: "error".into(),
                    error: format!("cannot start the agent: {e}"),
                    ..Default::default()
                }
            }
        };
        let result = tokio::time::timeout(self.timeout, self.conversation(&mut child, t)).await;
        let _ = child.kill().await;
        match result {
            Ok(s) => s,
            Err(_) => {
                t.append(EventKind::Checkpoint, json!({"reason": "harness_timeout"}));
                RunSummary {
                    stopped: "error".into(),
                    error: format!(
                        "the agent did not finish within {}s",
                        self.timeout.as_secs()
                    ),
                    ..Default::default()
                }
            }
        }
    }

    async fn conversation(&self, child: &mut Child, t: &Turn) -> RunSummary {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let mut incoming = Self::read_loop(child, Arc::clone(&pending));
        let mut rpc = Rpc {
            stdin: child.stdin.take().expect("stdin is piped"),
            next: 0,
            pending,
        };
        let mut summary = RunSummary {
            stopped: "done".into(),
            ..Default::default()
        };
        let setup = async {
            let init = rpc
                .call(
                    "initialize",
                    json!({"protocolVersion": 1, "clientCapabilities": {}}),
                )
                .await?;
            let caps = init
                .pointer("/agentCapabilities/promptCapabilities")
                .cloned()
                .unwrap_or(Value::Null);
            let (sid, primed) = self.session(&mut rpc, t).await?;
            let mut known = self
                .models_set
                .lock()
                .expect("not poisoned")
                .get(&sid)
                .cloned();
            if !t.model.is_empty() && known.as_deref() != Some(t.model.as_str()) {
                match rpc
                    .call(
                        "session/set_model",
                        json!({"sessionId": sid, "modelId": t.model}),
                    )
                    .await
                {
                    Ok(_) => known = Some(t.model.clone()),
                    Err(e) => {
                        t.append(
                            EventKind::Checkpoint,
                            json!({"reason": "model_refused", "detail": e}),
                        );
                    }
                }
                if let Some(m) = known {
                    self.models_set
                        .lock()
                        .expect("not poisoned")
                        .insert(sid.clone(), m);
                }
            }
            Ok::<_, String>((sid, primed, caps))
        };
        let (sid, primed, caps) = match setup.await {
            Ok(x) => x,
            Err(e) => {
                t.append(
                    EventKind::Checkpoint,
                    json!({"reason": "harness_error", "detail": e}),
                );
                return RunSummary {
                    stopped: "error".into(),
                    error: e,
                    ..Default::default()
                };
            }
        };
        let blocks = self.prompt_blocks(t, &primed, &caps);
        t.phase("inference");
        let mut answer = match rpc
            .request(
                "session/prompt",
                json!({"sessionId": sid, "prompt": blocks}),
            )
            .await
        {
            Ok(rx) => rx,
            Err(e) => {
                return RunSummary {
                    stopped: "error".into(),
                    error: e,
                    ..Default::default()
                }
            }
        };
        let mut text = String::new();
        let mut open_calls: HashMap<String, bool> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        let mut cancelled = false;
        let mut grace = 0u32;
        loop {
            tokio::select! {
                biased;
                Some(msg) = incoming.recv() => match msg {
                    Incoming::Update(u) => self.on_update(t, &u, &mut text, &mut open_calls, &mut summary),
                    Incoming::Request(id, method, params) => {
                        let reply = if method == "session/request_permission" {
                            let title = params.pointer("/toolCall/title").and_then(Value::as_str).unwrap_or("an action").to_string();
                            let allowed = t.runner.approve(&t.agent, &title).await;
                            let options: HashMap<String, String> = params.get("options").and_then(Value::as_array).map(|a| a.iter().filter_map(|o| Some((o.get("kind")?.as_str()?.to_string(), o.get("optionId")?.as_str()?.to_string()))).collect()).unwrap_or_default();
                            let outcome = if allowed {
                                json!({"outcome": "selected", "optionId": options.get("allow_once").cloned().unwrap_or_else(|| "allow".into())})
                            } else if let Some(deny) = options.get("reject_once") {
                                json!({"outcome": "selected", "optionId": deny})
                            } else {
                                json!({"outcome": "cancelled"})
                            };
                            json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unsupported: {method}")}})
                        };
                        let _ = rpc.send(&reply).await;
                    }
                },
                reply = &mut answer => {
                    match reply {
                        Ok(Ok(r)) if !cancelled => {
                            let stop = r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn");
                            summary.stopped = if matches!(stop, "end_turn" | "max_tokens") { "done".into() } else { stop.into() };
                        }
                        Ok(Err(e)) => { summary.stopped = "error".into(); summary.error = e; }
                        Err(_) => { summary.stopped = "error".into(); summary.error = "the agent process ended".into(); }
                        _ => {}
                    }
                    break;
                }
                _ = tick.tick() => {
                    if cancelled {
                        grace += 1;
                        if grace > 10 {
                            break; // the agent ignores the cancel: stop waiting, the process is killed
                        }
                    } else if !t.running() {
                        // a teleport froze the agent: stop the agent at once
                        let _ = rpc.send(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}})).await;
                        cancelled = true;
                        summary.stopped = "frozen".into();
                    }
                }
            }
        }
        while let Ok(Incoming::Update(u)) = incoming.try_recv() {
            self.on_update(t, &u, &mut text, &mut open_calls, &mut summary);
        }
        self.flush(t, &mut text, &mut summary);
        summary
    }

    fn flush(&self, t: &Turn, text: &mut String, summary: &mut RunSummary) {
        let body = text.trim().to_string();
        text.clear();
        if !body.is_empty() {
            if let Some(e) = t.append(EventKind::AssistantMessage, json!({"text": body})) {
                t.emit(&e);
            }
            summary.final_text = body;
        }
    }

    fn on_update(
        &self,
        t: &Turn,
        u: &Value,
        text: &mut String,
        open: &mut HashMap<String, bool>,
        summary: &mut RunSummary,
    ) {
        match u.get("sessionUpdate").and_then(Value::as_str) {
            Some("agent_message_chunk") => {
                text.push_str(&content_text(u.get("content").unwrap_or(&Value::Null)));
                t.phase("inference");
            }
            Some("tool_call") => {
                self.flush(t, text, summary);
                let call_id = u
                    .get("toolCallId")
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();
                let name = u
                    .get("title")
                    .or_else(|| u.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let data = json!({"call_id": call_id, "name": name, "arguments": {"kind": u.get("kind"), "input": u.get("rawInput")}});
                if let Some(e) = t.append(EventKind::ToolCall, data) {
                    t.emit(&e);
                }
                open.insert(call_id, true);
            }
            Some("tool_call_update") => {
                let status = u.get("status").and_then(Value::as_str).unwrap_or("");
                if status != "completed" && status != "failed" {
                    return;
                }
                let call_id = u
                    .get("toolCallId")
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();
                if open.remove(&call_id).is_some() {
                    let mut output = content_text(u.get("content").unwrap_or(&Value::Null));
                    if output.is_empty() {
                        output = u
                            .get("rawOutput")
                            .map(|v| {
                                v.as_str()
                                    .map(String::from)
                                    .unwrap_or_else(|| v.to_string())
                            })
                            .unwrap_or_default();
                    }
                    let output: String = output.chars().take(20_000).collect();
                    let data = json!({"call_id": call_id, "output": output, "is_error": status == "failed"});
                    if let Some(e) = t.append(EventKind::ToolResult, data) {
                        t.emit(&e);
                    }
                }
            }
            _ => {} // thoughts and plans are the agent's own: not journaled
        }
    }

    /// The prompt: optional priming, the user's text with the attachments named, then
    /// the attachments themselves in the richest form the agent accepts.
    fn prompt_blocks(&self, t: &Turn, primed: &str, caps: &Value) -> Vec<Value> {
        let mut text = if primed.is_empty() {
            t.text.clone()
        } else {
            format!("{primed}\n\n{}", t.text)
        };
        if !t.attachments.is_empty() {
            text.push_str("\n\nAttached files (in your working directory):");
            for a in &t.attachments {
                text.push_str(&format!("\n- {} ({}, {} bytes)", a.path, a.mime, a.size));
            }
        }
        let mut blocks = vec![json!({"type": "text", "text": text})];
        let images = caps.get("image").and_then(Value::as_bool).unwrap_or(false);
        for a in &t.attachments {
            let path = t.workdir.join(safe_relative(&a.path).unwrap_or_default());
            if images && is_image(a.mime) {
                if let Ok(data) = std::fs::read(&path) {
                    blocks.push(json!({"type": "image", "mimeType": a.mime,
                                       "data": base64::engine::general_purpose::STANDARD.encode(data)}));
                    continue;
                }
            }
            blocks.push(
                json!({"type": "resource_link", "uri": format!("file://{}", path.display()),
                               "name": a.path, "mimeType": a.mime}),
            );
        }
        blocks
    }

    /// Reuse this node's session for the thread when the agent still has it; otherwise a
    /// fresh session primed with the journal so far (the thread came from elsewhere).
    async fn session(&self, rpc: &mut Rpc, t: &Turn) -> Result<(String, String), String> {
        if let Some(sid) = self.sessions().get(&t.agent).and_then(Value::as_str) {
            if rpc
                .call(
                    "session/load",
                    json!({"sessionId": sid, "cwd": t.workdir, "mcpServers": []}),
                )
                .await
                .is_ok()
            {
                return Ok((sid.to_string(), String::new()));
            }
        }
        let made = rpc
            .call("session/new", json!({"cwd": t.workdir, "mcpServers": []}))
            .await?;
        let sid = made
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("the agent gave no session id")?
            .to_string();
        self.remember(&t.agent, &sid);
        let capsule = io::load(&t.path()).map_err(|e| e.to_string())?;
        let mut events: Vec<&TimelineEvent> = capsule
            .timeline
            .ordered()
            .into_iter()
            .filter(|e| e.kind != EventKind::Checkpoint)
            .collect();
        if events
            .last()
            .is_some_and(|e| e.kind == EventKind::UserMessage)
        {
            events.pop(); // this turn's message is sent as the prompt
        }
        if events.iter().all(|e| e.kind == EventKind::System) {
            return Ok((sid, String::new()));
        }
        let mut lines: Vec<String> = Vec::new();
        let mut used = 0;
        for e in events.iter().rev() {
            let s: String = summary(e).chars().take(500).collect();
            if s.is_empty() {
                continue;
            }
            let line = format!("[{}] {s}", e.kind.as_str());
            if used + line.len() > self.prime_chars {
                lines.push(format!(
                    "[…] {} earlier events omitted",
                    events.len() - lines.len()
                ));
                break;
            }
            used += line.len();
            lines.push(line);
        }
        lines.reverse();
        Ok((
            sid,
            format!(
            "You are continuing a conversation that started elsewhere (another machine or model). \
             Earlier journal, oldest first:\n{}\n\nNew message from the user:",
            lines.join("\n")
        ),
        ))
    }
}

/// Drives the agents a node holds: one turn at a time per agent.
pub struct Runner {
    pub host: Arc<CapsuleHost>,
    pub harnesses: BTreeMap<String, AcpHarness>,
    pub default_harness: String,
    publish: Option<Publisher>,
    approver: Option<Approver>,
    pub phases: Mutex<HashMap<String, String>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Runner {
    pub fn new(
        host: Arc<CapsuleHost>,
        harnesses: Vec<(String, AcpHarness)>,
        publish: Option<Publisher>,
        approver: Option<Approver>,
    ) -> Arc<Self> {
        let default_harness = harnesses.first().map(|h| h.0.clone()).unwrap_or_default();
        Arc::new(Self {
            host,
            harnesses: harnesses.into_iter().collect(),
            default_harness,
            publish,
            approver,
            phases: Mutex::default(),
            locks: Mutex::default(),
        })
    }

    fn lock(&self, agent: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.locks
                .lock()
                .expect("not poisoned")
                .entry(agent.into())
                .or_default(),
        )
    }

    fn emit(&self, agent: &str, e: &TimelineEvent) {
        if let Some(p) = &self.publish {
            let text: String = summary(e).chars().take(400).collect();
            p(
                &format!("agent.{agent}.event"),
                json!({"agent": agent, "id": e.id, "kind": e.kind.as_str(), "text": text}),
            );
        }
    }

    fn set_phase(&self, agent: &str, phase: &str) {
        self.phases
            .lock()
            .expect("not poisoned")
            .insert(agent.into(), phase.into());
        if let Some(p) = &self.publish {
            p(
                &format!("agent.{agent}.state"),
                json!({"agent": agent, "node": self.host.node_id, "phase": phase}),
            );
        }
    }

    async fn approve(&self, agent: &str, command: &str) -> bool {
        match &self.approver {
            Some(a) => a(agent.into(), command.into()).await,
            None => false, // no approver connected: no
        }
    }

    fn labels(&self, agent: &str) -> Map<String, Value> {
        io::load(&self.host.agents.join(agent))
            .map(|c| c.manifest.labels())
            .unwrap_or_default()
    }

    /// A new agent held by this node, with its harness; returns its id (the thread id).
    pub fn create(&self, thread: &str, harness: &str, system: &str) -> Result<String, String> {
        let harness = if harness.is_empty() {
            self.default_harness.clone()
        } else {
            harness.to_string()
        };
        if !self.harnesses.contains_key(&harness) {
            return Err(format!("no harness {harness:?} on this node"));
        }
        let staging = self.host.root.join("new").join(thread);
        let _ = std::fs::remove_dir_all(&staging);
        let mut t = crate::capsule::Timeline::new();
        t.append(
            &format!("{}@{thread}", self.host.node_id),
            EventKind::System,
            json!({"text": system}),
            now(),
        );
        let made = io::create(
            &staging,
            thread,
            &crate::capsule::fork::uuid4(),
            Some(&self.host.node_id),
            &t,
            now(),
        )
        .map_err(|e| e.to_string())?;
        let mut manifest = made.manifest;
        manifest.set_label("harness", &harness);
        io::seal(&staging, manifest, &made.timeline).map_err(|e| e.to_string())?;
        let id = self.host.adopt(&staging).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&staging);
        Ok(id)
    }

    /// Put a file the person sent into the agent's workspace (waits for a turn in progress).
    pub async fn attach(&self, agent: &str, rel: &str, data: &[u8]) -> Result<Attached, String> {
        let lock = self.lock(agent);
        let _guard = lock.lock().await;
        if self.host.status(agent) != "running" {
            return Err("this node does not run the agent".into());
        }
        attach_file(&self.host.agents.join(agent), rel, data).map_err(|e| e.to_string())
    }

    /// Choose the harness or the model of a thread; journaled when it changes.
    pub fn set_choice(
        &self,
        agent: &str,
        harness: Option<&str>,
        model: Option<&str>,
    ) -> Result<bool, String> {
        let path = self.host.agents.join(agent);
        let capsule = io::load(&path).map_err(|e| e.to_string())?;
        let mut manifest = capsule.manifest.clone();
        let labels = manifest.labels();
        let mut changed = false;
        if let Some(h) = harness {
            if !self.harnesses.contains_key(h) {
                return Err(format!("no harness {h:?} on this node"));
            }
            if labels.get("harness").and_then(Value::as_str) != Some(h) {
                manifest.set_label("harness", h);
                manifest.set_label("model", "");
                changed = true;
            }
        }
        if let Some(m) = model {
            if labels.get("model").and_then(Value::as_str) != Some(m) {
                manifest.set_label("model", m);
                changed = true;
            }
        }
        if changed {
            io::seal(&path, manifest.clone(), &capsule.timeline).map_err(|e| e.to_string())?;
            let to = format!(
                "{}:{}",
                manifest
                    .labels()
                    .get("harness")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                manifest
                    .labels()
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
            let c = io::append_event(
                &path,
                &format!("{}@{agent}", self.host.node_id),
                EventKind::ProviderSwitch,
                json!({"to": to}),
                now(),
            )
            .map_err(|e| e.to_string())?;
            if let Some(e) = c.timeline.ordered().last() {
                self.emit(agent, e);
            }
        }
        Ok(changed)
    }

    /// One user turn: journal the message (with its attachments), restore the
    /// workspace, let the thread's harness work, keep the files.
    pub async fn run(
        self: &Arc<Self>,
        agent: &str,
        text: &str,
        attachments: Vec<Attached>,
    ) -> RunSummary {
        let lock = self.lock(agent);
        let _guard = lock.lock().await;
        if self.host.status(agent) != "running" {
            return RunSummary {
                stopped: "frozen".into(),
                error: "this node does not run the agent".into(),
                ..Default::default()
            };
        }
        let labels = self.labels(agent);
        let harness_name = labels
            .get("harness")
            .and_then(Value::as_str)
            .filter(|h| self.harnesses.contains_key(*h))
            .unwrap_or(&self.default_harness)
            .to_string();
        let model = labels
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let path = self.host.agents.join(agent);
        let workdir = self.host.root.join("work").join(agent);
        let _ = std::fs::remove_dir_all(&workdir);
        self.set_phase(agent, "listening");
        let files: Vec<Value> = attachments
            .iter()
            .map(|a| json!({"path": a.path, "mime": a.mime, "size": a.size}))
            .collect();
        let mut data = json!({"text": text});
        if !files.is_empty() {
            data["attachments"] = Value::Array(files);
        }
        if let Ok(c) = io::append_event(
            &path,
            &format!("{}@{agent}", self.host.node_id),
            EventKind::UserMessage,
            data,
            now(),
        ) {
            if let Some(e) = c.timeline.ordered().last() {
                self.emit(agent, e);
            }
        }
        let summary = match (
            io::materialize(&path, &workdir),
            self.harnesses.get(&harness_name),
        ) {
            (Ok(()), Some(h)) => {
                let turn = Turn {
                    runner: Arc::clone(self),
                    agent: agent.into(),
                    text: text.into(),
                    attachments,
                    workdir: workdir.clone(),
                    model,
                };
                h.turn(&turn).await
            }
            (Err(e), _) => RunSummary {
                stopped: "error".into(),
                error: e.to_string(),
                ..Default::default()
            },
            (_, None) => RunSummary {
                stopped: "error".into(),
                error: format!("no harness {harness_name:?}"),
                ..Default::default()
            },
        };
        if let Err(e) = io::update_snapshot(&path, &workdir) {
            eprintln!("snapshot of {agent} failed: {e}");
        }
        let _ = std::fs::remove_dir_all(&workdir);
        self.set_phase(
            agent,
            if summary.stopped == "error" {
                "error"
            } else {
                "success"
            },
        );
        self.set_phase(agent, "idle");
        summary
    }
}

pub fn attached_from(path: &str, data: &[u8]) -> Attached {
    Attached {
        path: path.into(),
        sha256: crate::canon::sha256_hex(data),
        size: data.len() as u64,
        mime: sniff(data),
    }
}
