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
        if self.mode.is_empty()
            || self.mode.len() > 64
            || !self
                .mode
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        {
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
        #[serde(default)]
        pub phase: Option<String>,
        #[serde(default)]
        pub danger_level: Option<String>,
        #[serde(default)]
        pub provider_reasoning_effort: Option<String>,
        #[serde(default)]
        pub pause_state: Option<String>,
        /// Structured session data by key: criteria, todos, review findings…
        #[serde(default)]
        pub metadata_entries: std::collections::BTreeMap<String, Vec<Entry>>,
    }

    /// One entry of the session's structured data (a criterion, a todo…).
    #[derive(Deserialize, Debug, Clone)]
    pub struct Entry {
        pub id: String,
        pub description: String,
        #[serde(default)]
        pub status: String,
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
    #[serde(rename_all = "camelCase")]
    pub struct SessionView {
        pub session: Session,
        #[serde(default)]
        pub messages: Vec<Message>,
        #[serde(default)]
        pub context_state: Option<ContextState>,
        #[serde(default)]
        pub session_stats: Option<StatsSummary>,
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
            #[serde(default, rename = "reasoningEfforts")]
            reasoning_efforts: Option<Vec<String>>,
            #[serde(default)]
            modes: Option<Vec<EffortMode>>,
        },
        Bare(String),
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct EffortMode {
        pub level: String,
    }

    impl ModelRef {
        pub fn id(&self) -> &str {
            match self {
                ModelRef::Named { id, .. } | ModelRef::Bare(id) => id,
            }
        }

        /// The reasoning efforts this model offers (none: it has no such setting).
        pub fn efforts(&self) -> Vec<String> {
            match self {
                ModelRef::Named {
                    reasoning_efforts,
                    modes,
                    ..
                } => {
                    let mut out: Vec<String> = reasoning_efforts.clone().unwrap_or_default();
                    for m in modes.iter().flatten() {
                        if !out.contains(&m.level) {
                            out.push(m.level.clone());
                        }
                    }
                    out
                }
                ModelRef::Bare(_) => Vec::new(),
            }
        }
    }

    /// An OpenFox agent (`GET /api/agents`): the top-level ones are what a session runs as.
    #[derive(Deserialize, Debug, Clone)]
    pub struct Agent {
        pub id: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub description: String,
        #[serde(default)]
        pub subagent: bool,
    }

    #[derive(Deserialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct Agents {
        #[serde(default)]
        pub defaults: Vec<Agent>,
        #[serde(default)]
        pub user_items: Vec<Agent>,
        #[serde(default)]
        pub project_items: Vec<Agent>,
    }

    impl Agents {
        /// The agents a session can run as: the top-level ones, a user's or project's own
        /// replacing a built-in one of the same id.
        pub fn top_level(self) -> Vec<Agent> {
            let mut out: Vec<Agent> = Vec::new();
            for a in self
                .defaults
                .into_iter()
                .chain(self.user_items)
                .chain(self.project_items)
                .filter(|a| !a.subagent)
            {
                match out.iter_mut().find(|o| o.id == a.id) {
                    Some(o) => *o = a,
                    None => out.push(a),
                }
            }
            out
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
    #[serde(rename_all = "camelCase")]
    pub struct Delta {
        pub content: String,
        /// Set when the text is a sub-agent's, not the answer's.
        #[serde(default)]
        pub sub_agent_type: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Thinking {
        pub message_id: String,
        pub content: String,
    }

    #[derive(Deserialize, Debug, Clone)]
    pub struct Todo {
        pub content: String,
        pub status: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct Todos {
        pub todos: Vec<Todo>,
    }

    #[derive(Deserialize, Debug)]
    pub struct MetadataUpdated {
        pub key: String,
        pub entries: Vec<Entry>,
    }

    #[derive(Deserialize, Debug)]
    pub struct CriterionStatus {
        #[serde(rename = "type")]
        pub kind: String,
        #[serde(default)]
        pub reason: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    pub struct Criterion {
        pub id: String,
        pub description: String,
        pub status: CriterionStatus,
    }

    #[derive(Deserialize, Debug)]
    pub struct CriteriaUpdated {
        pub criteria: Vec<Criterion>,
    }

    #[derive(Deserialize, Debug)]
    pub struct ModeChanged {
        pub mode: String,
        #[serde(default)]
        pub auto: bool,
        #[serde(default)]
        pub reason: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    pub struct PhaseChanged {
        pub phase: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Queued {
        pub queue_id: String,
        pub content: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct QueueState {
        pub messages: Vec<Queued>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Pause {
        pub pause_state: String,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct Retry {
        #[serde(default)]
        pub attempt: u64,
        #[serde(default)]
        pub retry_in_ms: u64,
        #[serde(default)]
        pub error: Option<Value>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct TurnStats {
        pub model: String,
        #[serde(default)]
        pub reasoning_effort: Option<String>,
        #[serde(default)]
        pub total_time: f64,
        #[serde(default)]
        pub prefill_tokens: u64,
        #[serde(default)]
        pub generation_tokens: u64,
        #[serde(default)]
        pub generation_speed: f64,
    }

    #[derive(Deserialize, Debug)]
    pub struct Stats {
        pub stats: TurnStats,
    }

    #[derive(Deserialize, Debug)]
    pub struct Progress {
        pub message: String,
    }

    /// `git.status`: the workspace's branch and what changed in it.
    #[derive(Deserialize, Debug)]
    pub struct GitStatus {
        pub branch: Option<String>,
        pub diff: GitDiff,
    }

    #[derive(Deserialize, Debug)]
    pub struct GitDiff {
        pub files: Vec<GitFile>,
    }

    #[derive(Deserialize, Debug, serde::Serialize, Clone)]
    pub struct GitFile {
        pub path: String,
        pub status: String,
        #[serde(default)]
        pub additions: u64,
        #[serde(default)]
        pub deletions: u64,
    }

    /// The session's totals (`sessionStats` of `GET /api/sessions/:id`).
    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct StatsSummary {
        pub total_time: f64,
        #[serde(default)]
        pub tool_time: f64,
        pub prefill_tokens: u64,
        pub generation_tokens: u64,
        #[serde(default)]
        pub avg_generation_speed: f64,
        #[serde(default)]
        pub response_count: u64,
        #[serde(default)]
        pub llm_call_count: u64,
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

    #[derive(Deserialize, serde::Serialize, Debug, Clone)]
    pub struct Choice {
        pub value: String,
        #[serde(default)]
        pub label: Option<String>,
        #[serde(default)]
        pub description: Option<String>,
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
        pub options: Option<Vec<Choice>>,
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
    #[serde(rename_all = "camelCase")]
    pub struct Done {
        #[serde(default)]
        pub reason: String,
        /// Set when a sub-agent finished, not the turn.
        #[serde(default)]
        pub agent_type: Option<String>,
    }

    /// `context.state`: how full the context window is (`{context: {...}}`).
    #[derive(Deserialize, Debug)]
    pub struct ContextPayload {
        pub context: ContextState,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct ContextState {
        pub current_tokens: u64,
        pub max_tokens: u64,
        #[serde(default)]
        pub compaction_count: u64,
        #[serde(default)]
        pub danger_zone: bool,
        #[serde(default)]
        pub can_compact: bool,
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
        Retrying(Retry),
        RetryFailed(Failure),
        Running(Running),
        Said(Said),
        Done(Done),
        Context(ContextPayload),
        Thinking(Thinking),
        Todos(Todos),
        Metadata(MetadataUpdated),
        Criteria(CriteriaUpdated),
        Mode(ModeChanged),
        Phase(PhaseChanged),
        Queue(QueueState),
        Paused(Pause),
        Stats(Stats),
        Progress(Progress),
        Git(GitStatus),
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
            "chat.thinking" => typed(&kind, payload, Event::Thinking),
            "chat.todo" => typed(&kind, payload, Event::Todos),
            "metadata.updated" => typed(&kind, payload, Event::Metadata),
            "criteria.updated" => typed(&kind, payload, Event::Criteria),
            "mode.changed" => typed(&kind, payload, Event::Mode),
            "phase.changed" => typed(&kind, payload, Event::Phase),
            "queue.state" => typed(&kind, payload, Event::Queue),
            "session.pause" => typed(&kind, payload, Event::Paused),
            "chat.stats" => typed(&kind, payload, Event::Stats),
            "chat.progress" => typed(&kind, payload, Event::Progress),
            "git.status" => typed(&kind, payload, Event::Git),
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
    /// What each thread shows of OpenFox beside its conversation (`Panel`).
    panels: Mutex<HashMap<String, Panel>>,
    /// OpenFox's top-level agents, read at most once a minute.
    agents_seen: Mutex<Option<(Instant, Vec<proto::Agent>)>>,
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
            panels: Mutex::default(),
            agents_seen: Mutex::default(),
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

    /// Whether OpenFox reaches a model server at `url`: the model it found there.
    pub async fn test_provider(&self, url: &str, backend: &str) -> Result<String, String> {
        let (status, raw) = self
            .raw(
                "POST",
                "/api/providers/test",
                Some(json!({"url": url, "backend": backend})),
                true,
            )
            .await?;
        let v: Value = serde_json::from_slice(&raw).unwrap_or_default();
        if status >= 400 || v.get("success") == Some(&Value::Bool(false)) {
            return Err(format!("OpenFox could not reach {url}: {}", reason(&raw)));
        }
        Ok(v.get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// Add a provider to OpenFox. Its API key, if any, goes to OpenFox and nowhere else: the
    /// core neither keeps nor says it.
    pub async fn add_provider(&self, p: &Value) -> Result<(), String> {
        let text = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let (name, url, backend) = (text("name"), text("url"), text("backend"));
        if name.is_empty() || url.is_empty() || backend.is_empty() {
            return Err("a provider needs a name, an address and a kind".into());
        }
        let mut body = json!({"name": name, "url": url, "backend": backend,
                              "isLocal": p.get("local").and_then(Value::as_bool).unwrap_or(true)});
        let key = text("api_key");
        if !key.is_empty() {
            body["apiKey"] = json!(key);
        }
        self.act("POST", "/api/providers", body).await
    }

    pub async fn remove_provider(&self, id: &str) -> Result<(), String> {
        let (status, raw) = self
            .raw("DELETE", &format!("/api/providers/{id}"), None, true)
            .await?;
        if status >= 400 {
            return Err(format!("OpenFox kept the provider: {}", reason(&raw)));
        }
        Ok(())
    }

    /// Ask the provider's server for its models again.
    pub async fn refresh_provider(&self, id: &str) -> Result<(), String> {
        self.act("POST", &format!("/api/providers/{id}/refresh"), json!({}))
            .await
    }

    /// How this node's OpenFox is: for the settings in the interface.
    pub async fn status(&self) -> Value {
        let s = self.settings();
        let mut out = json!({"url": s.url, "mode": s.mode, "turn_timeout": s.turn_timeout,
                             "modes": self.agent_list().await,
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
            modes: self.agent_list().await,
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
        if t.resume {
            // OpenFox runs its last turn again (after a model's failure): follow it, say nothing new
            let link = self.link_for(&sid).await?;
            self.start_panel(t, &session).await;
            self.live.lock().expect("not poisoned").insert(
                t.agent.clone(),
                Live {
                    session: sid.clone(),
                    posted: Vec::new(),
                },
            );
            link.send(
                &json!({"id": crate::wire::random_hex(4), "type": "chat.retry", "payload": {}}),
            )
            .await?;
            t.phase("inference");
            return self.follow(t, sid, link).await;
        }
        // the agent a new session starts with; after that it is OpenFox's (it switches by itself,
        // from planner to builder say) or the person's, from the panel: never put back here
        let wanted_mode = if t.mode.is_empty() {
            self.settings().mode
        } else {
            t.mode.clone()
        };
        if fresh && session.mode.as_deref() != Some(wanted_mode.as_str()) {
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
        let link = self.link_for(&sid).await?;
        self.start_panel(t, &session).await;
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
        self.follow(t, sid, link).await
    }

    /// Follow the session until its turn is over: its events into the journal and the panel.
    async fn follow(&self, t: &Turn, sid: String, mut link: Link) -> Result<RunSummary, String> {
        let mut follow = Follow::new(sid.clone());
        let mut tick = tokio::time::interval(PANEL_EVERY);
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
                    if follow.panel_dirty {
                        self.show(t, &mut follow, true); // what waited for its turn to be shown
                    }
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
        self.edit_panel(&t.agent, |p| {
            p.running = false;
            p.thinking.clear();
            p.progress.clear();
            for a in p.subagents.iter_mut() {
                a.done = true;
            }
        });
        if let Err(e) = self.read_back(&t.agent, &sid).await {
            eprintln!("openfox: the session's totals: {e}"); // the panel keeps what it had
        }
        self.show(t, &mut follow, true);
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
        let say = |f: &mut Follow, text: String| {
            acp_update(
                t,
                &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}),
                &mut f.text,
                &mut f.open,
                &mut f.summary,
            );
        };
        let mut force = false; // a change worth showing at once (not a piece of text)
        match event {
            Event::Delta(d) if d.content.is_empty() => {}
            Event::Delta(d) => match d.sub_agent_type {
                // a sub-agent's words go beside the conversation, not into the answer
                Some(kind) => self.edit_panel(&t.agent, |p| {
                    match p
                        .subagents
                        .iter_mut()
                        .rev()
                        .find(|a| a.kind == kind && !a.done)
                    {
                        Some(a) => a.text.push_str(&d.content),
                        None => {
                            force = true;
                            p.subagents.push(SubAgent {
                                kind,
                                text: d.content,
                                done: false,
                            })
                        }
                    }
                    for a in p.subagents.iter_mut() {
                        a.text = tail(&a.text, 4_000);
                    }
                }),
                None => {
                    self.edit_panel(&t.agent, |p| {
                        force = p.retry.is_some() || !p.progress.is_empty();
                        p.retry = None;
                        p.progress.clear();
                    });
                    say(f, d.content);
                }
            },
            Event::Thinking(th) => self.edit_panel(&t.agent, |p| {
                if f.thinking_id != th.message_id {
                    f.thinking_id = th.message_id.clone();
                    p.thinking.clear();
                }
                p.thinking.push_str(&th.content);
                p.thinking = tail(&p.thinking, 20_000);
            }),
            Event::ToolCall(c) => {
                if let Some(d) = edit_diff(&c.tool, &c.args) {
                    f.diffs.insert(c.call_id.clone(), d);
                }
                update(
                    f,
                    json!({"sessionUpdate": "tool_call", "toolCallId": c.call_id, "title": tool_title(&c.tool, &c.args),
                           "kind": tool_kind(&c.tool), "rawInput": c.args}),
                )
            }
            Event::ToolResult(r) => {
                let failed = r.result.get("success") == Some(&Value::Bool(false))
                    || r.result
                        .get("error")
                        .is_some_and(|e| !e.is_null() && e != "");
                let mut out: String = result_text(&r.result).chars().take(8_000).collect();
                if let Some(d) = f.diffs.remove(&r.call_id).filter(|_| !failed) {
                    out.push_str("\n\n");
                    out.push_str(&d);
                }
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
                if q.kind.as_deref() == Some("confirm") {
                    let yes = t.runner.approve(&t.agent, &q.question).await;
                    if let Err(e) = self
                        .act(
                            "POST",
                            &format!("/api/sessions/{}/answer", f.session),
                            json!({"callId": q.call_id, "answer": if yes { "yes" } else { "no" }}),
                        )
                        .await
                    {
                        eprintln!("openfox: answer: {e}");
                    }
                } else {
                    // a choice or a free answer: the page asks it; OpenFox waits for the answer
                    force = true;
                    self.edit_panel(&t.agent, |p| {
                        p.question = Some(Question {
                            call_id: q.call_id.clone(),
                            question: q.question.clone(),
                            kind: q.kind.clone().unwrap_or_else(|| "text".into()),
                            options: q.options.clone().unwrap_or_default(),
                        })
                    });
                }
            }
            Event::Error(e) => {
                let why = Some(e.text())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "OpenFox reported an error".into());
                say(f, format!("\n\n[OpenFox] {why}"));
            }
            Event::Retrying(r) => {
                force = true;
                let why = r
                    .error
                    .as_ref()
                    .map(|e| {
                        e.as_str()
                            .map(String::from)
                            .unwrap_or_else(|| e.to_string())
                    })
                    .unwrap_or_default();
                self.edit_panel(&t.agent, |p| {
                    p.retry = Some(RetryState {
                        attempt: r.attempt,
                        in_ms: r.retry_in_ms,
                        error: tail(&why, 300),
                        failed: false,
                    })
                });
            }
            Event::RetryFailed(e) => {
                // OpenFox gave up on the model: the page offers to retry (Runner::resume follows it)
                force = true;
                let attempts = e.attempts.as_ref().and_then(Value::as_u64).unwrap_or(0);
                self.edit_panel(&t.agent, |p| {
                    p.retry = Some(RetryState {
                        attempt: attempts,
                        in_ms: 0,
                        error: tail(&e.text(), 300),
                        failed: true,
                    })
                });
            }
            Event::Running(r) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.running = r.is_running);
                if r.is_running {
                    f.started = true;
                    f.idle = false;
                    t.phase("inference");
                } else if f.started {
                    f.idle = true; // the turn ends once nothing posted meanwhile is left for OpenFox
                    self.show(t, f, true);
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
            Event::Done(d) if d.agent_type.as_deref() == Some("sub-agent") => {
                force = true;
                self.edit_panel(&t.agent, |p| {
                    if let Some(a) = p.subagents.iter_mut().rev().find(|a| !a.done) {
                        a.done = true;
                    }
                });
            }
            Event::Done(d)
                if f.stopping.is_some() && (d.reason == "stopped" || d.reason == "complete") =>
            {
                return true
            }
            Event::Context(c) => {
                force = true;
                let c = c.context;
                t.runner
                    .set_usage(&t.agent, c.current_tokens, c.max_tokens, None);
                self.edit_panel(&t.agent, |p| p.context = Some(ContextView::from(&c)));
            }
            Event::Todos(list) => {
                force = true;
                self.edit_panel(&t.agent, |p| {
                    p.todos = list.todos.iter().map(Item::todo).collect()
                });
            }
            Event::Metadata(m) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.set_entries(&m.key, &m.entries));
            }
            Event::Criteria(c) => {
                force = true;
                self.edit_panel(&t.agent, |p| {
                    p.criteria = c
                        .criteria
                        .iter()
                        .map(|x| Item {
                            id: x.id.clone(),
                            text: x.description.clone(),
                            status: x.status.kind.clone(),
                            reason: x.status.reason.clone(),
                        })
                        .collect()
                });
            }
            Event::Mode(m) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.agent = m.mode);
            }
            Event::Phase(ph) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.phase = ph.phase);
            }
            Event::Queue(q) => {
                force = true;
                self.edit_panel(&t.agent, |p| {
                    p.queue = q
                        .messages
                        .iter()
                        .map(|m| json!({"id": m.queue_id, "content": m.content}))
                        .collect()
                });
            }
            Event::Paused(ps) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.paused = ps.pause_state);
            }
            Event::Stats(st) => {
                force = true;
                let x = st.stats;
                self.edit_panel(&t.agent, |p| {
                    p.stats = Some(json!({"model": x.model, "effort": x.reasoning_effort, "seconds": x.total_time,
                                          "tokens_in": x.prefill_tokens, "tokens_out": x.generation_tokens,
                                          "speed": x.generation_speed}))
                });
            }
            Event::Git(g) => {
                force = true;
                self.edit_panel(&t.agent, |p| {
                    p.git = Some(json!({"branch": g.branch, "files": g.diff.files}))
                });
            }
            Event::Progress(pr) => {
                force = true;
                self.edit_panel(&t.agent, |p| p.progress = pr.message);
            }
            Event::Malformed(kind, why) => {
                eprintln!("openfox: {kind} did not read ({why}): OpenFox may have changed it")
            }
            _ => {}
        }
        self.show(t, f, force);
        false
    }

    /// Change what a thread shows of OpenFox.
    fn edit_panel(&self, agent: &str, change: impl FnOnce(&mut Panel)) {
        let mut panels = self.panels.lock().expect("not poisoned");
        change(panels.entry(agent.to_string()).or_default());
    }

    /// Send the thread's panel to the pages: at most every PANEL_EVERY, at once when `now`.
    fn show(&self, t: &Turn, f: &mut Follow, now: bool) {
        if !now && f.panel_at.elapsed() < PANEL_EVERY {
            f.panel_dirty = true;
            return;
        }
        f.panel_at = Instant::now();
        f.panel_dirty = false;
        self.publish_panel(&t.runner, &t.agent);
    }

    fn publish_panel(&self, runner: &crate::agent::Runner, agent: &str) {
        let panel = self
            .panels
            .lock()
            .expect("not poisoned")
            .get(agent)
            .cloned()
            .unwrap_or_default();
        runner.tell(
            &format!("panel.{agent}"),
            json!({"agent": agent, "panel": panel}),
        );
    }

    /// The panel as OpenFox has the session now: its agent, effort, phase, structured data.
    async fn start_panel(&self, t: &Turn, session: &proto::Session) {
        let agents = self.agent_list().await;
        let efforts = self.efforts_for(session.model()).await;
        self.edit_panel(&t.agent, |p| {
            p.agent = session.mode.clone().unwrap_or_default();
            p.agents = agents;
            p.effort = session.provider_reasoning_effort.clone();
            p.efforts = efforts;
            p.danger = session
                .danger_level
                .clone()
                .unwrap_or_else(|| "normal".into());
            p.phase = session.phase.clone().unwrap_or_default();
            p.paused = session.pause_state.clone().unwrap_or_else(|| "none".into());
            p.running = true;
            p.question = None;
            p.error.clear();
            p.subagents.retain(|a| !a.done);
            for (key, entries) in &session.metadata_entries {
                p.set_entries(key, entries);
            }
        });
        self.publish_panel(&t.runner, &t.agent);
    }

    /// OpenFox's top-level agents, as modes `{id, name, description}` (read once a minute).
    async fn agent_list(&self) -> Vec<Value> {
        let fresh = self
            .agents_seen
            .lock()
            .expect("not poisoned")
            .as_ref()
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
            .map(|(_, a)| a.clone());
        let agents = match fresh {
            Some(a) => a,
            None => match self.call::<proto::Agents>("GET", "/api/agents", None).await {
                Ok(a) => {
                    let list = a.top_level();
                    *self.agents_seen.lock().expect("not poisoned") =
                        Some((Instant::now(), list.clone()));
                    list
                }
                Err(e) => {
                    eprintln!("openfox: {e}");
                    Vec::new()
                }
            },
        };
        if agents.is_empty() {
            return MODES
                .iter()
                .map(|(id, name, d)| json!({"id": id, "name": name, "description": d}))
                .collect();
        }
        agents
            .iter()
            .map(|a| json!({"id": a.id, "name": if a.name.is_empty() { &a.id } else { &a.name }, "description": a.description}))
            .collect()
    }

    /// The reasoning efforts the session's model offers ("<provider>/<model>"; none: OpenFox's default).
    async fn efforts_for(&self, model: Option<String>) -> Vec<String> {
        let Ok(p) = self.providers().await else {
            return Vec::new();
        };
        let (provider, model) = match model.as_deref().and_then(|m| m.split_once('/')) {
            Some((pr, m)) => (pr.to_string(), Some(m.to_string())),
            None => (p.active_provider_id.clone().unwrap_or_default(), None),
        };
        p.providers
            .iter()
            .find(|pr| pr.id == provider)
            .and_then(|pr| match &model {
                Some(m) => pr.models.iter().find(|x| x.id() == m),
                None => pr.models.first(),
            })
            .map(proto::ModelRef::efforts)
            .unwrap_or_default()
    }

    /// Something the person did on a thread's OpenFox panel. A retry is followed as a turn of its
    /// own (`Runner::resume`); everything else is OpenFox's at once, and the panel shows it.
    pub async fn thread_action(
        &self,
        runner: &std::sync::Arc<crate::agent::Runner>,
        agent: &str,
        action: &str,
        p: &Value,
    ) -> Result<(), String> {
        let sid = self
            .sessions()
            .get(agent)
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or("this thread has no OpenFox session yet: send it a message first")?;
        let path = |rest: &str| format!("/api/sessions/{sid}{rest}");
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let result = match action {
            "agent" => {
                let mode = text("mode");
                self.act("PUT", &path("/mode"), json!({"mode": mode}))
                    .await
                    .map(|()| {
                        self.edit_panel(agent, |x| x.agent = mode);
                    })
            }
            "effort" => {
                let view = self
                    .session_view(&sid)
                    .await?
                    .ok_or("OpenFox no longer has this session")?;
                let s = view.session;
                let effort = Some(text("effort")).filter(|e| !e.is_empty());
                let provider = s
                    .provider_id
                    .clone()
                    .or(self.providers().await?.active_provider_id)
                    .unwrap_or_default();
                let mut body = json!({"providerId": provider, "reasoningEffort": effort});
                if let Some(m) = &s.provider_model {
                    body["model"] = json!(m);
                }
                self.act("POST", &path("/provider"), body).await.map(|()| {
                    self.edit_panel(agent, |x| x.effort = effort);
                })
            }
            "danger" => {
                let level = if text("level") == "dangerous" {
                    "dangerous"
                } else {
                    "normal"
                };
                self.act("PUT", &path("/danger-level"), json!({"dangerLevel": level}))
                    .await
                    .map(|()| {
                        self.edit_panel(agent, |x| x.danger = level.into());
                    })
            }
            "criteria" => {
                let list: Vec<Value> = p
                    .get("criteria")
                    .and_then(Value::as_array)
                    .ok_or("no criteria")?
                    .iter()
                    .filter_map(|c| {
                        let d = c.get("text").and_then(Value::as_str)?.trim().to_string();
                        (!d.is_empty()).then(|| {
                            let mut o = json!({"description": d});
                            if let Some(id) = c
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|i| !i.is_empty())
                            {
                                o["id"] = json!(id);
                            }
                            if let Some(st) = c
                                .get("status")
                                .and_then(Value::as_str)
                                .filter(|i| !i.is_empty())
                            {
                                o["status"] = json!(st);
                            }
                            o
                        })
                    })
                    .collect();
                self.act("PUT", &path("/criteria"), json!({"criteria": list}))
                    .await
            }
            "answer" => {
                let call = text("call_id");
                let body = match p.get("answer").and_then(Value::as_str) {
                    Some(a) => json!({"callId": call, "answer": a}),
                    None => json!({"callId": call, "skip": true}),
                };
                self.act("POST", &path("/answer"), body).await.map(|()| {
                    self.edit_panel(agent, |x| x.question = None);
                })
            }
            "pause" => self.act("POST", &path("/pause"), json!({})).await,
            "resume" => self.act("POST", &path("/resume"), json!({})).await,
            "dequeue" => {
                let (status, raw) = self
                    .raw(
                        "DELETE",
                        &path(&format!("/queue/{}", text("id"))),
                        None,
                        true,
                    )
                    .await?;
                if status >= 400 {
                    Err(format!("HTTP {status} {}", reason(&raw)))
                } else {
                    Ok(())
                }
            }
            "compact" | "retry_now" => {
                // said on a WebSocket of its own: the turn's, if one runs, is that turn's
                let kind = if action == "compact" {
                    "context.compact"
                } else {
                    "chat.llm_retry_now"
                };
                let link = self.link_for(&sid).await?;
                let sent = link
                    .send(&json!({"id": crate::wire::random_hex(4), "type": kind, "payload": {}}))
                    .await;
                tokio::time::sleep(Duration::from_millis(200)).await; // let it go out before the socket closes
                sent
            }
            "retry" => {
                self.edit_panel(agent, |x| x.retry = None);
                let (r, a) = (std::sync::Arc::clone(runner), agent.to_string());
                tokio::spawn(async move {
                    r.resume(&a).await;
                });
                Ok(())
            }
            "refresh" => Ok(()),
            other => Err(format!("unknown OpenFox action {other:?}")),
        };
        // what OpenFox has now: outside a turn no WebSocket brings it, so it is read back
        if result.is_ok() && !["retry", "retry_now", "compact"].contains(&action) {
            if let Err(e) = self.read_back(agent, &sid).await {
                eprintln!("openfox: reading {agent}'s session back: {e}");
            }
        }
        self.edit_panel(agent, |x| {
            x.error = result.as_ref().err().cloned().unwrap_or_default()
        });
        self.publish_panel(runner, agent);
        result
    }

    /// The panel as OpenFox has the session now (REST): its agent, effort, danger, phase, context
    /// and structured data. Live things (thinking, sub-agents, a question) stay as they are.
    async fn read_back(&self, agent: &str, sid: &str) -> Result<(), String> {
        let view = self
            .session_view(sid)
            .await?
            .ok_or("OpenFox no longer has this session")?;
        let agents = self.agent_list().await;
        let efforts = self.efforts_for(view.session.model()).await;
        let s = view.session;
        self.edit_panel(agent, |x| {
            x.agent = s.mode.clone().unwrap_or_default();
            x.agents = agents;
            x.effort = s.provider_reasoning_effort.clone();
            x.efforts = efforts;
            x.danger = s.danger_level.clone().unwrap_or_else(|| "normal".into());
            x.phase = s.phase.clone().unwrap_or_default();
            x.paused = s.pause_state.clone().unwrap_or_else(|| "none".into());
            x.running = s.is_running;
            if let Some(c) = &view.context_state {
                x.context = Some(ContextView::from(c));
            }
            if let Some(st) = &view.session_stats {
                x.session_stats = Some(json!({"seconds": st.total_time, "tool_seconds": st.tool_time,
                    "tokens_in": st.prefill_tokens, "tokens_out": st.generation_tokens,
                    "speed": st.avg_generation_speed, "answers": st.response_count, "calls": st.llm_call_count}));
            }
            for (key, entries) in &s.metadata_entries {
                x.set_entries(key, entries);
            }
        });
        Ok(())
    }
}

/// How often the panel goes to the pages while text streams in it (thinking, a sub-agent).
const PANEL_EVERY: Duration = Duration::from_millis(250);

/// What a thread shows of OpenFox beside its conversation, sent live to the pages that may see
/// the thread (`panel.<agent>`): the agent it runs as and what it may switch to, the reasoning
/// effort, how bold it may be, its phase, its todos and criteria, its thinking, its sub-agents,
/// its context, the messages waiting in its queue, a question it asks, a retry under way.
#[derive(Serialize, Default, Clone, Debug)]
pub struct Panel {
    pub agent: String,
    pub agents: Vec<Value>,
    pub effort: Option<String>,
    pub efforts: Vec<String>,
    pub danger: String,
    pub phase: String,
    pub paused: String,
    pub running: bool,
    pub todos: Vec<Item>,
    pub criteria: Vec<Item>,
    /// Other structured data OpenFox keeps (review findings…), by key.
    pub metadata: BTreeMap<String, Vec<Item>>,
    pub thinking: String,
    pub subagents: Vec<SubAgent>,
    pub context: Option<ContextView>,
    pub queue: Vec<Value>,
    pub question: Option<Question>,
    pub retry: Option<RetryState>,
    pub stats: Option<Value>,
    /// The session's totals so far.
    pub session_stats: Option<Value>,
    /// The workspace's branch and changed files, as OpenFox last said (during a turn).
    pub git: Option<Value>,
    pub progress: String,
    /// The last action's failure, for the page to say.
    pub error: String,
}

impl Panel {
    fn set_entries(&mut self, key: &str, entries: &[proto::Entry]) {
        let items: Vec<Item> = entries
            .iter()
            .map(|e| Item {
                id: e.id.clone(),
                text: e.description.clone(),
                status: e.status.clone(),
                reason: None,
            })
            .collect();
        match key {
            "criteria" => self.criteria = items,
            "todos" => self.todos = items,
            _ => {
                self.metadata.insert(key.to_string(), items);
            }
        }
    }
}

#[derive(Serialize, Default, Clone, Debug)]
pub struct Item {
    pub id: String,
    pub text: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Item {
    fn todo(t: &proto::Todo) -> Self {
        Self {
            id: String::new(),
            text: t.content.clone(),
            status: t.status.clone(),
            reason: None,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct SubAgent {
    pub kind: String,
    pub text: String,
    pub done: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Question {
    pub call_id: String,
    pub question: String,
    /// "choice" (pick one of `options`) or "text" (a free answer).
    pub kind: String,
    pub options: Vec<proto::Choice>,
}

#[derive(Serialize, Clone, Debug)]
pub struct RetryState {
    pub attempt: u64,
    pub in_ms: u64,
    pub error: String,
    /// OpenFox gave up: only a retry (or another model) goes on.
    pub failed: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ContextView {
    pub used: u64,
    pub max: u64,
    pub compactions: u64,
    pub danger: bool,
    pub can_compact: bool,
}

impl From<&proto::ContextState> for ContextView {
    fn from(c: &proto::ContextState) -> Self {
        Self {
            used: c.current_tokens,
            max: c.max_tokens,
            compactions: c.compaction_count,
            danger: c.danger_zone,
            can_compact: c.can_compact,
        }
    }
}

/// What an edit or a write changes, as diff lines under a `@@ path` header (at most DIFF_LINES):
/// the thread shows it with the tool's result.
fn edit_diff(tool: &str, args: &Value) -> Option<String> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or("");
    let lines = |text: &str, mark: char| -> Vec<String> {
        text.lines().map(|l| format!("{mark} {l}")).collect()
    };
    let mut out: Vec<String> = match tool {
        "edit_file" => {
            let old = args.get("old_string")?.as_str()?;
            let new = args.get("new_string")?.as_str()?;
            lines(old, '-').into_iter().chain(lines(new, '+')).collect()
        }
        "write_file" => lines(args.get("content")?.as_str()?, '+'),
        _ => return None,
    };
    if out.len() > DIFF_LINES {
        let more = out.len() - DIFF_LINES;
        out.truncate(DIFF_LINES);
        out.push(format!("@@ … {more} more lines"));
    }
    Some(format!("@@ {path}\n{}", out.join("\n")))
}

/// How many lines of an edit the thread shows.
const DIFF_LINES: usize = 120;

/// The last `n` characters of a text.
fn tail(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        text.to_string()
    } else {
        text.chars().skip(count - n).collect()
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
    resynced: bool,
    stopping: Option<Instant>,
    heard: Instant,
    asked: Instant,
    /// The step whose thinking the panel shows.
    thinking_id: String,
    /// What an edit changes, by tool call: joined to its result in the journal.
    diffs: HashMap<String, String>,
    panel_at: Instant,
    panel_dirty: bool,
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
            resynced: false,
            stopping: None,
            heard: Instant::now(),
            asked: Instant::now(),
            thinking_id: String::new(),
            diffs: HashMap::new(),
            panel_at: Instant::now(),
            panel_dirty: false,
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
        let f = proto::read(r#"{"type":"devServer.state","payload":{}}"#).unwrap();
        assert!(matches!(f.event, Event::Other(ref k) if k == "devServer.state"));
        let f = proto::read(r#"{"type":"git.status","payload":{}}"#).unwrap();
        assert!(
            matches!(f.event, Event::Malformed(ref k, _) if k == "git.status"),
            "followed now: a git status without its files shows"
        );
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
        // any agent of OpenFox's, yours included; never an empty or odd name
        assert_eq!(
            Settings {
                mode: "reviewer".into(),
                ..ok.clone()
            }
            .problem(),
            None
        );
        for bad in ["", "../x", "a b"] {
            assert!(Settings {
                mode: bad.into(),
                ..ok.clone()
            }
            .problem()
            .is_some());
        }
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
