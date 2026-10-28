//! OpenFox, driven natively: the core speaks OpenFox's own HTTP API and WebSocket, the ones its
//! web interface uses, instead of going through an ACP bridge.
//!
//! A thread is one OpenFox session in the project whose folder is the thread's workspace. A turn
//! posts the message and follows the session's events until OpenFox says it stopped running;
//! the events become the same journal and drafts as an ACP agent's (`acp_update`), so the page
//! needs nothing new for them. What makes it lighter than the bridge it replaces:
//!
//! - no process per message: one WebSocket per thread, opened once and kept between turns;
//! - the end of a turn is never missed: a quiet session is asked how it is (REST) every few
//!   seconds, and a dropped WebSocket is opened again, the session's state read back;
//! - a message sent while OpenFox works goes to OpenFox's own queue, which slips it in at its
//!   next step (`asap`): its own way of working, nothing cancelled (`join`).
//!
//! OpenFox's messages are read into types (`proto`): a field OpenFox renamed shows as an error
//! in the logs rather than as silence. Provider records are read without their API keys.
//!
//! Every setting lives in the node's data folder and is changed from the interface (`Settings`);
//! a node that never set them uses `openfox-acp`'s (`~/.config/openfox-acp/{url,token}`).

use crate::agent::{acp_update, flush_answer, prepare_model, primer, Probe, RunSummary, Turn};
use crate::capsule::EventKind;
use crate::usage::http_with;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// OpenFox's two top-level agents: "builder" reads, writes and runs commands; "planner" only
/// reads and plans. A new OpenFox session starts in planner; a thread is there to act.
pub const MODES: [(&str, &str, &str); 2] = [
    (
        "builder",
        "Build",
        "OpenFox reads, writes and runs commands.",
    ),
    ("planner", "Plan", "OpenFox only reads and plans."),
];
/// How long OpenFox may stay silent before the core asks it how the session is.
const QUIET: Duration = Duration::from_secs(8);
/// How long a stopped session may take to say so before the turn ends anyway.
const STOP_GRACE: Duration = Duration::from_secs(20);
/// A message posted while OpenFox works that it never takes is given up after this.
const QUEUED_PATIENCE: Duration = Duration::from_secs(120);
/// A WebSocket unused this long is closed.
const LINK_IDLE: Duration = Duration::from_secs(15 * 60);
/// Each REST call's limit.
const CALL: Duration = Duration::from_secs(60);

// ── settings ────────────────────────────────────────────────────────────────

/// What the interface sets for a node's OpenFox (`<data_dir>/openfox/settings.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// OpenFox's address, http:// (it serves on the machine itself, 127.0.0.1:10369 by default).
    pub url: String,
    /// The OpenFox agent a new thread starts with (`MODES`).
    pub mode: String,
    /// How long a turn may run, in seconds.
    pub turn_timeout: u64,
}

fn openfox_acp_dir() -> PathBuf {
    std::env::var_os("OPENFOX_ACP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config")
                .join("openfox-acp")
        })
}

impl Default for Settings {
    /// What `openfox-acp` used: its address file, else OpenFox's own default.
    fn default() -> Self {
        let url = std::fs::read_to_string(openfox_acp_dir().join("url"))
            .ok()
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:10369".into());
        Self {
            url,
            mode: "builder".into(),
            turn_timeout: 900,
        }
    }
}

impl Settings {
    /// What is wrong with these settings, if anything.
    pub fn problem(&self) -> Option<String> {
        if !self.url.starts_with("http://") {
            return Some("OpenFox's address must start with http://".into());
        }
        if !MODES.iter().any(|(id, _, _)| *id == self.mode) {
            return Some(format!("unknown OpenFox agent {:?}", self.mode));
        }
        if !(30..=86_400).contains(&self.turn_timeout) {
            return Some("a turn's limit is between 30 s and 24 h".into());
        }
        None
    }
}

// ── OpenFox's messages, as types ────────────────────────────────────────────

pub mod proto {
    use serde::Deserialize;
    use serde_json::Value;

    /// `GET /api/auth`.
    #[derive(Deserialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct Auth {
        #[serde(default)]
        pub requires_auth: bool,
        #[serde(default)]
        pub has_password: bool,
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct Project {
        pub id: String,
        #[serde(default)]
        pub workdir: String,
    }

    /// `GET /api/projects`: a list, or `{projects: [...]}` depending on the version.
    #[derive(Deserialize, Debug)]
    #[serde(untagged)]
    pub enum Projects {
        Wrapped { projects: Vec<Project> },
        Bare(Vec<Project>),
    }

    impl Projects {
        pub fn list(self) -> Vec<Project> {
            match self {
                Projects::Wrapped { projects } | Projects::Bare(projects) => projects,
            }
        }
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Session {
        pub id: String,
        #[serde(default)]
        pub mode: Option<String>,
        #[serde(default)]
        pub provider_id: Option<String>,
        #[serde(default)]
        pub provider_model: Option<String>,
        #[serde(default)]
        pub is_running: bool,
    }

    impl Session {
        /// The session's model as "<provider>/<model>", when it chose one.
        pub fn model(&self) -> Option<String> {
            Some(format!(
                "{}/{}",
                self.provider_id.as_ref()?,
                self.provider_model.as_ref()?
            ))
        }
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct Message {
        #[serde(default)]
        pub role: String,
        #[serde(default)]
        pub content: String,
    }

    /// `GET /api/sessions/:id`.
    #[derive(Deserialize, Debug)]
    pub struct SessionView {
        pub session: Session,
        #[serde(default)]
        pub messages: Vec<Message>,
    }

    /// `POST /api/sessions` and `POST /api/projects`.
    #[derive(Deserialize, Debug)]
    pub struct MadeSession {
        pub session: Session,
    }
    #[derive(Deserialize, Debug)]
    pub struct MadeProject {
        pub project: Project,
    }

    /// A model a provider offers: an object or a bare name, depending on the provider.
    #[derive(Deserialize, Debug, Clone)]
    #[serde(untagged)]
    pub enum ModelRef {
        Named {
            id: String,
            #[serde(default, rename = "contextWindow")]
            context_window: Option<u64>,
        },
        Bare(String),
    }

    impl ModelRef {
        pub fn id(&self) -> &str {
            match self {
                ModelRef::Named { id, .. } | ModelRef::Bare(id) => id,
            }
        }
    }

    /// A provider as OpenFox lists it, read without its API key or credentials.
    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Provider {
        pub id: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub backend: String,
        #[serde(default)]
        pub is_local: Option<bool>,
        #[serde(default)]
        pub status: Option<String>,
        #[serde(default)]
        pub models: Vec<ModelRef>,
    }

    /// `GET /api/providers`.
    #[derive(Deserialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct Providers {
        #[serde(default)]
        pub providers: Vec<Provider>,
        #[serde(default)]
        pub active_provider_id: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    pub struct Login {
        pub token: String,
    }

    // the WebSocket's events, as `{type, payload, sessionId?, id?}`

    #[derive(Deserialize, Debug)]
    pub struct Delta {
        #[serde(default)]
        pub content: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct ToolCall {
        pub call_id: String,
        #[serde(default)]
        pub tool: String,
        #[serde(default)]
        pub args: Value,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct ToolResult {
        pub call_id: String,
        #[serde(default)]
        pub result: Value,
    }

    /// A path or command OpenFox wants confirmed (`chat.path_confirmation`).
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Confirmation {
        pub call_id: String,
        #[serde(default)]
        pub tool: String,
        #[serde(default)]
        pub paths: Vec<String>,
        #[serde(default)]
        pub reason: String,
        #[serde(default)]
        pub session_id: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    pub struct Choice {
        #[serde(default)]
        pub value: Value,
        #[serde(default)]
        pub label: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct AskUser {
        pub call_id: String,
        #[serde(default)]
        pub question: String,
        #[serde(default, rename = "type")]
        pub kind: Option<String>,
        #[serde(default)]
        pub options: Vec<Choice>,
    }

    #[derive(Deserialize, Debug)]
    pub struct Failure {
        #[serde(default)]
        pub error: Option<Value>,
        #[serde(default)]
        pub message: Option<String>,
        #[serde(default)]
        pub attempts: Option<Value>,
    }

    impl Failure {
        pub fn text(&self) -> String {
            match (&self.error, &self.message) {
                (Some(Value::String(e)), _) => e.clone(),
                (Some(e), _) if !e.is_null() => e.to_string(),
                (_, Some(m)) => m.clone(),
                _ => String::new(),
            }
        }
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Running {
        pub is_running: bool,
    }

    #[derive(Deserialize, Debug)]
    pub struct Said {
        pub message: Message,
    }

    #[derive(Deserialize, Debug)]
    pub struct Done {
        #[serde(default)]
        pub reason: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct ContextState {
        #[serde(default)]
        pub current_tokens: u64,
        #[serde(default)]
        pub max_tokens: u64,
    }

    /// One WebSocket message from OpenFox.
    #[derive(Debug)]
    pub enum Event {
        Delta(Delta),
        ToolCall(ToolCall),
        ToolResult(ToolResult),
        Confirm(Confirmation),
        Ask(AskUser),
        Error(Failure),
        Retrying(Failure),
        RetryFailed(Failure),
        Running(Running),
        Said(Said),
        Done(Done),
        Context(ContextState),
        Ack,
        /// A kind this core does not follow (yet): git status, queue state, workflows…
        Other(String),
        /// A known kind whose payload did not read: OpenFox changed it.
        Malformed(String, String),
    }

    /// An event and the session it belongs to (none: for whoever is subscribed).
    #[derive(Debug)]
    pub struct Frame {
        pub session: Option<String>,
        pub id: Option<String>,
        pub event: Event,
    }

    /// Read one WebSocket message; None when it is not OpenFox's JSON at all.
    pub fn read(raw: &str) -> Option<Frame> {
        let mut v: Value = serde_json::from_str(raw).ok()?;
        let kind = v.get("type")?.as_str()?.to_string();
        let session = v.get("sessionId").and_then(Value::as_str).map(String::from);
        let id = v.get("id").and_then(Value::as_str).map(String::from);
        let payload = v.get_mut("payload").map(Value::take).unwrap_or(Value::Null);
        fn typed<T: serde::de::DeserializeOwned>(
            kind: &str,
            payload: Value,
            f: impl FnOnce(T) -> Event,
        ) -> Event {
            match serde_json::from_value::<T>(payload) {
                Ok(p) => f(p),
                Err(e) => Event::Malformed(kind.to_string(), e.to_string()),
            }
        }
        let event = match kind.as_str() {
            "chat.delta" => typed(&kind, payload, Event::Delta),
            "chat.tool_call" => typed(&kind, payload, Event::ToolCall),
            "chat.tool_result" => typed(&kind, payload, Event::ToolResult),
            "chat.path_confirmation" | "session.confirmation_pending" => {
                typed(&kind, payload, Event::Confirm)
            }
            "chat.ask_user" => typed(&kind, payload, Event::Ask),
            "chat.error" => typed(&kind, payload, Event::Error),
            "chat.llm_retry" => typed(&kind, payload, Event::Retrying),
            "chat.llm_retry_failed" => typed(&kind, payload, Event::RetryFailed),
            "session.running" => typed(&kind, payload, Event::Running),
            "chat.message" => typed(&kind, payload, Event::Said),
            "chat.done" => typed(&kind, payload, Event::Done),
            "context.state" => typed(&kind, payload, Event::Context),
            "ack" => Event::Ack,
            _ => Event::Other(kind.clone()),
        };
        Some(Frame { session, id, event })
    }
}

use proto::Event;

// ── a minimal WebSocket client (RFC 6455): text frames out, OpenFox's events in ──

/// One WebSocket to OpenFox, subscribed to one session. Its events wait in `events`; the
/// reader stops (and `events` ends) when OpenFox closes it.
struct Link {
    events: mpsc::UnboundedReceiver<String>,
    writer: std::sync::Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    reader: tokio::task::JoinHandle<()>,
    used: Instant,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Link {
    fn alive(&self) -> bool {
        !self.reader.is_finished()
    }
}

/// A client frame (masked, as RFC 6455 requires of clients).
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut mask = [0u8; 4];
    getrandom::getrandom(&mut mask).expect("the system has a random source");
    let mut out = vec![0x80 | opcode];
    let n = payload.len();
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n < 65_536 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

async fn exact(r: &mut OwnedReadHalf, buf: &mut Vec<u8>, n: usize) -> Option<Vec<u8>> {
    while buf.len() < n {
        let mut chunk = [0u8; 16_384];
        let got = r.read(&mut chunk).await.ok()?;
        if got == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..got]);
    }
    Some(buf.drain(..n).collect())
}

/// The next whole message: (opcode, payload); None once the connection is closed.
async fn next_message(r: &mut OwnedReadHalf, buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    let mut parts = Vec::new();
    let mut first = 0u8;
    loop {
        let head = exact(r, buf, 2).await?;
        let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0F);
        let mut n = (head[1] & 0x7F) as u64;
        if n == 126 {
            n = u16::from_be_bytes(exact(r, buf, 2).await?.try_into().ok()?) as u64;
        } else if n == 127 {
            n = u64::from_be_bytes(exact(r, buf, 8).await?.try_into().ok()?);
        }
        if n > 64 * 1024 * 1024 {
            return None; // no event of OpenFox's is that large
        }
        let mask = if head[1] & 0x80 != 0 {
            Some(exact(r, buf, 4).await?)
        } else {
            None
        };
        let mut payload = exact(r, buf, n as usize).await?;
        if let Some(m) = mask {
            payload
                .iter_mut()
                .enumerate()
                .for_each(|(i, b)| *b ^= m[i % 4]);
        }
        if opcode >= 0x8 {
            return Some((opcode, payload)); // control frames come whole, between fragments
        }
        if opcode != 0 {
            first = opcode;
        }
        parts.extend_from_slice(&payload);
        if fin {
            return Some((first, parts));
        }
    }
}

/// Open a WebSocket to `url` (http://host:port) with OpenFox's token in the query.
async fn open_socket(url: &str, token: &str) -> Result<Link, String> {
    let authority = url
        .strip_prefix("http://")
        .ok_or("OpenFox's address must start with http://")?
        .split('/')
        .next()
        .unwrap_or_default()
        .to_string();
    let addr = if authority.contains(':') {
        authority.clone()
    } else {
        format!("{authority}:80")
    };
    let query: String = token
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let mut key = [0u8; 16];
    getrandom::getrandom(&mut key).expect("the system has a random source");
    let key = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);
    let connect = async {
        let mut s = TcpStream::connect(&addr).await.map_err(|e| e.to_string())?;
        let path = if token.is_empty() {
            "/".to_string()
        } else {
            format!("/?token={query}")
        };
        s.write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .map_err(|e| e.to_string())?;
        let mut head = Vec::new();
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let mut chunk = [0u8; 4096];
            let n = s.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("OpenFox closed the WebSocket while opening it".to_string());
            }
            head.extend_from_slice(&chunk[..n]);
            if head.len() > 64 * 1024 {
                return Err("OpenFox's WebSocket answer is too long".into());
            }
        }
        let end = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0) + 4;
        let text = String::from_utf8_lossy(&head[..end]).to_lowercase();
        let first = text.lines().next().unwrap_or_default().to_string();
        // The accept key would need SHA-1, which the core does not carry: a 101 with an
        // Upgrade to websocket is the handshake's answer, on the machine's own OpenFox.
        if !first.contains(" 101 ") || !text.contains("upgrade: websocket") {
            return Err(format!("OpenFox refused the WebSocket: {first}"));
        }
        Ok::<_, String>((s, head[end..].to_vec()))
    };
    let (stream, rest) = tokio::time::timeout(Duration::from_secs(15), connect)
        .await
        .map_err(|_| "OpenFox's WebSocket did not open within 15 s".to_string())??;
    let (mut read, write) = stream.into_split();
    let writer = std::sync::Arc::new(tokio::sync::Mutex::new(write));
    let (tx, rx) = mpsc::unbounded_channel();
    let pong = std::sync::Arc::clone(&writer);
    let reader = tokio::spawn(async move {
        let mut buf = rest;
        while let Some((opcode, payload)) = next_message(&mut read, &mut buf).await {
            match opcode {
                0x1 | 0x2 => {
                    if tx
                        .send(String::from_utf8_lossy(&payload).into_owned())
                        .is_err()
                    {
                        return;
                    }
                }
                0x8 => return,
                0x9 => {
                    let _ = pong.lock().await.write_all(&frame(0xA, &payload)).await;
                }
                _ => {}
            }
        }
    });
    Ok(Link {
        events: rx,
        writer,
        reader,
        used: Instant::now(),
    })
}

impl Link {
    async fn send(&self, v: &Value) -> Result<(), String> {
        self.writer
            .lock()
            .await
            .write_all(&frame(0x1, v.to_string().as_bytes()))
            .await
            .map_err(|e| e.to_string())
    }
}

// ── the harness ─────────────────────────────────────────────────────────────

/// A turn in progress, by thread: its session, and the messages posted into it meanwhile that
/// OpenFox has not taken yet (they are journaled when it does).
struct Live {
    session: String,
    posted: Vec<(String, Instant)>,
}

pub struct OpenFoxHarness {
    /// Where the settings, the token and the threads' sessions are kept.
    dir: PathBuf,
    /// Run before a turn whose thread chose a model (as an ACP harness's `prepare`).
    pub prepare: Vec<String>,
    pub prepare_timeout: Duration,
    pub extra_env: BTreeMap<String, String>,
    /// How long OpenFox may stay silent before it is asked how the session is (QUIET).
    pub quiet: Duration,
    settings: Mutex<Settings>,
    links: Mutex<HashMap<String, Link>>,
    live: Mutex<HashMap<String, Live>>,
}

impl OpenFoxHarness {
    /// The harness keeping its settings and sessions in `dir` (the node's `<data_dir>/openfox`).
    pub fn new(dir: PathBuf) -> Self {
        let settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            dir,
            prepare: Vec::new(),
            prepare_timeout: Duration::from_secs(600),
            extra_env: BTreeMap::new(),
            quiet: QUIET,
            settings: Mutex::new(settings),
            links: Mutex::default(),
            live: Mutex::default(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("not poisoned").clone()
    }

    /// Keep new settings (checked first); the WebSockets are opened again with them.
    pub fn set_settings(&self, s: Settings) -> Result<(), String> {
        if let Some(why) = s.problem() {
            return Err(why);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        crate::capsule::io::atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&s).expect("serializes"),
        )
        .map_err(|e| e.to_string())?;
        *self.settings.lock().expect("not poisoned") = s;
        self.links.lock().expect("not poisoned").clear();
        Ok(())
    }

    /// The token this node signs in to OpenFox with: its own, else `openfox-acp`'s.
    fn token(&self) -> String {
        [self.dir.join("token"), openfox_acp_dir().join("token")]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.trim().to_string())
            .unwrap_or_default()
    }

    /// Sign in with OpenFox's password: the token it gives is kept (mode 600), never the password.
    pub async fn login(&self, password: &str) -> Result<(), String> {
        let (status, raw) = self
            .raw(
                "POST",
                "/api/auth/login",
                Some(json!({"password": password})),
                false,
            )
            .await?;
        if status != 200 {
            return Err(format!(
                "OpenFox refused the password (HTTP {status} {})",
                reason(&raw)
            ));
        }
        let login: proto::Login =
            serde_json::from_slice(&raw).map_err(|e| format!("OpenFox's login answer: {e}"))?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join("token");
        crate::capsule::io::atomic_write(&path, format!("{}\n", login.token).as_bytes())
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        self.links.lock().expect("not poisoned").clear();
        Ok(())
    }

    async fn raw(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        signed: bool,
    ) -> Result<(u16, Vec<u8>), String> {
        let url = self.settings().url;
        let token = if signed { self.token() } else { String::new() };
        let headers: Vec<(&str, &str)> = if token.is_empty() {
            Vec::new()
        } else {
            vec![("x-session-token", token.as_str())]
        };
        let bytes = body.map(|b| serde_json::to_vec(&b).expect("serializes"));
        http_with(
            method,
            &format!("{url}{path}"),
            bytes.as_deref(),
            &headers,
            CALL,
        )
        .await
        .map_err(|e| format!("{method} {path}: OpenFox does not answer at {url} ({e})"))
    }

    /// A REST call whose answer reads as `T`.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, String> {
        let (status, raw) = self.raw(method, path, body, true).await?;
        if status >= 400 {
            return Err(format!("{method} {path}: HTTP {status} {}", reason(&raw)));
        }
        serde_json::from_slice(&raw).map_err(|e| format!("{method} {path}: OpenFox's answer: {e}"))
    }

    /// A REST call whose answer does not matter beyond its status.
    async fn act(&self, method: &str, path: &str, body: Value) -> Result<(), String> {
        let (status, raw) = self.raw(method, path, Some(body), true).await?;
        if status >= 400 {
            return Err(format!("{method} {path}: HTTP {status} {}", reason(&raw)));
        }
        Ok(())
    }

    /// The session, or None when OpenFox no longer has it.
    async fn session_view(&self, sid: &str) -> Result<Option<proto::SessionView>, String> {
        let path = format!("/api/sessions/{sid}");
        let (status, raw) = self.raw("GET", &path, None, true).await?;
        match status {
            404 => Ok(None),
            s if s >= 400 => Err(format!("GET {path}: HTTP {s} {}", reason(&raw))),
            _ => serde_json::from_slice(&raw)
                .map(Some)
                .map_err(|e| format!("GET {path}: OpenFox's answer: {e}")),
        }
    }

    /// The providers and their models, as OpenFox has them (never their keys).
    pub async fn providers(&self) -> Result<proto::Providers, String> {
        self.call("GET", "/api/providers", None).await
    }

    /// Make a provider (and one of its models) OpenFox's default, as its own interface does.
    pub async fn activate(&self, provider: &str, model: &str) -> Result<(), String> {
        let body = if model.is_empty() {
            json!({})
        } else {
            json!({"model": model})
        };
        self.act("POST", &format!("/api/providers/{provider}/activate"), body)
            .await
    }

    /// How this node's OpenFox is: for the settings in the interface.
    pub async fn status(&self) -> Value {
        let s = self.settings();
        let mut out = json!({"url": s.url, "mode": s.mode, "turn_timeout": s.turn_timeout,
                             "modes": MODES.iter().map(|(id, name, d)| json!({"id": id, "name": name, "description": d})).collect::<Vec<_>>(),
                             "signed_in": !self.token().is_empty()});
        let auth: Result<proto::Auth, String> = self.call("GET", "/api/auth", None).await;
        match auth {
            Err(e) => {
                out["reachable"] = json!(false);
                out["problem"] = json!(e);
                return out;
            }
            Ok(a) => {
                out["reachable"] = json!(true);
                out["password"] = json!(a.requires_auth);
            }
        }
        match self.providers().await {
            Ok(p) => {
                out["active_provider"] = json!(p.active_provider_id);
                out["providers"] = json!(p
                    .providers
                    .iter()
                    .map(|pr| json!({"id": pr.id, "name": pr.name, "backend": pr.backend,
                                     "local": pr.is_local, "status": pr.status,
                                     "models": pr.models.iter().map(|m| m.id()).collect::<Vec<_>>()}))
                    .collect::<Vec<_>>());
            }
            Err(e) => out["problem"] = json!(e),
        }
        out
    }

    fn sessions(&self) -> serde_json::Map<String, Value> {
        std::fs::read(self.dir.join("sessions.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn remember(&self, agent: &str, sid: &str) {
        let mut all = self.sessions();
        all.insert(agent.into(), json!(sid));
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = crate::capsule::io::atomic_write(
            &self.dir.join("sessions.json"),
            &serde_json::to_vec_pretty(&all).expect("serializes"),
        );
    }

    /// What OpenFox offers: its models ("<provider>/<model>") and its agents.
    pub async fn probe(&self) -> Probe {
        let models = match self.providers().await {
            Ok(p) => p
                .providers
                .iter()
                .flat_map(|pr| {
                    pr.models
                        .iter()
                        .map(move |m| format!("{}/{}", pr.id, m.id()))
                })
                .collect(),
            Err(e) => {
                eprintln!("openfox: {e}");
                Vec::new()
            }
        };
        Probe {
            models,
            modes: MODES
                .iter()
                .map(|(id, name, d)| json!({"id": id, "name": name, "description": d}))
                .collect(),
            commands: Vec::new(),
        }
    }

    /// The OpenFox project whose folder is `workdir`, made when missing.
    async fn project_for(&self, workdir: &Path) -> Result<String, String> {
        let wanted = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
        let projects: proto::Projects = self.call("GET", "/api/projects", None).await?;
        for p in projects.list() {
            let at =
                std::fs::canonicalize(&p.workdir).unwrap_or_else(|_| PathBuf::from(&p.workdir));
            if at == wanted {
                return Ok(p.id);
            }
        }
        let name: String = wanted
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let made: proto::MadeProject = self
            .call(
                "POST",
                "/api/projects",
                Some(json!({"name": if name.is_empty() { "workspace".into() } else { name }, "workdir": wanted})),
            )
            .await?;
        Ok(made.project.id)
    }

    /// The thread's session (made when OpenFox does not have it), and whether it is new.
    async fn session_for(&self, t: &Turn) -> Result<(proto::Session, bool), String> {
        if let Some(sid) = self.sessions().get(&t.agent).and_then(Value::as_str) {
            if let Some(view) = self.session_view(sid).await? {
                return Ok((view.session, false));
            }
        }
        let project = self.project_for(&t.workdir).await?;
        let made: proto::MadeSession = self
            .call(
                "POST",
                "/api/sessions",
                Some(json!({"projectId": project, "title": "Inari"})),
            )
            .await?;
        self.remember(&t.agent, &made.session.id);
        Ok((made.session, true))
    }

    /// The thread's WebSocket: kept from an earlier turn, or opened and subscribed now.
    async fn link_for(&self, sid: &str) -> Result<Link, String> {
        let kept = self.links.lock().expect("not poisoned").remove(sid);
        if let Some(mut link) = kept.filter(Link::alive) {
            while link.events.try_recv().is_ok() {} // what came between turns is not this turn's
            return Ok(link);
        }
        let mut link = open_socket(&self.settings().url, &self.token()).await?;
        let id = format!("load-{}", crate::wire::random_hex(4));
        link.send(&json!({"id": id, "type": "session.load", "payload": {"sessionId": sid}}))
            .await?;
        // subscribed once OpenFox acknowledges: what it says before is not this session's
        let acked = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(raw) = link.events.recv().await {
                if let Some(f) = proto::read(&raw) {
                    if f.id.as_deref() == Some(id.as_str()) {
                        return matches!(f.event, Event::Ack);
                    }
                }
            }
            false
        })
        .await;
        match acked {
            Ok(true) => Ok(link),
            Ok(false) => Err("OpenFox did not take the session's subscription".into()),
            Err(_) => Err("OpenFox did not acknowledge the session within 10 s".into()),
        }
    }

    /// Keep the WebSocket for the next turn, and close those unused for long.
    fn keep(&self, sid: &str, mut link: Link) {
        link.used = Instant::now();
        let mut links = self.links.lock().expect("not poisoned");
        links.retain(|_, l| l.used.elapsed() < LINK_IDLE && l.alive());
        if link.alive() {
            links.insert(sid.to_string(), link);
        }
    }

    /// A message sent while this thread's turn runs: posted to OpenFox's own queue, which slips it
    /// in at its next step. False when no turn runs here (the message then starts one).
    pub async fn join(&self, agent: &str, text: &str) -> bool {
        let sid = {
            let mut live = self.live.lock().expect("not poisoned");
            let Some(l) = live.get_mut(agent) else {
                return false;
            };
            l.posted.push((text.trim().to_string(), Instant::now()));
            l.session.clone()
        };
        match self
            .act(
                "POST",
                &format!("/api/sessions/{sid}/message"),
                json!({"content": text}),
            )
            .await
        {
            Ok(()) => true,
            Err(e) => {
                eprintln!("openfox: a message during a turn: {e}");
                if let Some(l) = self.live.lock().expect("not poisoned").get_mut(agent) {
                    l.posted.retain(|(p, _)| p != text.trim());
                }
                false
            }
        }
    }

    /// Take a posted message OpenFox just added to the session: true when it was one.
    fn taken(&self, agent: &str, content: &str) -> bool {
        let mut live = self.live.lock().expect("not poisoned");
        let Some(l) = live.get_mut(agent) else {
            return false;
        };
        match l.posted.iter().position(|(p, _)| p == content.trim()) {
            Some(i) => {
                l.posted.remove(i);
                true
            }
            None => false,
        }
    }

    /// Messages posted meanwhile that OpenFox has not taken yet (and still might).
    fn waiting(&self, agent: &str) -> bool {
        let mut live = self.live.lock().expect("not poisoned");
        live.get_mut(agent).is_some_and(|l| {
            l.posted.retain(|(_, at)| at.elapsed() < QUEUED_PATIENCE);
            !l.posted.is_empty()
        })
    }

    pub async fn turn(&self, t: &Turn) -> RunSummary {
        if let Err(e) = prepare_model(&self.prepare, self.prepare_timeout, &self.extra_env, t).await
        {
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
        let limit = Duration::from_secs(self.settings().turn_timeout);
        let out = tokio::time::timeout(limit, self.conversation(t)).await;
        self.live.lock().expect("not poisoned").remove(&t.agent);
        match out {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                t.append(
                    EventKind::Checkpoint,
                    json!({"reason": "harness_error", "detail": e}),
                );
                RunSummary {
                    stopped: "error".into(),
                    error: e,
                    ..Default::default()
                }
            }
            Err(_) => {
                if let Some(sid) = self.sessions().get(&t.agent).and_then(Value::as_str) {
                    let _ = self
                        .act("POST", &format!("/api/sessions/{sid}/stop"), json!({}))
                        .await;
                }
                t.append(EventKind::Checkpoint, json!({"reason": "harness_timeout"}));
                RunSummary {
                    stopped: "error".into(),
                    error: format!("OpenFox did not finish within {}s", limit.as_secs()),
                    ..Default::default()
                }
            }
        }
    }

    async fn conversation(&self, t: &Turn) -> Result<RunSummary, String> {
        let (session, fresh) = self.session_for(t).await?;
        let sid = session.id.clone();
        let wanted_mode = if MODES.iter().any(|(id, _, _)| *id == t.mode) {
            t.mode.clone()
        } else {
            self.settings().mode
        };
        if session.mode.as_deref() != Some(wanted_mode.as_str()) {
            self.act(
                "PUT",
                &format!("/api/sessions/{sid}/mode"),
                json!({"mode": wanted_mode}),
            )
            .await?;
        }
        if !t.model.is_empty() && session.model().as_deref() != Some(t.model.as_str()) {
            let (provider, model) = t
                .model
                .split_once('/')
                .ok_or_else(|| format!("unknown OpenFox model {:?}", t.model))?;
            self.act(
                "POST",
                &format!("/api/sessions/{sid}/provider"),
                json!({"providerId": provider, "model": model}),
            )
            .await?;
        }
        let mut text = if fresh {
            match primer(t, 24_000)? {
                p if p.is_empty() => t.text.clone(),
                p => format!("{p}\n\n{}", t.text),
            }
        } else {
            t.text.clone()
        };
        if !t.attachments.is_empty() {
            text.push_str("\n\nAttached files (in your working directory):");
            for a in &t.attachments {
                text.push_str(&format!("\n- {} ({}, {} bytes)", a.path, a.mime, a.size));
            }
        }
        let mut link = self.link_for(&sid).await?;
        self.live.lock().expect("not poisoned").insert(
            t.agent.clone(),
            Live {
                session: sid.clone(),
                posted: Vec::new(),
            },
        );
        self.act(
            "POST",
            &format!("/api/sessions/{sid}/message"),
            json!({"content": text}),
        )
        .await?;
        t.phase("inference");
        let mut follow = Follow::new(sid.clone());
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let sent = Instant::now();
        loop {
            tokio::select! {
                got = link.events.recv() => match got {
                    Some(raw) => {
                        follow.heard = Instant::now();
                        if let Some(f) = proto::read(&raw) {
                            if f.session.as_deref().is_some_and(|s| s != sid) {
                                continue; // another session's (a confirmation broadcast to all, say)
                            }
                            if self.on_event(t, &mut follow, f.event).await {
                                break;
                            }
                        }
                    }
                    None => {
                        // the WebSocket dropped: subscribe again, and read where the session is
                        link = self.reopen(&sid, t).await?;
                        follow.resynced = true;
                        follow.asked = Instant::now().checked_sub(self.quiet).unwrap_or_else(Instant::now); // ask at once
                    }
                },
                _ = tick.tick() => {
                    if follow.stopping.is_none() && (t.cancelled() || !t.running()) {
                        let _ = self.act("POST", &format!("/api/sessions/{sid}/stop"), json!({})).await;
                        follow.stopping = Some(Instant::now());
                        follow.summary.stopped = if t.cancelled() { "cancelled".into() } else { "frozen".into() };
                    }
                    if follow.stopping.is_some_and(|at| at.elapsed() > STOP_GRACE) {
                        break;
                    }
                    if follow.idle && !self.waiting(&t.agent) {
                        break; // it stopped, and nothing posted meanwhile is left for it to take
                    }
                    // a quiet session is asked how it is: an end OpenFox said while we could not hear
                    if follow.heard.elapsed() > self.quiet && follow.asked.elapsed() > self.quiet {
                        follow.asked = Instant::now();
                        if let Some(view) = self.session_view(&sid).await? {
                            if !view.session.is_running && (follow.started || sent.elapsed() > self.quiet) && !self.waiting(&t.agent) {
                                follow.reconcile(&view);
                                break;
                            }
                        } else {
                            return Err("OpenFox no longer has this thread's session".into());
                        }
                    }
                }
            }
        }
        if follow.resynced {
            if let Ok(Some(view)) = self.session_view(&sid).await {
                follow.reconcile(&view);
            }
        }
        flush_answer(t, &mut follow.text, &mut follow.summary);
        self.keep(&sid, link);
        Ok(follow.summary)
    }

    async fn reopen(&self, sid: &str, t: &Turn) -> Result<Link, String> {
        let mut wait = Duration::from_millis(500);
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            match self.link_for(sid).await {
                Ok(l) => return Ok(l),
                Err(e) if Instant::now() > until || t.cancelled() => {
                    return Err(format!(
                        "OpenFox's WebSocket dropped and did not come back: {e}"
                    ))
                }
                Err(_) => {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(8));
                }
            }
        }
    }

    /// One event of the turn; true when the turn is over.
    async fn on_event(&self, t: &Turn, f: &mut Follow, event: Event) -> bool {
        let update = |f: &mut Follow, u: Value| {
            acp_update(t, &u, &mut f.text, &mut f.open, &mut f.summary);
        };
        match event {
            Event::Delta(d) if !d.content.is_empty() => update(
                f,
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": d.content}}),
            ),
            Event::ToolCall(c) => update(
                f,
                json!({"sessionUpdate": "tool_call", "toolCallId": c.call_id, "title": tool_title(&c.tool, &c.args),
                       "kind": tool_kind(&c.tool), "rawInput": c.args}),
            ),
            Event::ToolResult(r) => {
                let failed = r.result.get("success") == Some(&Value::Bool(false))
                    || r.result
                        .get("error")
                        .is_some_and(|e| !e.is_null() && e != "");
                let out: String = result_text(&r.result).chars().take(8_000).collect();
                update(
                    f,
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": r.call_id,
                           "status": if failed { "failed" } else { "completed" },
                           "content": [{"type": "content", "content": {"type": "text", "text": out}}]}),
                );
            }
            Event::Confirm(c) => {
                if c.session_id.as_deref().is_some_and(|s| s != f.session)
                    || !f.asked_calls.insert(c.call_id.clone())
                {
                    return false;
                }
                let what = if c.paths.is_empty() {
                    c.reason.replace('_', " ")
                } else {
                    c.paths.join(", ")
                };
                let allowed = t
                    .runner
                    .approve(&t.agent, &format!("{}: {what}", c.tool))
                    .await;
                if let Err(e) = self
                    .act(
                        "POST",
                        &format!("/api/sessions/{}/confirm-path", f.session),
                        json!({"callId": c.call_id, "approved": allowed, "alwaysAllow": false}),
                    )
                    .await
                {
                    eprintln!("openfox: confirm: {e}");
                }
            }
            Event::Ask(q) => {
                if !f.asked_calls.insert(q.call_id.clone()) {
                    return false;
                }
                let path = format!("/api/sessions/{}/answer", f.session);
                let body = if q.kind.as_deref() == Some("confirm") {
                    let yes = t.runner.approve(&t.agent, &q.question).await;
                    json!({"callId": q.call_id, "answer": if yes { "yes" } else { "no" }})
                } else {
                    // a question with choices or free text: shown in the thread, the next message answers it
                    let mut shown = format!("\n\n{}\n", q.question);
                    for o in &q.options {
                        let v = o
                            .value
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| o.value.to_string());
                        shown.push_str(&format!("- {}\n", o.label.clone().unwrap_or(v)));
                    }
                    update(
                        f,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": shown}}),
                    );
                    json!({"callId": q.call_id, "skip": true})
                };
                if let Err(e) = self.act("POST", &path, body).await {
                    eprintln!("openfox: answer: {e}");
                }
            }
            Event::Error(e) => {
                let said = format!(
                    "\n\n[OpenFox] {}",
                    Some(e.text())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "OpenFox reported an error".into())
                );
                update(
                    f,
                    json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}}),
                );
            }
            Event::Retrying(e) => {
                if !f.retrying {
                    f.retrying = true;
                    let said = format!(
                        "[OpenFox] the model did not answer ({}); retrying…\n",
                        e.text().trim().chars().take(200).collect::<String>()
                    );
                    update(
                        f,
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}}),
                    );
                }
            }
            Event::RetryFailed(e) => {
                let attempts = e
                    .attempts
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_else(|| "?".into());
                let said = format!(
                    "\n[OpenFox] gave up after {attempts} attempts: {}. Pick another model.\n",
                    e.text().trim().chars().take(300).collect::<String>()
                );
                update(
                    f,
                    json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}}),
                );
                return true;
            }
            Event::Running(r) => {
                if r.is_running {
                    f.started = true;
                    f.idle = false;
                    t.phase("inference");
                } else if f.started {
                    f.idle = true; // the turn ends once nothing posted meanwhile is left for OpenFox
                    return f.stopping.is_some() || !self.waiting(&t.agent);
                }
            }
            Event::Said(s) if s.message.role == "user" => {
                // a message posted during the turn, which OpenFox takes now: journaled where it lands
                if self.taken(&t.agent, &s.message.content) {
                    flush_answer(t, &mut f.text, &mut f.summary);
                    if let Some(e) = t.append(
                        EventKind::UserMessage,
                        json!({"text": s.message.content.trim()}),
                    ) {
                        t.emit(&e);
                    }
                }
            }
            Event::Done(d)
                if f.stopping.is_some() && (d.reason == "stopped" || d.reason == "complete") =>
            {
                return true
            }
            Event::Context(c) => t
                .runner
                .set_usage(&t.agent, c.current_tokens, c.max_tokens, None),
            Event::Malformed(kind, why) => {
                eprintln!("openfox: {kind} did not read ({why}): OpenFox may have changed it")
            }
            _ => {}
        }
        false
    }
}

/// Where a turn stands while it follows OpenFox.
struct Follow {
    session: String,
    text: String,
    open: HashMap<String, bool>,
    summary: RunSummary,
    asked_calls: std::collections::HashSet<String>,
    started: bool,
    /// OpenFox said it stopped running (it may start again for a message posted meanwhile).
    idle: bool,
    retrying: bool,
    resynced: bool,
    stopping: Option<Instant>,
    heard: Instant,
    asked: Instant,
}

impl Follow {
    fn new(session: String) -> Self {
        Self {
            session,
            text: String::new(),
            open: HashMap::new(),
            summary: RunSummary {
                stopped: "done".into(),
                ..Default::default()
            },
            asked_calls: Default::default(),
            started: false,
            idle: false,
            retrying: false,
            resynced: false,
            stopping: None,
            heard: Instant::now(),
            asked: Instant::now(),
        }
    }

    /// After a gap in the events: the answer as OpenFox kept it, when ours lacks its end.
    fn reconcile(&mut self, view: &proto::SessionView) {
        let Some(last) = view.messages.iter().rev().find(|m| m.role == "assistant") else {
            return;
        };
        let full = last.content.trim();
        if !self.text.trim().is_empty() && full.starts_with(self.text.trim()) {
            self.text = full.to_string();
        }
    }
}

fn reason(raw: &[u8]) -> String {
    serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
        .unwrap_or_default()
}

/// The ACP kind of an OpenFox tool, for the thread's icons.
pub fn tool_kind(tool: &str) -> &'static str {
    match tool.to_lowercase().as_str() {
        "read" | "read_file" | "view" => "read",
        "glob" | "grep" | "search" | "list" | "ls" => "search",
        "write" | "write_file" | "edit" | "edit_file" | "patch" | "multi_edit" => "edit",
        "delete" => "delete",
        "move" => "move",
        "bash" | "run" | "shell" | "exec" | "run_command" => "execute",
        "fetch" | "web_fetch" | "web_search" => "fetch",
        "think" => "think",
        _ => "other",
    }
}

/// A tool call's title: the tool and its main argument.
pub fn tool_title(tool: &str, args: &Value) -> String {
    for key in [
        "command",
        "cmd",
        "path",
        "file_path",
        "filePath",
        "pattern",
        "url",
        "query",
    ] {
        if let Some(v) = args
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            return format!("{tool}: {}", v.chars().take(120).collect::<String>());
        }
    }
    tool.to_string()
}

fn result_text(result: &Value) -> String {
    match result {
        Value::String(s) => s.clone(),
        Value::Object(o) => ["output", "content", "error", "message"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| result.to_string().chars().take(4_000).collect()),
        other => other.to_string().chars().take(4_000).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_into_types_and_a_changed_one_shows() {
        let f = proto::read(
            r#"{"type":"chat.delta","sessionId":"s1","payload":{"messageId":"m","content":"Hel"}}"#,
        )
        .unwrap();
        assert_eq!(f.session.as_deref(), Some("s1"));
        assert!(matches!(f.event, Event::Delta(ref d) if d.content == "Hel"));
        let f = proto::read(r#"{"type":"session.running","payload":{"isRunning":false}}"#).unwrap();
        assert!(matches!(f.event, Event::Running(ref r) if !r.is_running));
        let f = proto::read(r#"{"type":"chat.tool_call","payload":{"tool":"bash"}}"#).unwrap();
        assert!(
            matches!(f.event, Event::Malformed(ref k, _) if k == "chat.tool_call"),
            "no callId: {f:?}"
        );
        let f = proto::read(r#"{"type":"git.status","payload":{}}"#).unwrap();
        assert!(matches!(f.event, Event::Other(ref k) if k == "git.status"));
        assert!(proto::read("not json").is_none());
    }

    #[test]
    fn providers_are_read_without_their_keys() {
        let p: proto::Providers = serde_json::from_value(json!({"activeProviderId": "local",
            "providers": [{"id": "local", "name": "vLLM", "apiKey": "sk-secret", "credentialRef": "x",
                           "models": [{"id": "qwen", "contextWindow": 32768}, "plain"]}]}))
        .unwrap();
        let shown = format!("{p:?}");
        assert!(!shown.contains("sk-secret") && !shown.contains("credentialRef"));
        assert_eq!(
            p.providers[0]
                .models
                .iter()
                .map(|m| m.id())
                .collect::<Vec<_>>(),
            ["qwen", "plain"]
        );
    }

    #[test]
    fn settings_are_checked_before_they_are_kept() {
        let ok = Settings {
            url: "http://127.0.0.1:10369".into(),
            mode: "builder".into(),
            turn_timeout: 900,
        };
        assert_eq!(ok.problem(), None);
        assert!(Settings {
            url: "https://x".into(),
            ..ok.clone()
        }
        .problem()
        .is_some());
        assert!(Settings {
            mode: "chaos".into(),
            ..ok.clone()
        }
        .problem()
        .is_some());
        assert!(Settings {
            turn_timeout: 5,
            ..ok
        }
        .problem()
        .is_some());
    }

    #[test]
    fn a_frame_masks_its_payload_and_says_its_length() {
        let f = frame(0x1, b"hi");
        assert_eq!(f[0], 0x81);
        assert_eq!(f[1], 0x80 | 2);
        let mask = &f[2..6];
        assert_eq!([f[6] ^ mask[0], f[7] ^ mask[1]], *b"hi");
        assert_eq!(frame(0x1, &[0u8; 300])[1], 0x80 | 126);
    }
}
