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

pub const DEFAULT_SYSTEM: &str =
    "You are an agent working in a sandboxed workspace. Use the tools to inspect and change \
                                  files. Be concise; say when you are done.";

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
/// What a page shows of an event: a message whole (up to a generous bound), anything else
/// shortened. Cutting messages to a summary's length left the agent's sentences unfinished.
pub fn page_text(e: &TimelineEvent) -> String {
    let limit = match e.kind {
        EventKind::UserMessage | EventKind::AssistantMessage => 200_000,
        _ => 4_000,
    };
    summary(e).chars().take(limit).collect()
}

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
        EventKind::Checkpoint => {
            let reason = d.get("reason").and_then(Value::as_str).unwrap_or("");
            match d
                .get("detail")
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
            {
                Some(detail) => format!("{reason}: {detail}"),
                None => reason.to_string(),
            }
        }
        _ => ["text", "name", "output"]
            .iter()
            .filter_map(|k| d.get(*k).and_then(Value::as_str).filter(|x| !x.is_empty()))
            .collect::<Vec<_>>()
            .join(" "),
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
    /// The mode chosen for the thread (plan, auto-edit…); empty: the agent's own default.
    pub mode: String,
    /// The pass this thread runs under (empty: the owner's own thread).
    pub pass: String,
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

/// Stop an agent process and everything it started (its process group): an ACP agent
/// often runs the real CLI as a child, which would outlive a plain kill.
async fn end(child: &mut Child) {
    if let Some(pid) = child.id() {
        // SAFETY: signalling the process group spawn() created for this child.
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    let _ = child.kill().await;
}

/// What an agent announces about itself.
#[derive(Default, Clone)]
pub struct Probe {
    pub models: Vec<String>,
    pub modes: Vec<Value>,
    pub commands: Vec<Value>,
}

/// The named string fields of a JSON object, as a new object (None when it has no first field).
fn pick(v: &Value, keys: &[&str]) -> Option<Value> {
    v.get(keys[0])?;
    let mut o = Map::new();
    for k in keys {
        o.insert(
            (*k).into(),
            json!(v.get(*k).and_then(Value::as_str).unwrap_or("")),
        );
    }
    Some(Value::Object(o))
}

pub struct AcpHarness {
    pub command: Vec<String>,
    sessions_file: PathBuf,
    pub timeout: Duration,
    pub prime_chars: usize,
    pub extra_env: BTreeMap<String, String>,
    /// Run before a turn whose thread chose a model, with `{model}` replaced by its id:
    /// what makes the model ready (load it on a GPU, say). A non-zero exit ends the turn.
    pub prepare: Vec<String>,
    pub prepare_timeout: Duration,
    /// Environment of a thread that runs under a pass, replacing the owner's (`{pass}` is the
    /// pass id): a HOME of its own, so the agent reads none of the owner's instructions,
    /// tools or credentials. Empty: pass threads share the owner's environment.
    pub pass_env: BTreeMap<String, String>,
    models_set: Mutex<HashMap<String, String>>,
    modes_set: Mutex<HashMap<String, String>>,
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
            prepare: Vec::new(),
            prepare_timeout: Duration::from_secs(600),
            pass_env: BTreeMap::new(),
            models_set: Mutex::default(),
            modes_set: Mutex::default(),
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

    fn spawn(&self, cwd: &Path, pass: &str) -> std::io::Result<Child> {
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..])
            .current_dir(cwd)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0) // its own group: ending it ends whatever it started too
            .kill_on_drop(true);
        for k in ENV_KEEP {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        cmd.envs(&self.extra_env);
        if !pass.is_empty() {
            for (k, v) in &self.pass_env {
                let v = v.replace("{pass}", pass);
                if k == "HOME" || k.starts_with("XDG_") {
                    let _ = std::fs::create_dir_all(&v);
                }
                cmd.env(k, v);
            }
        }
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

    /// What the agent offers, read from a throw-away session: its models, its modes (plan,
    /// auto-edit…) and the slash commands it announces.
    pub async fn probe(&self, cwd: &Path) -> Probe {
        let Ok(mut child) = self.spawn(cwd, "") else {
            return Probe::default();
        };
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let mut incoming = Self::read_loop(&mut child, Arc::clone(&pending));
        let mut rpc = Rpc {
            stdin: child.stdin.take().expect("stdin is piped"),
            next: 0,
            pending,
        };
        let session = async {
            rpc.call(
                "initialize",
                json!({"protocolVersion": 1, "clientCapabilities": {}}),
            )
            .await?;
            rpc.call("session/new", json!({"cwd": cwd, "mcpServers": []}))
                .await
        };
        let mut out = Probe::default();
        if let Ok(Ok(made)) = tokio::time::timeout(Duration::from_secs(60), session).await {
            out.models = made
                .pointer("/models/availableModels")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.get("modelId").and_then(Value::as_str).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            out.modes = made
                .pointer("/modes/availableModes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| pick(m, &["id", "name", "description"]))
                        .collect()
                })
                .unwrap_or_default();
            // the commands arrive as an update just after the session exists
            let _ = tokio::time::timeout(Duration::from_secs(4), async {
                while let Some(msg) = incoming.recv().await {
                    if let Incoming::Update(u) = msg {
                        if u.get("sessionUpdate").and_then(Value::as_str)
                            == Some("available_commands_update")
                        {
                            out.commands = u
                                .get("availableCommands")
                                .and_then(Value::as_array)
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|c| pick(c, &["name", "description"]))
                                        .collect()
                                })
                                .unwrap_or_default();
                            break;
                        }
                    }
                }
            })
            .await;
        }
        end(&mut child).await;
        out
    }

    /// The model ids the agent offers.
    pub async fn list_models(&self, cwd: &Path) -> Vec<String> {
        self.probe(cwd).await.models
    }

    /// Make the chosen model ready before the agent asks it anything.
    async fn make_ready(&self, t: &Turn) -> Result<(), String> {
        if self.prepare.is_empty() || t.model.is_empty() {
            return Ok(());
        }
        t.phase("loading");
        let args: Vec<String> = self
            .prepare
            .iter()
            .map(|a| a.replace("{model}", &t.model))
            .collect();
        let mut cmd = Command::new(&args[0]);
        cmd.args(&args[1..])
            .current_dir(&t.workdir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        for k in ENV_KEEP {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        cmd.envs(&self.extra_env);
        let child = cmd
            .spawn()
            .map_err(|e| format!("cannot start the model preparation: {e}"))?;
        let pid = child.id();
        match tokio::time::timeout(self.prepare_timeout, child.wait_with_output()).await {
            Ok(Ok(out)) if out.status.success() => Ok(()),
            Ok(Ok(out)) => {
                let mut why = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if why.is_empty() {
                    why = String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
                Err(format!(
                    "the model {} could not be made ready: {}",
                    t.model,
                    why.chars().take(400).collect::<String>()
                ))
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => {
                if let Some(pid) = pid {
                    // SAFETY: signalling the process group spawn() created.
                    unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                }
                Err(format!(
                    "the model {} was not ready within {}s",
                    t.model,
                    self.prepare_timeout.as_secs()
                ))
            }
        }
    }

    pub async fn turn(&self, t: &Turn) -> RunSummary {
        if let Err(e) = self.make_ready(t).await {
            t.append(
                EventKind::Checkpoint,
                json!({"reason": "model_unavailable", "detail": e}),
            );
            return RunSummary {
                stopped: "error".into(),
                error: e,
                ..Default::default()
            };
        }
        let mut child = match self.spawn(&t.workdir, &t.pass) {
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
        end(&mut child).await;
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
            let known_mode = self
                .modes_set
                .lock()
                .expect("not poisoned")
                .get(&sid)
                .cloned();
            if !t.mode.is_empty() && known_mode.as_deref() != Some(t.mode.as_str()) {
                match rpc
                    .call(
                        "session/set_mode",
                        json!({"sessionId": sid, "modeId": t.mode}),
                    )
                    .await
                {
                    Ok(_) => {
                        self.modes_set
                            .lock()
                            .expect("not poisoned")
                            .insert(sid.clone(), t.mode.clone());
                    }
                    Err(e) => {
                        t.append(
                            EventKind::Checkpoint,
                            json!({"reason": "mode_refused", "detail": e}),
                        );
                    }
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
                let chunk = content_text(u.get("content").unwrap_or(&Value::Null));
                // Gemini announces a mode change as message text: that is a notice, not its answer
                if chunk.trim_start().starts_with("[MODE_UPDATE]") {
                    return;
                }
                text.push_str(&chunk);
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
            let text = page_text(e);
            p(
                &format!("agent.{agent}.event"),
                json!({"agent": agent, "id": e.id, "kind": e.kind.as_str(), "text": text}),
            );
        }
    }

    fn set_phase(&self, agent: &str, phase: &str) {
        // a thread deleted while a turn was still running must not come back as a phantom
        if matches!(self.host.status(agent).as_str(), "deleted" | "absent") {
            return;
        }
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
    pub fn create(&self, thread: &str, harness: &str, title: &str) -> Result<String, String> {
        let system = DEFAULT_SYSTEM;
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
        if !title.is_empty() {
            manifest.extra.insert("title".into(), json!(title));
        }
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

    fn journal(&self, agent: &str, kind: EventKind, data: Value) -> Result<TimelineEvent, String> {
        let c = io::append_event(
            &self.host.agents.join(agent),
            &format!("{}@{agent}", self.host.node_id),
            kind,
            data,
            now(),
        )
        .map_err(|e| e.to_string())?;
        let e = c
            .timeline
            .ordered()
            .last()
            .cloned()
            .cloned()
            .ok_or("empty journal")?;
        self.emit(agent, &e);
        Ok(e)
    }

    /// Rename a thread (its title in the manifest).
    pub fn set_title(&self, agent: &str, title: &str) -> Result<(), String> {
        io::edit_manifest(&self.host.agents.join(agent), |m| {
            m.extra.insert("title".into(), json!(title));
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Set one label on an agent's capsule (its pass, say).
    pub fn label(&self, agent: &str, key: &str, value: &str) -> Result<(), String> {
        self.relabel(agent, &[(key, value)])
    }

    fn relabel(&self, agent: &str, labels: &[(&str, &str)]) -> Result<(), String> {
        io::edit_manifest(&self.host.agents.join(agent), |m| {
            for (k, v) in labels {
                m.set_label(k, v);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Choose who thinks for a thread; changing harness clears its model. Journaled
    /// as the reference does; false when nothing changed.
    pub fn set_harness(&self, agent: &str, name: &str) -> Result<bool, String> {
        if !self.harnesses.contains_key(name) {
            return Err(format!("no harness {name:?} on this node"));
        }
        if self.labels(agent).get("harness").and_then(Value::as_str) == Some(name) {
            return Ok(false);
        }
        self.relabel(agent, &[("harness", name), ("model", ""), ("mode", "")])?;
        self.journal(
            agent,
            EventKind::ProviderSwitch,
            json!({"from": "", "to": format!("{name}:default"), "harness": name}),
        )?;
        Ok(true)
    }

    /// Remember the mode chosen for a thread (empty: the agent's default); false when unchanged.
    pub fn set_mode(&self, agent: &str, mode: &str) -> Result<bool, String> {
        let previous = self
            .labels(agent)
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if previous == mode {
            return Ok(false);
        }
        self.relabel(agent, &[("mode", mode)])?;
        self.journal(
            agent,
            EventKind::Checkpoint,
            json!({"reason": "mode_changed", "detail": if mode.is_empty() { "default" } else { mode }}),
        )?;
        Ok(true)
    }

    /// Remember the model chosen for a thread; false when nothing changed.
    pub fn set_model(&self, agent: &str, model: &str) -> Result<bool, String> {
        let previous = self
            .labels(agent)
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if previous == model {
            return Ok(false);
        }
        self.relabel(agent, &[("model", model)])?;
        self.journal(
            agent,
            EventKind::ProviderSwitch,
            json!({"from": previous, "to": model, "text": format!("model: {model}")}),
        )?;
        Ok(true)
    }

    /// A command the person typed, run in the agent's workspace with a scrubbed
    /// environment; journaled as a tool call so the agent knows, files kept.
    pub async fn exec_user(&self, agent: &str, cmd: &str, timeout: Duration) -> Value {
        let lock = self.lock(agent);
        let _guard = lock.lock().await;
        if self.host.status(agent) != "running" {
            return json!({"ok": false, "output": "this node does not run the agent", "code": -1});
        }
        let path = self.host.agents.join(agent);
        let workdir = self.host.root.join("work").join(agent);
        let _ = std::fs::remove_dir_all(&workdir);
        if let Err(e) = io::materialize(&path, &workdir) {
            return json!({"ok": false, "output": e.to_string(), "code": -1});
        }
        let call_id = crate::wire::random_hex(6);
        let args = json!({"cmd": cmd, "by": "user"});
        let _ = self.journal(
            agent,
            EventKind::ToolCall,
            json!({"call_id": call_id, "name": "bash", "arguments": args}),
        );
        let (code, output, timed_out) = run_scrubbed(&workdir, cmd, timeout).await;
        let mut output: String = output.chars().take(20_000).collect();
        if output.is_empty() {
            output = "(no output)".into();
        }
        let _ = self.journal(
            agent,
            EventKind::ToolResult,
            json!({"call_id": call_id, "output": output, "is_error": code != 0}),
        );
        if let Err(e) = io::update_snapshot(&path, &workdir) {
            eprintln!("snapshot of {agent} failed: {e}");
        }
        let _ = std::fs::remove_dir_all(&workdir);
        json!({"ok": code == 0, "output": output, "code": code, "timed_out": timed_out})
    }

    /// The attachments of a message, from the paths already in the workspace.
    pub fn attached(&self, agent: &str, paths: &[String]) -> Vec<Attached> {
        let Ok(c) = io::load(&self.host.agents.join(agent)) else {
            return vec![];
        };
        paths
            .iter()
            .filter_map(|p| {
                let sha = c.snapshot.tree.get(p)?;
                let data =
                    std::fs::read(self.host.agents.join(agent).join(io::BLOBS).join(sha)).ok()?;
                Some(attached_from(p, &data))
            })
            .collect()
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
                    mode: labels
                        .get("mode")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    pass: labels
                        .get("pass")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
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

const SAFE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// `sh -c cmd` in `cwd` with nothing inherited but a safe PATH; the whole process group
/// is killed at the deadline, so no child outlives it. Returns code, output, timed out.
pub async fn run_scrubbed(cwd: &Path, cmd: &str, timeout: Duration) -> (i32, String, bool) {
    let child = Command::new("/bin/sh")
        .args(["-c", cmd])
        .current_dir(cwd)
        .env_clear()
        .env("PATH", SAFE_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => return (-1, format!("cannot start the shell: {e}"), false),
    };
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut text = String::from_utf8_lossy(&out.stdout[..out.stdout.len().min(1_000_000)])
                .into_owned();
            text.push_str(&String::from_utf8_lossy(
                &out.stderr[..out.stderr.len().min(1_000_000)],
            ));
            (out.status.code().unwrap_or(-1), text, false)
        }
        Ok(Err(e)) => (-1, e.to_string(), false),
        Err(_) => {
            if let Some(pid) = pid {
                // SAFETY: signalling the process group this function created.
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
            (-1, format!("timed out after {}s", timeout.as_secs()), true)
        }
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
