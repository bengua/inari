//! HTTP/SSE gateway between a browser (desktop or phone) and the bus: it gathers the mesh
//! into one snapshot, pushes it as server-sent events, serves the web interface (`web/`,
//! built into the binary), and turns the person's actions into bus messages. It holds the
//! approval key; pages sign in for a session cookie (`session`), tools send a bearer token.
//! Files from the phone arrive in parts (`upload_id`, `offset`, `total`), so a photo
//! or a PDF of several megabytes needs no large request.

use crate::approvals::sign;
use crate::bus::{request, Bus, Message};
use crate::capsule::attach::{Progress, UploadStore};
use crate::mesh::Telemetry;
use crate::usage::{http, Source};
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

const INDEX: &str = include_str!("../web/index.html");
/// The Kitsune page: the voice notebook, served beside the shell and sharing its palette.
const KITSUNE_INDEX: &str = include_str!("../web/kitsune.html");

/// What the page loads besides itself, under `/assets/`: its style, its script, its fonts.
/// Public like the page (they hold no data), and cached: their URLs carry the page's version.
const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "app.css",
        "text/css; charset=utf-8",
        include_bytes!("../web/app.css"),
    ),
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../web/app.js"),
    ),
    (
        "i18n.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../web/i18n.js"),
    ),
    (
        "kitsune.css",
        "text/css; charset=utf-8",
        include_bytes!("../web/kitsune.css"),
    ),
    (
        "kitsune.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../web/kitsune.js"),
    ),
    (
        "fonts/ibm_plex_sans.woff2",
        "font/woff2",
        include_bytes!("../web/fonts/ibm_plex_sans.woff2"),
    ),
    (
        "fonts/ibm_plex_mono_regular.woff2",
        "font/woff2",
        include_bytes!("../web/fonts/ibm_plex_mono_regular.woff2"),
    ),
    (
        "fonts/ibm_plex_mono_medium.woff2",
        "font/woff2",
        include_bytes!("../web/fonts/ibm_plex_mono_medium.woff2"),
    ),
    (
        "fonts/bricolage_grotesque.woff2",
        "font/woff2",
        include_bytes!("../web/fonts/bricolage_grotesque.woff2"),
    ),
];

/// The page's version: a hash of what this binary serves (the page and its assets). A page
/// that sees another version in the snapshot (the core was updated) reloads itself — a
/// phone's app keeps its page open for days.
fn page_version() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let mut all = INDEX.as_bytes().to_vec();
        all.extend_from_slice(KITSUNE_INDEX.as_bytes());
        for (_, _, body) in ASSETS {
            all.extend_from_slice(body);
        }
        crate::canon::sha256_hex(&all)[..12].to_string()
    })
}
const MAX_BODY: usize = 64 * 1024;
/// One part of an upload, base64 included (the page sends 512 KiB of file per part).
const MAX_PART_BODY: usize = 1024 * 1024;
const MAX_UPLOAD: u64 = 25 * 1024 * 1024;
/// Connections served at once (each open page holds one for its event stream).
const MAX_CONNECTIONS: usize = 512;
/// A request must arrive whole within this time, an upload part included.
const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct State {
    log: HashMap<String, Vec<Value>>,
    notices: Vec<Value>,
    moves: BTreeMap<String, (Value, Instant)>,
    holder: HashMap<String, String>,
    telemetry: Telemetry,
    nodes: Vec<Value>,
    results: Vec<Value>,
    approvals: BTreeMap<String, Value>,
    usage: Vec<Value>,
    logos: Map<String, Value>,
    logo_cache: HashMap<String, (&'static str, Vec<u8>)>,
    /// Harness updates in progress or just finished, by "node/harness".
    update_jobs: BTreeMap<String, Value>,
    /// The core passes the hub keeps (each known by the hash of its token).
    passes: Vec<Value>,
    /// Each thread's answer so far, while its model writes it (`draft.<agent>`).
    drafts: HashMap<String, String>,
    /// The last OpenFox setting changed from the interface, by "node/harness": done or why not.
    openfox_results: BTreeMap<String, Value>,
    /// The last harness change asked of each core, with how it went (owner only).
    harness_results: BTreeMap<String, Value>,
    /// What each OpenFox thread shows beside its conversation (`panel.<agent>`).
    panels: HashMap<String, Value>,
}

/// A clone onto a harness the thread's core lacks: fork it there, teleport the copy to a
/// core that has the harness, and switch the copy's harness on arrival.
async fn clone_across(
    bus: Arc<Bus>,
    agent: String,
    new: String,
    title: String,
    harness: String,
    here: String,
    there: String,
) {
    let mut created = bus.subscribe("agent.created");
    bus.publish(
        &format!("agent.{agent}.fork"),
        json!({"new_thread": new, "title": title}),
    );
    let made = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(m) = created.recv().await {
            if m.payload["agent"] == new.as_str() {
                return m.payload["ok"] == true;
            }
        }
        false
    });
    if !made.await.unwrap_or(false) {
        return;
    }
    let request = crate::wire::random_hex(16);
    let mut results = bus.subscribe("teleport.result");
    bus.publish(
        "teleport.request",
        json!({"request": request, "agent": new, "source": here, "target": there}),
    );
    let moved = tokio::time::timeout(Duration::from_secs(180), async {
        while let Some(m) = results.recv().await {
            if m.payload["request"] == request.as_str() {
                return m.payload["ok"] == true;
            }
        }
        false
    });
    if moved.await.unwrap_or(false) {
        bus.publish(&format!("agent.{new}.harness"), json!({"harness": harness}));
    }
}

/// A thread's journal for a page that already holds it up to event `after`: only what came
/// since, with `from` (where it starts) and `count` (the whole length) so the page can tell
/// a plain continuation from a journal that changed under it and ask for all of it again.
/// An unknown or empty `after` answers the whole journal (`from` 0).
pub fn timeline_since(log: Vec<Value>, after: &str) -> Value {
    let count = log.len();
    let from = if after.is_empty() {
        0
    } else {
        log.iter()
            .position(|e| e.get("id").and_then(Value::as_str) == Some(after))
            .map_or(0, |i| i + 1)
    };
    let events: Vec<Value> = log.into_iter().skip(from).collect();
    json!({"events": events, "from": from, "count": count})
}

/// Who is asking: the core's owner, or the holder of a pass (only its own threads).
#[derive(Clone)]
pub enum Access {
    Owner,
    Guest(Value),
}

impl Access {
    fn pass_id(&self) -> Option<&str> {
        match self {
            Access::Owner => None,
            Access::Guest(p) => p.get("id").and_then(Value::as_str),
        }
    }

    /// The pass's threads are named after it: `<pass id>-<name>`.
    pub fn owns(&self, agent: &str) -> bool {
        match self.pass_id() {
            None => true,
            Some(id) => agent
                .strip_prefix(id)
                .is_some_and(|rest| rest.starts_with('-')),
        }
    }
}

/// What a gateway is given: its access token, the device key that signs approvals,
/// the interface options, the quota sources, where uploads wait, and whether pages
/// opened on this machine need no token.
pub struct GatewayOptions {
    pub token: String,
    pub key: Vec<u8>,
    pub ui: Map<String, Value>,
    pub mascot_dir: Option<PathBuf>,
    pub usage: Vec<Value>,
    pub data_dir: PathBuf,
    pub trust_loopback: bool,
}

pub struct Gateway {
    /// A page opened on this very machine needs no token (see `local_page`).
    trust_loopback: bool,
    bus: Arc<Bus>,
    token: String,
    key: Vec<u8>,
    ui: Map<String, Value>,
    mascot_dir: Option<PathBuf>,
    sources: Vec<Source>,
    uploads: Option<UploadStore>,
    state: Mutex<State>,
    clients: broadcast::Sender<()>,
    /// Answers so far, as they come (agent, text): sent to the pages on their own, not in snapshots.
    drafts: broadcast::Sender<(String, String)>,
    /// OpenFox panels as they change (agent, panel), sent on their own like the drafts.
    panels: broadcast::Sender<(String, Value)>,
    pub usage_interval: Duration,
    sessions: crate::session::Sessions,
    /// The persona each person chose for the mascot.
    personas: crate::personas::Choices,
    /// The voice second brain this core holds (None when it could not be opened).
    kitsune: Option<Arc<std::sync::Mutex<kitsune::Kitsune>>>,
}

/// An OpenFox thread's panel, as an SSE event of its own (`panel`).
fn panel_frame(agent: &str, panel: &Value) -> String {
    format!(
        "event: panel\ndata: {}\n\n",
        json!({"agent": agent, "panel": panel})
    )
}

/// A thread's answer so far, as an SSE event of its own (`draft`); empty text: it is journaled now.
fn draft_frame(agent: &str, text: &str) -> String {
    format!(
        "event: draft\ndata: {}\n\n",
        json!({"agent": agent, "text": text})
    )
}

fn s(p: &Value, k: &str) -> String {
    match p.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// OpenFox panel actions a pass may not do: they run commands, or OpenFox sessions outside the
/// thread (a task), that the pass's supervision does not see.
const OWNER_ONLY: [&str; 9] = [
    "task_add",
    "task_move",
    "task_remove",
    "dev_config",
    "dev_start",
    "dev_restart",
    "instructions",
    // with the owner's forge token
    "git_clone",
    "git_publish",
];

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(a), Some(b)) => {
                        out.push((a * 16 + b) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_of(target: &str) -> (String, HashMap<String, String>) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    (percent_decode(path), params)
}

fn words(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 1)
        .map(String::from)
        .collect()
}

fn mime_of(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "",
    }
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
    /// The connection comes from this machine.
    loopback: bool,
    /// The address on the other end (the gate's, behind one): sign-in attempts are counted by it.
    peer: Option<std::net::IpAddr>,
}

impl Request {
    /// Reached over HTTPS: the gate in front says so. A session cookie is then `Secure`.
    fn https(&self) -> bool {
        self.headers
            .get("x-forwarded-proto")
            .is_some_and(|p| p.eq_ignore_ascii_case("https"))
    }
}

/// What a page may load and where it may talk: its own origin only, no inline script, no
/// framing by another site. Inline styles stay allowed (the page sets a few attributes).
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'";

async fn send(w: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    send_with(w, status, ctype, body, "").await
}

/// [`send`] with extra header lines (each ending in CRLF), such as a `Set-Cookie`.
async fn send_with(
    w: &mut TcpStream,
    status: u16,
    ctype: &str,
    body: &[u8],
    extra: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nContent-Security-Policy: {CSP}\r\nReferrer-Policy: no-referrer\r\n\
         {extra}Connection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    w.write_all(head.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await
}

/// A file of the page: the same headers as [`send`], cached for a day (its URL carries the
/// page's version, so an update is never hidden by the cache).
async fn send_asset(w: &mut TcpStream, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    w.write_all(head.as_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await
}

async fn send_json(w: &mut TcpStream, status: u16, v: &Value) -> std::io::Result<()> {
    send(
        w,
        status,
        "application/json",
        &serde_json::to_vec(v).expect("serializes"),
    )
    .await
}

impl Gateway {
    pub fn new(bus: Arc<Bus>, o: GatewayOptions) -> Arc<Self> {
        let GatewayOptions {
            token,
            key,
            ui,
            mascot_dir,
            usage,
            data_dir,
            trust_loopback,
        } = o;
        let (usage, data_dir) = (&usage[..], &data_dir);
        let (clients, _) = broadcast::channel(50); // a slow client loses old frames, not the stream
        let (drafts, _) = broadcast::channel(256);
        let (panels, _) = broadcast::channel(256);
        let mut full_ui = Map::new();
        full_ui.insert("footer".into(), json!(""));
        full_ui.extend(ui);
        let g = Arc::new(Self {
            trust_loopback,
            bus: Arc::clone(&bus),
            token,
            key,
            ui: full_ui,
            mascot_dir,
            sources: usage.iter().filter_map(Source::from_config).collect(),
            uploads: UploadStore::new(data_dir.join("uploads"), MAX_UPLOAD).ok(),
            state: Mutex::default(),
            clients,
            drafts,
            panels,
            usage_interval: Duration::from_secs(60),
            sessions: crate::session::Sessions::open(data_dir),
            personas: crate::personas::Choices::open(data_dir),
            kitsune: match kitsune::Kitsune::open(&data_dir.join("kitsune")) {
                Ok(k) => Some(Arc::new(std::sync::Mutex::new(k))),
                Err(e) => {
                    eprintln!("kitsune: {e}");
                    None
                }
            },
        });
        g.subscribe();
        g
    }

    fn on(self: &Arc<Self>, pattern: &str, f: fn(&Gateway, &mut State, Message)) {
        let mut rx = self.bus.subscribe(pattern);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                {
                    let mut st = me.state.lock().expect("not poisoned");
                    f(&me, &mut st, m);
                }
                me.push();
            }
        });
    }

    fn subscribe(self: &Arc<Self>) {
        // an OpenFox thread's panel: kept for a page that opens later, passed on without a snapshot
        let mut prx = self.bus.subscribe("panel.*");
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(m) = prx.recv().await {
                let agent = s(&m.payload, "agent");
                let panel = m.payload.get("panel").cloned().unwrap_or(Value::Null);
                me.state
                    .lock()
                    .expect("not poisoned")
                    .panels
                    .insert(agent.clone(), panel.clone());
                let _ = me.panels.send((agent, panel));
            }
        });
        // an answer so far: kept for a page that opens meanwhile, and passed on without a snapshot
        let mut rx = self.bus.subscribe("draft.*");
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                let (agent, text) = (s(&m.payload, "agent"), s(&m.payload, "text"));
                {
                    let mut st = me.state.lock().expect("not poisoned");
                    if text.is_empty() {
                        st.drafts.remove(&agent);
                    } else {
                        st.drafts.insert(agent.clone(), text.clone());
                    }
                }
                let _ = me.drafts.send((agent, text));
            }
        });
        self.on("mesh.nodes", |_, st, m| {
            st.nodes = m
                .payload
                .get("nodes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        });
        self.on("agent.*.state", |g, st, m| {
            let p = &m.payload;
            let (agent, node) = (s(p, "agent"), s(p, "node"));
            let extra: Map<String, Value> = p
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| !["agent", "node", "phase"].contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            st.telemetry
                .update_agent(&agent, &node, &s(p, "phase"), &extra);
            if st.holder.insert(agent.clone(), node.clone()).as_ref() != Some(&node) {
                // first sighting, or it moved: ask the holder for the journal
                g.bus
                    .publish("agent.history.request", json!({"agent": agent}));
            }
        });
        self.on("node.*.load", |_, st, m| {
            let p = &m.payload;
            st.telemetry.update_node(
                &s(p, "node"),
                p.get("gpu").and_then(Value::as_f64).unwrap_or(0.0),
                p.get("packets").and_then(Value::as_f64).unwrap_or(0.0),
            );
        });
        self.on("node.lost", |_, st, m| {
            let node = s(&m.payload, "node");
            st.telemetry.drop_node(&node);
            st.nodes
                .retain(|n| n.get("node_id").and_then(Value::as_str) != Some(node.as_str()));
        });
        self.on("teleport.result", |_, st, m| st.results.push(m.payload));
        self.on("node.*.harness.result", |_, st, m| {
            let key = s(&m.payload, "node");
            let mut r = m.payload;
            r["at"] = json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0));
            st.harness_results.insert(key, r);
        });
        self.on("node.*.openfox.result", |_, st, m| {
            let key = format!("{}/{}", s(&m.payload, "node"), s(&m.payload, "harness"));
            let mut r = m.payload;
            r["at"] = json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0));
            st.openfox_results.insert(key, r);
        });
        self.on("mesh.passes", |_, st, m| {
            st.passes = m
                .payload
                .get("passes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
        });
        self.on("node.*.update.state", |_, st, m| {
            let key = format!("{}/{}", s(&m.payload, "node"), s(&m.payload, "harness"));
            st.update_jobs.insert(key, m.payload);
        });
        self.on("approval.request", |_, st, m| {
            let id = s(&m.payload, "request_id");
            if !id.is_empty() {
                st.approvals.insert(id, m.payload);
            }
        });
        self.on("teleport.state", |_, st, m| {
            st.moves
                .insert(s(&m.payload, "agent"), (m.payload, Instant::now()));
        });
        self.on("agent.*.event", |_, st, m| {
            let p = m.payload;
            let events = st.log.entry(s(&p, "agent")).or_default();
            let id = s(&p, "id");
            if !events.iter().rev().take(50).any(|e| s(e, "id") == id) {
                events.push(json!({"id": id, "kind": p.get("kind"), "text": p.get("text")}));
            }
        });
        self.on("agent.*.history", |_, st, m| {
            let agent = s(&m.payload, "agent");
            let history = m
                .payload
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let known: HashSet<String> = history.iter().map(|e| s(e, "id")).collect();
            let live: Vec<Value> = st
                .log
                .get(&agent)
                .map(|l| {
                    l.iter()
                        .filter(|e| !known.contains(&s(e, "id")))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            st.log
                .insert(agent, history.into_iter().chain(live).collect()); // a late reply keeps newer events
        });
        self.on("agent.deleted", |_, st, m| {
            let agent = s(&m.payload, "agent");
            st.telemetry.agents.remove(&agent);
            st.log.remove(&agent);
            st.holder.remove(&agent);
        });
        self.on("agent.merged", |g, _, m| {
            // the target's journal grew by another one: read it again
            if m.payload.get("ok").and_then(Value::as_bool) == Some(true) {
                g.bus.publish(
                    "agent.history.request",
                    json!({"agent": s(&m.payload, "target")}),
                );
            }
        });
        for subject in ["agent.*.done", "agent.created", "agent.merged"] {
            self.on(subject, |_, st, m| {
                let mut notice = Map::new();
                notice.insert("subject".into(), json!(m.subject));
                if let Some(o) = m.payload.as_object() {
                    notice.extend(o.clone());
                }
                st.notices.push(Value::Object(notice));
                let excess = st.notices.len().saturating_sub(10);
                st.notices.drain(..excess);
            });
        }
    }

    pub fn snapshot(&self) -> Value {
        let mut st = self.state.lock().expect("not poisoned");
        // moves in flight, and finished ones for a few seconds so a client can show the end
        st.moves.retain(|_, (v, at)| {
            !matches!(v["state"].as_str(), Some("committed" | "rolled_back"))
                || at.elapsed() < Duration::from_secs(5)
        });
        let tail = |v: &Vec<Value>| -> Vec<Value> { v[v.len().saturating_sub(5)..].to_vec() };
        let mut ui = self.ui.clone();
        ui.insert("mascot".into(), json!(self.mascot_dir.is_some()));
        ui.insert("self".into(), json!(self.bus.name)); // the core serving this page: the default for new threads
        ui.insert("page".into(), json!(page_version())); // an open page older than this one reloads itself
        json!({
            "nodes": st.nodes,
            "agents": st.telemetry.agents,
            "moves": st.moves.iter().map(|(k, (v, _))| (k.clone(), v.clone())).collect::<Map<String, Value>>(),
            "load": st.telemetry.load(),
            "mascot": st.telemetry.mascot(),
            "results": tail(&st.results),
            "notices": tail(&st.notices),
            "usage": st.usage,
            "ui": ui,
            "approvals": st.approvals.values().cloned().collect::<Vec<_>>(),
            "update_jobs": st.update_jobs,
            "openfox_results": st.openfox_results,
            "harness_results": st.harness_results,
        })
    }

    fn push(&self) {
        if self.clients.receiver_count() > 0 {
            let _ = self.clients.send(());
        }
    }

    /// What one client may see: everything for the owner (with the passes, never their
    /// token hashes); for a pass, its own threads, the cores and its allowed harnesses only.
    pub fn snapshot_for(&self, access: &Access) -> Value {
        let mut snap = self.snapshot();
        // the mascot this person chose, on every device they open
        snap["ui"]["persona"] = json!(self.persona_of(access));
        let passes: Vec<Value> = self
            .state
            .lock()
            .expect("not poisoned")
            .passes
            .iter()
            .map(|p| {
                let mut p = p.clone();
                if let Some(o) = p.as_object_mut() {
                    o.remove("token_sha");
                }
                p
            })
            .collect();
        let Access::Guest(pass) = access else {
            snap["passes"] = json!(passes);
            return snap;
        };
        let allowed: Vec<String> = pass
            .get("harnesses")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|h| h.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(agents) = snap["agents"].as_object_mut() {
            let pid = pass["id"].as_str().unwrap_or("");
            agents.retain(|id, a| {
                self.owned(access, id) || a.get("pass").and_then(Value::as_str) == Some(pid)
            });
        }
        let mine: HashSet<String> = snap["agents"]
            .as_object()
            .map(|a| a.keys().cloned().collect())
            .unwrap_or_default();
        if let Some(list) = snap["approvals"].as_array_mut() {
            list.retain(|r| mine.contains(r.get("agent_id").and_then(Value::as_str).unwrap_or("")));
        }
        if let Some(list) = snap["notices"].as_array_mut() {
            list.retain(|n| mine.contains(n.get("agent").and_then(Value::as_str).unwrap_or("")));
        }
        if let Some(nodes) = snap["nodes"].as_array_mut() {
            for n in nodes.iter_mut() {
                let harnesses: Map<String, Value> = n
                    .pointer("/capabilities/harnesses")
                    .and_then(Value::as_object)
                    .map(|h| {
                        h.iter()
                            .filter(|(k, _)| allowed.contains(k))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let labels = n
                    .pointer("/capabilities/model_labels")
                    .cloned()
                    .unwrap_or(json!([]));
                let hidden = n
                    .pointer("/capabilities/model_hidden")
                    .cloned()
                    .unwrap_or(json!([]));
                let keep = |key: &str| -> Value {
                    let all = n
                        .pointer(&format!("/capabilities/{key}"))
                        .and_then(Value::as_object);
                    Value::Object(
                        all.map(|h| {
                            h.iter()
                                .filter(|(k, _)| allowed.contains(k))
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    )
                };
                let (modes, commands) = (keep("harness_modes"), keep("harness_commands"));
                n["capabilities"] = json!({"harnesses": harnesses, "model_labels": labels, "model_hidden": hidden,
                                           "harness_modes": modes, "harness_commands": commands});
            }
        }
        for k in ["moves", "update_jobs", "openfox_results", "harness_results"] {
            snap[k] = json!({});
        }
        snap["results"] = json!([]);
        snap["usage"] = json!([]);
        snap["ui"]["pass"] = json!({"id": pass["id"], "name": pass["name"], "role": pass["role"], "harnesses": allowed});
        snap
    }

    pub fn search(&self, query: &str, agent: &str) -> Vec<Value> {
        let st = self.state.lock().expect("not poisoned");
        let needle = query.trim().to_lowercase();
        let wanted = words(query);
        let mut exact = Vec::new();
        let mut near: Vec<(usize, Value)> = Vec::new();
        for (a, events) in st
            .log
            .iter()
            .filter(|(a, _)| agent.is_empty() || a.as_str() == agent)
        {
            for e in events {
                let text = s(e, "text");
                let mut hit = e.clone();
                hit["agent"] = json!(a);
                if !needle.is_empty() && text.to_lowercase().contains(&needle) {
                    exact.push(hit);
                } else {
                    let shared = words(&text).intersection(&wanted).count();
                    if shared > 0 {
                        near.push((shared, hit));
                    }
                }
            }
        }
        near.sort_by_key(|a| std::cmp::Reverse(a.0));
        exact
            .into_iter()
            .chain(near.into_iter().take(6).map(|(_, h)| h))
            .take(10)
            .collect()
    }

    /// Ask the node that holds an agent and wait for its one reply.
    async fn ask(&self, subject: &str, mut payload: Value, timeout: Duration) -> Option<Value> {
        let reply = format!("reply.{}", crate::wire::random_hex(16));
        payload["reply"] = json!(reply);
        request(&self.bus, subject, payload, &reply, timeout)
            .await
            .map(|m| m.payload)
    }

    pub fn answer_approval(&self, request_id: &str, approved: bool) -> bool {
        let Some(req) = self
            .state
            .lock()
            .expect("not poisoned")
            .approvals
            .remove(request_id)
        else {
            return false;
        };
        let signature = sign(&req, approved, &self.key);
        self.bus.publish(
            "approval.response",
            json!({"request_id": request_id, "approved": approved, "signature": signature}),
        );
        self.push();
        true
    }

    pub async fn start(self: &Arc<Self>, host: &str, port: u16) -> std::io::Result<u16> {
        let listener = TcpListener::bind((host, port)).await?;
        let bound = listener.local_addr()?.port();
        let me = Arc::clone(self);
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        tokio::spawn(async move {
            // One failed accept (out of file descriptors, a reset) must not stop the page for good.
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        // past the limit, a new connection is closed at once rather than queued
                        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                            continue;
                        };
                        let me = Arc::clone(&me);
                        tokio::spawn(async move {
                            me.serve(stream).await;
                            drop(slot);
                        });
                    }
                    Err(e) => {
                        eprintln!("gateway: accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });
        if !self.sources.is_empty() {
            let me = Arc::clone(self);
            tokio::spawn(async move { me.poll_usage().await });
        }
        Ok(bound)
    }

    async fn poll_usage(self: Arc<Self>) {
        loop {
            let mut items = Vec::new();
            let mut logos = Map::new();
            for source in &self.sources {
                if let Ok((found, l)) = source.fetch().await {
                    items.extend(found); // one failing source never hides the others
                    logos.extend(l);
                }
            }
            {
                let mut st = self.state.lock().expect("not poisoned");
                st.usage = items;
                st.logos.extend(logos);
            }
            self.push();
            tokio::time::sleep(self.usage_interval).await;
        }
    }

    /// A request from a page this machine opened on itself: loopback connection, a
    /// loopback host name (no DNS rebinding) and no other site behind it. Another site's
    /// page cannot pass: its Origin differs, and a POST it could send without asking
    /// first (not JSON) is refused anyway.
    fn local_page(&self, r: &Request) -> bool {
        if !self.trust_loopback || !r.loopback {
            return false;
        }
        let host = r.headers.get("host").map(String::as_str).unwrap_or("");
        let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
        if !matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
            return false;
        }
        let same = format!("http://{host}");
        let origin_ok = r.headers.get("origin").is_none_or(|o| *o == same);
        let site_ok = r
            .headers
            .get("sec-fetch-site")
            .is_none_or(|v| v == "same-origin" || v == "none");
        origin_ok && site_ok
    }

    /// Who the request comes from. A pass presented is a guest or nothing: it never falls
    /// back to the owner's access, whatever else the request carries.
    pub fn access_of(&self, pass_token: Option<&str>, owner: bool) -> Option<Access> {
        if let Some(token) = pass_token {
            let hash = crate::passes::token_hash(token);
            let st = self.state.lock().expect("not poisoned");
            return st
                .passes
                .iter()
                .find(|p| p["token_sha"] == hash.as_str() && crate::passes::valid(p))
                .cloned()
                .map(Access::Guest);
        }
        owner.then_some(Access::Owner)
    }

    /// Who the request comes from, in this order: a pass it presents (a guest or nothing), the
    /// owner's token it presents, its session cookie, then a page this machine opened itself.
    fn access(&self, r: &Request) -> Option<Access> {
        let pass = r
            .headers
            .get("x-inari-pass")
            .or_else(|| r.query.get("pass"))
            .map(String::as_str)
            .filter(|t| !t.is_empty());
        if pass.is_some() {
            return self.access_of(pass, false);
        }
        if self.token_given(r) {
            return Some(Access::Owner);
        }
        let session = r
            .headers
            .get("cookie")
            .and_then(|c| crate::session::from_header(c))
            .and_then(|c| self.session_access(c));
        if session.is_some() {
            return session;
        }
        self.local_page(r).then_some(Access::Owner)
    }

    /// What a session cookie opens: the owner's access while the token it was opened with is
    /// still the token; a pass's while that pass is still valid (revoking it ends the session).
    fn session_access(&self, cookie: &str) -> Option<Access> {
        use crate::session::Kind;
        let (kind, hash) = self.sessions.read(cookie)?;
        match kind {
            Kind::Owner => (!self.token.is_empty()
                && hash == crate::session::owner_hash(&self.token))
            .then_some(Access::Owner),
            Kind::Pass => {
                let st = self.state.lock().expect("not poisoned");
                st.passes
                    .iter()
                    .find(|p| p["token_sha"] == hash.as_str() && crate::passes::valid(p))
                    .cloned()
                    .map(Access::Guest)
            }
        }
    }

    /// Whether what opened a long request still holds: a pass revoked or expired meanwhile
    /// ends the page's event stream at once, not only its next request.
    fn still(&self, access: &Access) -> bool {
        match access {
            Access::Owner => true,
            Access::Guest(p) => {
                let st = self.state.lock().expect("not poisoned");
                st.passes
                    .iter()
                    .any(|q| q["id"] == p["id"] && crate::passes::valid(q))
            }
        }
    }

    /// The owner's token, given in the `Authorization` header (or, for old clients, `?token=`).
    fn token_given(&self, r: &Request) -> bool {
        let header = r
            .headers
            .get("authorization")
            .map(|h| h.trim_start_matches("Bearer ").trim().to_string())
            .unwrap_or_default();
        let given = if header.is_empty() {
            r.query.get("token").cloned().unwrap_or_default()
        } else {
            header
        };
        let (a, b) = (given.as_bytes(), self.token.as_bytes());
        !self.token.is_empty()
            && a.len() == b.len()
            && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    async fn read_request(stream: &mut TcpStream) -> Option<Result<Request, u16>> {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        let end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            if buf.len() > 32 * 1024 {
                return Some(Err(400));
            }
            let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
                .await
                .ok()?
                .ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.splitn(3, ' ');
        let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
            .collect();
        let (path, query) = query_of(&target);
        let size: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let limit = if path == "/api/upload" {
            MAX_PART_BODY
        } else {
            MAX_BODY
        };
        if size > limit {
            return Some(Err(413));
        }
        let mut body = buf[end + 4..].to_vec();
        while body.len() < size {
            let n = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut chunk))
                .await
                .ok()?
                .ok()?;
            if n == 0 {
                return None;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(size);
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        Some(Ok(Request {
            method,
            path,
            query,
            headers,
            body,
            loopback: peer.is_some_and(|ip| ip.is_loopback()),
            peer,
        }))
    }

    async fn serve(self: Arc<Self>, mut stream: TcpStream) {
        let read = tokio::time::timeout(REQUEST_DEADLINE, Self::read_request(&mut stream));
        let r = match read.await.ok().flatten() {
            None => return,
            Some(Err(status)) => {
                let _ = send(&mut stream, status, "text/plain", reason(status).as_bytes()).await;
                return;
            }
            Some(Ok(r)) => r,
        };
        let _ = self.route(r, &mut stream).await;
    }

    async fn route(&self, r: Request, w: &mut TcpStream) -> std::io::Result<()> {
        let get = r.method == "GET";
        if get && r.path == "/" {
            let page = INDEX.replace("__INARI_PAGE__", page_version());
            return send(w, 200, "text/html; charset=utf-8", page.as_bytes()).await;
        }
        if get && r.path == "/kitsune" {
            let page = KITSUNE_INDEX.replace("__INARI_PAGE__", page_version());
            return send(w, 200, "text/html; charset=utf-8", page.as_bytes()).await;
        }
        if get && r.path.starts_with("/assets/") {
            let rel = r.path.trim_start_matches("/assets/");
            return match ASSETS.iter().find(|(name, _, _)| *name == rel) {
                Some((_, ctype, body)) => send_asset(w, ctype, body).await,
                None => send_json(w, 404, &json!({"error": "not found"})).await,
            };
        }
        if get && r.path.starts_with("/fox/") {
            if let Some(dir) = &self.mascot_dir {
                let rel = r.path.trim_start_matches("/fox/");
                return self
                    .serve_static(w, dir, if rel.is_empty() { "index.html" } else { rel })
                    .await;
            }
        }
        if r.method == "POST" && r.path == "/api/session" {
            return self.sign_in(&r, w).await;
        }
        if r.path == "/api/kitsune/mcp" && r.method == "POST" {
            // the agentic network's door: the owner, or a caller with the brain's own MCP token.
            // Handled before the gateway's own access gate so a token-bearing guest is not turned
            // away at 401; the brain checks its token itself.
            return self.kitsune_mcp(&r, w).await;
        }
        if r.method == "POST" && r.path == "/api/logout" {
            let end = crate::session::set_cookie(None, r.https());
            return send_with(w, 200, "application/json", b"{}", &end).await;
        }
        let Some(access) = self.access(&r) else {
            return send_json(w, 401, &json!({"error": "unauthorized"})).await;
        };
        let parts: Vec<&str> = r.path.split('/').collect(); // "", "api", "agent", id, what
        if get && r.path == "/events" {
            return self.events(w, access).await;
        }
        if get && r.path == "/api/personas" {
            let list = self
                .mascot_dir
                .as_deref()
                .map(crate::personas::list)
                .unwrap_or_default();
            let current = self.persona_of(&access);
            return send_json(w, 200, &json!({"personas": list, "current": current})).await;
        }
        if r.method == "POST" && r.path == "/api/persona" {
            if !r
                .headers
                .get("content-type")
                .is_some_and(|c| c.starts_with("application/json"))
            {
                return send_json(w, 400, &json!({"error": "JSON only"})).await;
            }
            return self.choose_persona(&access, &r, w).await;
        }
        if get && r.path == "/api/whoami" {
            let me = match &access {
                Access::Owner => json!({"role": "owner"}),
                Access::Guest(p) => json!({"role": p["role"], "pass": p["id"], "name": p["name"]}),
            };
            return send_json(w, 200, &me).await;
        }
        if get && parts.len() == 5 && parts[1] == "api" && parts[2] == "agent" {
            let agent = parts[3].to_string();
            if !self.owned(&access, &agent) {
                return send_json(w, 403, &json!({"error": "not your thread"})).await;
            }
            return match parts[4] {
                "timeline" => {
                    let log = self
                        .state
                        .lock()
                        .expect("not poisoned")
                        .log
                        .get(&agent)
                        .cloned()
                        .unwrap_or_default();
                    let after = r.query.get("after").map(String::as_str).unwrap_or("");
                    send_json(w, 200, &timeline_since(log, after)).await
                }
                "files" => {
                    let reply = self
                        .ask(
                            "agent.files.request",
                            json!({"agent": agent}),
                            Duration::from_secs(5),
                        )
                        .await;
                    send_json(w, 200, &reply.unwrap_or(json!({"files": []}))).await
                }
                "file" => {
                    let path = r.query.get("path").cloned().unwrap_or_default();
                    let reply = self
                        .ask(
                            "agent.file.request",
                            json!({"agent": agent, "path": path}),
                            Duration::from_secs(5),
                        )
                        .await;
                    send_json(w, 200, &reply.unwrap_or(json!({"text": ""}))).await
                }
                _ => send_json(w, 404, &json!({"error": "not found"})).await,
            };
        }
        if get && r.path == "/api/openfox/library" {
            // part of a core's OpenFox library: its agents, skills, MCP servers… (the owner's)
            if !matches!(access, Access::Owner) {
                return send_json(w, 403, &json!({"error": "the core's owner sees this"})).await;
            }
            let q = |k: &str| r.query.get(k).cloned().unwrap_or_default();
            if !q("node")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
                || q("node").is_empty()
            {
                return send_json(w, 400, &json!({"error": "bad request"})).await;
            }
            let reply = self
                .ask(
                    &format!("node.{}.openfox.library", q("node")),
                    json!({"harness": q("harness"), "kind": q("kind"), "id": q("id")}),
                    Duration::from_secs(15),
                )
                .await;
            return send_json(
                w,
                200,
                &reply.unwrap_or(json!({"ok": false, "error": "the core did not answer"})),
            )
            .await;
        }
        if get && r.path.starts_with("/api/logo/") {
            return self.logo(w, r.path.rsplit('/').next().unwrap_or("")).await;
        }
        if r.path.starts_with("/api/kitsune") {
            // the brain's own data: the core's owner only
            if !matches!(access, Access::Owner) {
                return send_json(w, 403, &json!({"error": "the core's owner sees this"})).await;
            }
            return self.kitsune_api(&r, w).await;
        }
        if get && r.path == "/api/search" {
            let q = r.query.get("q").cloned().unwrap_or_default();
            let agent = r.query.get("agent").cloned().unwrap_or_default();
            let hits: Vec<Value> = self
                .search(&q, &agent)
                .into_iter()
                .filter(|h| {
                    self.owned(
                        &access,
                        h.get("agent").and_then(Value::as_str).unwrap_or(""),
                    )
                })
                .collect();
            return send_json(w, 200, &json!({"hits": hits})).await;
        }
        if r.method == "POST" {
            // only JSON: a form or text/plain POST is what another site could send unasked
            if !r
                .headers
                .get("content-type")
                .is_some_and(|c| c.starts_with("application/json"))
            {
                return send_json(w, 400, &json!({"error": "JSON only"})).await;
            }
            let data: Value =
                match serde_json::from_slice(if r.body.is_empty() { b"{}" } else { &r.body }) {
                    Ok(v @ Value::Object(_)) => v,
                    _ => return send_json(w, 400, &json!({"error": "bad request"})).await,
                };
            let data = match self.guest_rules(&access, &r.path, data) {
                Ok(d) => d,
                Err((status, why)) => return send_json(w, status, &json!({"error": why})).await,
            };
            if let Some((status, reply)) = self.post(&r.path, &data).await {
                return send_json(w, status, &reply).await;
            }
        }
        send_json(w, 404, &json!({"error": "not found"})).await
    }

    /// Who a preference belongs to: the owner, or a pass by its id.
    fn who(access: &Access) -> String {
        match access.pass_id() {
            None => "owner".into(),
            Some(id) => format!("pass:{id}"),
        }
    }

    /// The persona this person chose, while FoxBot still publishes it; else the original FoxBot.
    fn persona_of(&self, access: &Access) -> String {
        let chosen = self.personas.get(&Self::who(access));
        match self.mascot_dir.as_deref() {
            Some(dir) if crate::personas::published(dir).contains(&chosen) => chosen,
            _ => String::new(),
        }
    }

    /// `POST /api/persona {"persona": …}`: the mascot this person shows; empty for the original
    /// FoxBot. Only a persona FoxBot's folder holds is accepted.
    async fn choose_persona(
        &self,
        access: &Access,
        r: &Request,
        w: &mut TcpStream,
    ) -> std::io::Result<()> {
        let persona = serde_json::from_slice::<Value>(&r.body)
            .ok()
            .and_then(|v| v.get("persona").and_then(Value::as_str).map(String::from))
            .unwrap_or_default();
        let known = self
            .mascot_dir
            .as_deref()
            .map(crate::personas::list)
            .unwrap_or_default();
        if !persona.is_empty() && !known.iter().any(|p| p["id"] == persona.as_str()) {
            return send_json(w, 400, &json!({"error": "unknown persona"})).await;
        }
        if let Err(e) = self.personas.set(&Self::who(access), &persona) {
            return send_json(w, 500, &json!({"error": e.to_string()})).await;
        }
        self.push(); // every page of this person follows
        send_json(w, 200, &json!({"persona": persona})).await
    }

    /// `POST /api/session {"secret": …}`: the owner's token or a pass, exchanged for a session
    /// cookie. Answers like `/api/whoami`. Wrong secrets make the address wait (429).
    async fn sign_in(&self, r: &Request, w: &mut TcpStream) -> std::io::Result<()> {
        use crate::session::{owner_hash, set_cookie, Kind};
        if !r
            .headers
            .get("content-type")
            .is_some_and(|c| c.starts_with("application/json"))
        {
            return send_json(w, 400, &json!({"error": "JSON only"})).await;
        }
        let wait = self.sessions.wait(r.peer);
        if !wait.is_zero() {
            let body = json!({"error": "too many attempts", "retry_after": wait.as_secs() + 1});
            return send_json(w, 429, &body).await;
        }
        let secret = serde_json::from_slice::<Value>(&r.body)
            .ok()
            .and_then(|v| {
                v.get("secret")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .map(String::from)
            })
            .unwrap_or_default();
        let pass = (!secret.is_empty())
            .then(|| self.access_of(Some(&secret), false))
            .flatten();
        let given = secret.as_bytes();
        let owner = !self.token.is_empty()
            && given.len() == self.token.len()
            && given
                .iter()
                .zip(self.token.as_bytes())
                .fold(0u8, |acc, (x, y)| acc | (x ^ y))
                == 0;
        let (cookie, me) = match (&pass, owner) {
            (Some(Access::Guest(p)), _) => (
                self.sessions
                    .issue(Kind::Pass, &crate::passes::token_hash(&secret)),
                json!({"role": p["role"], "pass": p["id"], "name": p["name"]}),
            ),
            (_, true) => (
                self.sessions.issue(Kind::Owner, &owner_hash(&self.token)),
                json!({"role": "owner"}),
            ),
            _ => {
                self.sessions.attempt(r.peer, false);
                return send_json(w, 401, &json!({"error": "unknown token or pass"})).await;
            }
        };
        self.sessions.attempt(r.peer, true);
        let body = serde_json::to_vec(&me).expect("serializes");
        send_with(
            w,
            200,
            "application/json",
            &body,
            &set_cookie(Some(&cookie), r.https()),
        )
        .await
    }

    /// A thread is the access's own: always for the owner; for a pass, named after it or
    /// shared with it (its `pass` label).
    pub fn owned(&self, access: &Access, agent: &str) -> bool {
        if access.owns(agent) {
            return true;
        }
        let Some(id) = access.pass_id() else {
            return false;
        };
        let st = self.state.lock().expect("not poisoned");
        st.telemetry
            .agents
            .get(agent)
            .and_then(|a| a.get("pass"))
            .and_then(Value::as_str)
            == Some(id)
    }

    /// What a pass holder may do, and how its requests are bound to its pass. The owner
    /// passes through unchanged.
    fn guest_rules(
        &self,
        access: &Access,
        path: &str,
        mut d: Value,
    ) -> Result<Value, (u16, &'static str)> {
        let Access::Guest(pass) = access else {
            return Ok(d);
        };
        let id = pass["id"].as_str().unwrap_or("");
        let allowed: Vec<&str> = pass
            .get("harnesses")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let harness_ok = |h: &str| allowed.contains(&h);
        match path {
            "/api/agent" => {
                let name: String = s(&d, "thread_id")
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .take(40)
                    .collect();
                let name = if name.is_empty() {
                    crate::wire::random_hex(4)
                } else {
                    name
                };
                d["thread_id"] = json!(format!(
                    "{id}-{}",
                    name.trim_start_matches(&format!("{id}-"))
                ));
                let harness = s(&d, "harness");
                let harness = if harness.is_empty() {
                    allowed.first().copied().unwrap_or("").to_string()
                } else {
                    harness
                };
                if !harness_ok(&harness) {
                    return Err((403, "this harness is not allowed by the pass"));
                }
                d["harness"] = json!(harness);
                d["pass"] = json!(id);
                Ok(d)
            }
            "/api/message"
            | "/api/upload"
            | "/api/model"
            | "/api/mode"
            | "/api/archive"
            | "/api/cancel"
            | "/api/rename"
            | "/api/pin"
            | "/api/openfox/thread" => {
                // what runs commands or OpenFox sessions Inari does not supervise is the owner's
                let unsupervised = (path == "/api/message"
                    && d.get("workflow").is_some_and(|w| !w.is_null()))
                    || (path == "/api/openfox/thread"
                        && OWNER_ONLY.contains(&s(&d, "action").as_str()));
                if !self.owned(access, &s(&d, "agent")) {
                    Err((403, "not your thread"))
                } else if unsupervised {
                    Err((403, "the core's owner does this"))
                } else {
                    Ok(d)
                }
            }
            "/api/harness" => {
                if self.owned(access, &s(&d, "agent")) && harness_ok(&s(&d, "harness")) {
                    Ok(d)
                } else {
                    Err((403, "not allowed by the pass"))
                }
            }
            "/api/approval" => {
                // under supervision the owner answers the agents' requests, not the guest
                let req = self
                    .state
                    .lock()
                    .expect("not poisoned")
                    .approvals
                    .get(&s(&d, "request_id"))
                    .cloned();
                let mine = req.is_some_and(|r| {
                    self.owned(
                        access,
                        r.get("agent_id").and_then(Value::as_str).unwrap_or(""),
                    )
                });
                if mine && pass["role"] == "collaboration" {
                    Ok(d)
                } else {
                    Err((403, "the core's owner answers this"))
                }
            }
            _ => Err((403, "not with a pass")),
        }
    }

    /// The actions; None for an unknown path, 400 when a field is missing.
    async fn post(&self, path: &str, d: &Value) -> Option<(u16, Value)> {
        let need = |keys: &[&str]| keys.iter().all(|k| d.get(*k).is_some_and(|v| !v.is_null()));
        let bad = Some((400, json!({"error": "bad request"})));
        let ok = Some((200, json!({"ok": true})));
        match path {
            "/api/teleport" => {
                if !need(&["agent", "source", "target"]) {
                    return bad;
                }
                let req = crate::wire::random_hex(16);
                self.bus.publish("teleport.request", json!({"request": req, "agent": s(d, "agent"), "source": s(d, "source"), "target": s(d, "target")}));
                Some((200, json!({"request": req})))
            }
            "/api/exec" => {
                if !need(&["agent", "cmd"]) {
                    return bad;
                }
                let reply = self
                    .ask(
                        "agent.exec.request",
                        json!({"agent": s(d, "agent"), "cmd": s(d, "cmd")}),
                        Duration::from_secs(75),
                    )
                    .await;
                Some((
                    200,
                    reply.unwrap_or(json!({"ok": false, "output": "no answer"})),
                ))
            }
            "/api/upload" => {
                if !need(&["agent", "path", "b64"]) {
                    return bad;
                }
                Some(self.upload(d).await)
            }
            "/api/harness" => {
                if !need(&["agent", "harness"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.harness", s(d, "agent")),
                    json!({"harness": s(d, "harness")}),
                );
                ok
            }
            "/api/auto" => {
                // owner only: a pass is never routed here (see the pass allow-list above)
                if !need(&["agent"]) {
                    return bad;
                }
                let on = d.get("auto").and_then(Value::as_bool).unwrap_or(true);
                self.bus.publish(
                    &format!("agent.{}.label", s(d, "agent")),
                    json!({"key": "auto", "value": if on { "1" } else { "" }}),
                );
                ok
            }
            "/api/cancel" => {
                if !need(&["agent"]) {
                    return bad;
                }
                self.bus
                    .publish(&format!("agent.{}.cancel", s(d, "agent")), json!({}));
                ok
            }
            "/api/rename" => {
                if !need(&["agent", "title"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.rename", s(d, "agent")),
                    json!({"title": s(d, "title")}),
                );
                ok
            }
            "/api/pin" => {
                if !need(&["agent"]) {
                    return bad;
                }
                let on = d.get("pinned").and_then(Value::as_bool).unwrap_or(true);
                self.bus.publish(
                    &format!("agent.{}.label", s(d, "agent")),
                    json!({"key": "pinned", "value": if on { "1" } else { "" }}),
                );
                ok
            }
            "/api/mode" => {
                if !need(&["agent", "mode"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.mode", s(d, "agent")),
                    json!({"mode": s(d, "mode")}),
                );
                ok
            }
            "/api/model" => {
                if !need(&["agent", "model"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.model", s(d, "agent")),
                    json!({"model": s(d, "model")}),
                );
                ok
            }
            "/api/agent" => {
                let mut p = Map::new();
                for k in ["thread_id", "node", "harness", "model", "pass"] {
                    if let Some(v) = d.get(k) {
                        p.insert(k.into(), v.clone());
                    }
                }
                p.insert("title".into(), json!(s(d, "title")));
                self.bus.publish("agent.create", Value::Object(p));
                Some((200, json!({"ok": true, "thread_id": s(d, "thread_id")})))
            }
            "/api/message" => {
                if !need(&["agent", "text"]) {
                    return bad;
                }
                let mut p = json!({"text": s(d, "text")});
                // a model given here is set before the turn reads it, in the same task
                if !s(d, "model").is_empty() {
                    p["model"] = json!(s(d, "model"));
                }
                if let Some(a) = d.get("attachments").filter(|a| a.is_array()) {
                    p["attachments"] = a.clone();
                }
                // an OpenFox workflow launched with it (the node checks it)
                if let Some(w) = d.get("workflow").filter(|w| w.is_object()) {
                    p["workflow"] = w.clone();
                }
                self.bus.publish(&format!("agent.{}.run", s(d, "agent")), p);
                ok
            }
            "/api/fork" => {
                if !need(&["agent", "new_thread"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.fork", s(d, "agent")),
                    json!({"new_thread": s(d, "new_thread"), "upto": d.get("upto")}),
                );
                ok
            }
            "/api/merge" => {
                if !need(&["target", "source"]) {
                    return bad;
                }
                self.bus.publish(
                    "agent.merge",
                    json!({"target": s(d, "target"), "source": s(d, "source")}),
                );
                ok
            }
            "/api/clone" => {
                if !need(&["agent", "harness"]) {
                    return bad;
                }
                let (agent, harness) = (s(d, "agent"), s(d, "harness"));
                let new = format!("{agent}-{harness}-{}", crate::wire::random_hex(2));
                let title = self
                    .state
                    .lock()
                    .expect("not poisoned")
                    .telemetry
                    .agents
                    .get(&agent)
                    .and_then(|a| a.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or(&agent)
                    .to_string();
                let label: String = s(d, "label").chars().take(30).collect();
                let title = if label.is_empty() {
                    title
                } else {
                    format!("{title} · {label}")
                };
                // which core thinks with that harness: the thread's own, or another one of the mesh
                let (here, there) = {
                    let st = self.state.lock().expect("not poisoned");
                    let here = st
                        .telemetry
                        .agents
                        .get(&agent)
                        .and_then(|a| a.get("node"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let has = |n: &Value| {
                        n.pointer(&format!("/capabilities/harnesses/{harness}"))
                            .is_some()
                    };
                    let home_has = st
                        .nodes
                        .iter()
                        .any(|n| n["node_id"] == here.as_str() && has(n));
                    let there = if home_has {
                        here.clone()
                    } else {
                        st.nodes
                            .iter()
                            .find(|n| has(n))
                            .and_then(|n| n["node_id"].as_str())
                            .unwrap_or("")
                            .to_string()
                    };
                    (here, there)
                };
                if there.is_empty() {
                    return Some((
                        404,
                        json!({"error": "no core of the mesh has this harness"}),
                    ));
                }
                if there == here {
                    self.bus.publish(
                        &format!("agent.{agent}.fork"),
                        json!({"new_thread": new, "harness": harness, "title": title}),
                    );
                } else {
                    // copy here, carry the copy to the core that has the harness, switch it there
                    let bus = Arc::clone(&self.bus);
                    let (new2, harness2) = (new.clone(), harness.clone());
                    tokio::spawn(async move {
                        clone_across(bus, agent, new2, title, harness2, here, there).await
                    });
                }
                Some((200, json!({"ok": true, "thread_id": new})))
            }
            "/api/delete" => {
                if !need(&["agent"]) {
                    return bad;
                }
                self.bus
                    .publish(&format!("agent.{}.delete", s(d, "agent")), json!({}));
                ok
            }
            "/api/share" => {
                if !need(&["agent"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("agent.{}.label", s(d, "agent")),
                    json!({"key": "pass", "value": s(d, "pass")}),
                );
                ok
            }
            "/api/archive" => {
                if !need(&["agent"]) {
                    return bad;
                }
                let value = if d.get("archived").and_then(Value::as_bool).unwrap_or(true) {
                    "1"
                } else {
                    ""
                };
                self.bus.publish(
                    &format!("agent.{}.label", s(d, "agent")),
                    json!({"key": "archived", "value": value}),
                );
                ok
            }
            "/api/pass" => {
                let name: String = s(d, "name").trim().chars().take(40).collect();
                let role = s(d, "role");
                if name.is_empty() || !crate::passes::ROLES.contains(&role.as_str()) {
                    return bad;
                }
                let (id, token) = (crate::wire::random_hex(3), crate::enroll::new_code());
                self.bus.publish("pass.create", json!({"id": id, "name": name, "role": role,
                    "token_sha": crate::passes::token_hash(&token), "harnesses": d.get("harnesses").cloned().unwrap_or(json!([])),
                    "until": d.get("until").and_then(Value::as_f64).unwrap_or(0.0)}));
                Some((200, json!({"id": id, "token": token})))
            }
            "/api/pass/revoke" => {
                if !need(&["id"]) {
                    return bad;
                }
                self.bus.publish("pass.revoke", json!({"id": s(d, "id")}));
                ok
            }
            "/api/invite" => {
                let node = s(d, "node_id");
                if node.is_empty()
                    || node.len() > 32
                    || !node.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    return bad;
                }
                let code = crate::enroll::new_code();
                self.bus
                    .publish("mesh.invite", json!({"code": code, "node_id": node}));
                Some((
                    200,
                    json!({"code": code, "node_id": node, "expires_in": crate::enroll::INVITE_TTL.as_secs()}),
                ))
            }
            "/api/update" => {
                if !need(&["node", "harness"]) {
                    return bad;
                }
                self.bus.publish(
                    &format!("node.{}.update", s(d, "node")),
                    json!({"harness": s(d, "harness")}),
                );
                ok
            }
            "/api/openfox/thread" => {
                // something done on a thread's OpenFox panel; the thread's node does it
                const ACTIONS: [&str; 27] = [
                    "git_clone",
                    "git_publish",
                    "agent",
                    "effort",
                    "danger",
                    "criteria",
                    "answer",
                    "pause",
                    "resume",
                    "dequeue",
                    "compact",
                    "retry_now",
                    "retry",
                    "refresh",
                    "workflow_exit",
                    "task_add",
                    "task_move",
                    "task_remove",
                    "dev_config",
                    "dev_start",
                    "dev_stop",
                    "dev_restart",
                    "dev_logs",
                    "process_stop",
                    "checkout",
                    "branch_new",
                    "instructions",
                ];
                if !need(&["agent", "action"]) || !ACTIONS.contains(&s(d, "action").as_str()) {
                    return bad;
                }
                self.bus
                    .publish(&format!("agent.{}.openfox", s(d, "agent")), d.clone());
                ok
            }
            "/api/engine" => {
                // a local engine's setting, a provider of it (its key goes to the core only)
                let action = s(d, "action");
                let node = s(d, "node");
                if !need(&["node", "harness", "action"])
                    || ![
                        "set",
                        "provider_add",
                        "provider_remove",
                        "provider_test",
                        "forge_add",
                        "forge_remove",
                        "forge_test",
                    ]
                    .contains(&action.as_str())
                    || !node
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
                {
                    return bad;
                }
                let mut p = d.clone();
                if let Some(o) = p.as_object_mut() {
                    o.remove("node");
                    o.remove("action");
                }
                self.bus.publish(&format!("node.{node}.engine.{action}"), p);
                ok
            }
            "/api/harnesses" => {
                // a harness added, changed, taken away or tried on a core (Claude Code, Codex…)
                let action = s(d, "action");
                let node = s(d, "node");
                if !need(&["node", "action"])
                    || !["add", "remove", "test"].contains(&action.as_str())
                    || !node
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
                {
                    return bad;
                }
                let mut p = d.clone();
                if let Some(o) = p.as_object_mut() {
                    o.remove("node");
                    o.remove("action");
                }
                self.bus
                    .publish(&format!("node.{node}.harness.{action}"), p);
                ok
            }
            "/api/openfox" => {
                // an OpenFox setting from the interface: its address, agent, limit; sign-in; the
                // active provider; a check. The node does it and says how it went.
                let action = s(d, "action");
                if !need(&["node", "harness", "action"])
                    || !([
                        "set",
                        "login",
                        "activate",
                        "check",
                        "provider_test",
                        "provider_add",
                        "provider_remove",
                        "provider_refresh",
                    ]
                    .contains(&action.as_str())
                        || crate::openfox_library::ACTIONS.contains(&action.as_str()))
                {
                    return bad;
                }
                let mut p = d.clone();
                if let Some(o) = p.as_object_mut() {
                    o.remove("node");
                    o.remove("action");
                }
                self.bus
                    .publish(&format!("node.{}.openfox.{action}", s(d, "node")), p);
                ok
            }
            "/api/update/check" => {
                let node = Some(s(d, "node"))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "all".into());
                self.bus
                    .publish(&format!("node.{node}.update.check"), json!({}));
                ok
            }
            "/api/approval" => {
                if !need(&["request_id", "approved"]) {
                    return bad;
                }
                let done = self.answer_approval(
                    &s(d, "request_id"),
                    d["approved"].as_bool().unwrap_or(false),
                );
                Some((if done { 200 } else { 404 }, json!({"ok": done})))
            }
            _ => None,
        }
    }

    /// A whole small file, or one part of a larger one; the node gets the file once complete.
    async fn upload(&self, d: &Value) -> (u16, Value) {
        let b64 = base64::engine::general_purpose::STANDARD;
        let mut whole = s(d, "b64");
        if let Some(id) = d.get("upload_id").and_then(Value::as_str) {
            let (Some(store), Ok(bytes)) = (&self.uploads, b64.decode(&whole)) else {
                return (400, json!({"ok": false, "error": "bad part"}));
            };
            let offset = d.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let total = d
                .get("total")
                .and_then(Value::as_u64)
                .unwrap_or(bytes.len() as u64);
            match store.put(id, offset, total, &bytes) {
                Ok(Progress::Partial { received }) => {
                    return (
                        200,
                        json!({"ok": true, "received": received, "total": total}),
                    )
                }
                Ok(Progress::Complete(_)) => match store.take(id) {
                    Ok(data) => whole = b64.encode(data),
                    Err(e) => return (400, json!({"ok": false, "error": e.to_string()})),
                },
                Err(e) => return (400, json!({"ok": false, "error": e.to_string()})),
            }
        }
        let reply = self
            .ask(
                "agent.upload.request",
                json!({"agent": s(d, "agent"), "path": s(d, "path"), "b64": whole}),
                Duration::from_secs(30),
            )
            .await;
        (
            200,
            reply.unwrap_or(json!({"ok": false, "error": "no answer"})),
        )
    }

    async fn serve_static(&self, w: &mut TcpStream, root: &Path, rel: &str) -> std::io::Result<()> {
        let (Ok(base), Ok(target)) = (root.canonicalize(), root.join(rel).canonicalize()) else {
            return send_json(w, 404, &json!({"error": "not found"})).await;
        };
        if !target.starts_with(&base) || !target.is_file() {
            return send_json(w, 404, &json!({"error": "not found"})).await;
        }
        let mut file = tokio::fs::File::open(&target).await?;
        let size = file.metadata().await?.len();
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {size}\r\nCache-Control: max-age=3600\r\n\
             X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
            mime_of(&target.to_string_lossy())
        );
        w.write_all(head.as_bytes()).await?;
        tokio::io::copy(&mut file, w).await?; // streamed: sprite sets are large
        w.flush().await
    }

    /// Provider logos live on the usage source's server: fetched once, served from here.
    /// Run something on the brain off the async runtime (it does blocking I/O: sqlite, and HTTP to
    /// the voice servers). None when this core holds no brain.
    async fn with_kitsune<T, F>(&self, f: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut kitsune::Kitsune) -> T + Send + 'static,
    {
        let k = self.kitsune.clone()?;
        tokio::task::spawn_blocking(move || f(&mut k.lock().expect("not poisoned")))
            .await
            .ok()
    }

    /// The brain's door for the agentic network: a JSON-RPC MCP request. The owner may always use
    /// it; another caller (n8n, Home Assistant) needs the brain's own MCP token, which it carries
    /// as a Bearer header or an `_auth` argument — the brain checks it, so no owner pass is needed.
    async fn kitsune_mcp(&self, r: &Request, w: &mut TcpStream) -> std::io::Result<()> {
        let Some(request) = serde_json::from_slice::<Value>(&r.body)
            .ok()
            .filter(Value::is_object)
        else {
            return send_json(w, 400, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "not JSON"}})).await;
        };
        let owner = matches!(self.access(r), Some(Access::Owner));
        // put the Bearer token (if any) where the brain's handler looks for it
        let mut request = request;
        if !owner {
            let bearer = r
                .headers
                .get("authorization")
                .and_then(|a| a.strip_prefix("Bearer "))
                .map(str::to_string);
            if let Some(tok) = bearer {
                if let Some(args) = request
                    .pointer_mut("/params/arguments")
                    .and_then(Value::as_object_mut)
                {
                    args.entry("_auth").or_insert(json!(tok));
                }
            }
        }
        let reply = self
            .with_kitsune(move |k| {
                // a guest may only reach the brain when it has a token set (else the owner only)
                if !owner && k.settings().mcp.token.is_empty() {
                    return Some(json!({"jsonrpc": "2.0", "id": request.get("id").cloned().unwrap_or(Value::Null), "error": {"code": -32001, "message": "this core's Kitsune is not open to the network"}}));
                }
                k.mcp(&request)
            })
            .await;
        match reply {
            Some(Some(v)) => send_json(w, 200, &v).await,
            Some(None) => send(w, 204, "application/json", b"").await, // a notification: no reply
            None => send_json(w, 503, &json!({"error": "this core holds no Kitsune"})).await,
        }
    }

    /// The owner's own door to the brain: its settings, a note kept or found, a reminder, the
    /// voice. Everything configurable, nothing assumed.
    async fn kitsune_api(&self, r: &Request, w: &mut TcpStream) -> std::io::Result<()> {
        if self.kitsune.is_none() {
            return send_json(w, 503, &json!({"error": "this core holds no Kitsune"})).await;
        }
        let b64 = base64::engine::general_purpose::STANDARD;
        // GET /api/kitsune → how it is; GET /api/kitsune/audio?path= → the sealed audio, opened
        if r.method == "GET" && r.path == "/api/kitsune/audio" {
            let rel = r.query.get("path").cloned().unwrap_or_default();
            let got = self
                .with_kitsune(move |k| k.vault().get(&rel).map(|b| (b,)))
                .await;
            return match got {
                Some(Ok((bytes,))) => send(w, 200, "application/octet-stream", &bytes).await,
                _ => send_json(w, 404, &json!({"error": "no such audio"})).await,
            };
        }
        if r.method == "GET" {
            let mascot = self.mascot_dir.is_some();
            let status = self
                .with_kitsune(move |k| {
                    let s = k.settings();
                    json!({"settings": s, "asr_ready": s.asr.ready(), "tts_ready": s.tts.ready(),
                           "notes": k.store().notes_between(0.0, f64::MAX).map(|n| n.len()).unwrap_or(0),
                           "triggers": k.store().list_triggers().map(|t| t.len()).unwrap_or(0),
                           "mascot": mascot})
                })
                .await
                .unwrap_or(json!({}));
            return send_json(w, 200, &status).await;
        }
        // POST /api/kitsune {action, …}
        let d: Value = serde_json::from_slice(if r.body.is_empty() { b"{}" } else { &r.body })
            .unwrap_or(Value::Null);
        let action = s(&d, "action");
        let result: Option<std::result::Result<Value, String>> = match action.as_str() {
            "set_settings" => {
                let settings: std::result::Result<kitsune::Settings, _> =
                    serde_json::from_value(d.get("settings").cloned().unwrap_or(Value::Null));
                match settings {
                    Ok(settings) => {
                        self.with_kitsune(move |k| {
                            k.set_settings(settings)
                                .map(|_| json!({"ok": true}))
                                .map_err(|e| e.to_string())
                        })
                        .await
                    }
                    Err(e) => Some(Err(format!("bad settings: {e}"))),
                }
            }
            "capture_text" => {
                let text = s(&d, "text");
                let tags: Vec<String> = d
                    .get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect();
                self.with_kitsune(move |k| {
                    k.capture_text(&text, tags)
                        .map(|id| json!({"id": id}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "ingest_voice" => {
                let audio = b64.decode(s(&d, "audio")).unwrap_or_default();
                let place = s(&d, "place");
                let at = match (
                    d.get("lat").and_then(Value::as_f64),
                    d.get("lon").and_then(Value::as_f64),
                ) {
                    (Some(la), Some(lo)) => Some((la, lo)),
                    _ => None,
                };
                if audio.is_empty() {
                    Some(Err("no audio (base64 in the audio field)".into()))
                } else {
                    self.with_kitsune(move |k| {
                        k.ingest_voice(&audio, &place, at)
                            .map(|id| json!({"id": id}))
                            .map_err(|e| e.to_string())
                    })
                    .await
                }
            }
            "search" => {
                let query = s(&d, "query");
                let opts = kitsune::SearchOptions {
                    k: d.get("k")
                        .and_then(Value::as_u64)
                        .unwrap_or(10)
                        .clamp(1, 100) as usize,
                    filters: serde_json::from_value(d.get("filters").cloned().unwrap_or(json!({})))
                        .unwrap_or_default(),
                    ..Default::default()
                };
                self.with_kitsune(move |k| {
                    let model = k.settings().embed_model.clone();
                    k.store()
                        .search(&query, None, &model, &opts)
                        .map(|hits| json!({"hits": hits}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "speak" => {
                let text = s(&d, "text");
                let note_id = d.get("note_id").and_then(Value::as_i64);
                self.with_kitsune(move |k| {
                    k.speak(&text, note_id)
                        .map(|(audio, mime, path)| json!({"path": path, "mime": mime, "audio": base64::engine::general_purpose::STANDARD.encode(audio)}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "create_trigger" => {
                let kind = s(&d, "kind");
                let spec = d.get("spec").cloned().unwrap_or(json!({}));
                let note_id = d.get("note_id").and_then(Value::as_i64);
                let next_at = d.get("next_at").and_then(Value::as_f64);
                self.with_kitsune(move |k| {
                    k.store()
                        .create_trigger(&kind, note_id, &spec, next_at)
                        .map(|id| json!({"id": id}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "list_triggers" => {
                self.with_kitsune(|k| {
                    k.store()
                        .list_triggers()
                        .map(|t| json!({"triggers": t}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "delete_trigger" => {
                let id = d.get("id").and_then(Value::as_i64).unwrap_or(-1);
                self.with_kitsune(move |k| {
                    k.store()
                        .delete_trigger(id)
                        .map(|gone| json!({"removed": gone}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "get_note" => {
                let id = d.get("id").and_then(Value::as_i64).unwrap_or(-1);
                self.with_kitsune(move |k| {
                    k.store()
                        .get_note(id)
                        .map(|n| json!({"note": n}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "correct_note" => {
                let id = d.get("id").and_then(Value::as_i64).unwrap_or(-1);
                let text = s(&d, "text");
                self.with_kitsune(move |k| {
                    k.store()
                        .correct_note(id, &text)
                        .map(|ok| json!({"ok": ok}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "delete_note" => {
                let id = d.get("id").and_then(Value::as_i64).unwrap_or(-1);
                self.with_kitsune(move |k| {
                    k.store()
                        .delete_note(id)
                        .map(|gone| json!({"removed": gone}))
                        .map_err(|e| e.to_string())
                })
                .await
            }
            other => Some(Err(format!("unknown Kitsune action {other:?}"))),
        };
        match result {
            Some(Ok(v)) => send_json(w, 200, &v).await,
            Some(Err(e)) => send_json(w, 400, &json!({"error": e})).await,
            None => send_json(w, 503, &json!({"error": "this core holds no Kitsune"})).await,
        }
    }

    async fn logo(&self, w: &mut TcpStream, family: &str) -> std::io::Result<()> {
        let (cached, url) = {
            let st = self.state.lock().expect("not poisoned");
            (
                st.logo_cache.get(family).cloned(),
                st.logos
                    .get(family)
                    .and_then(Value::as_str)
                    .map(String::from),
            )
        };
        let found = match (cached, url) {
            (Some(hit), _) => Some(hit),
            (None, Some(url)) => match http("GET", &url, None, Duration::from_secs(10)).await {
                Ok((200, raw)) => {
                    let entry = (
                        if raw.starts_with(b"\x89PNG\r") {
                            "image/png"
                        } else {
                            "image/svg+xml"
                        },
                        raw,
                    );
                    self.state
                        .lock()
                        .expect("not poisoned")
                        .logo_cache
                        .insert(family.into(), entry.clone());
                    Some(entry)
                }
                _ => None,
            },
            _ => None,
        };
        match found {
            Some((ctype, raw)) => send(w, 200, ctype, &raw).await,
            None => send_json(w, 404, &json!({"error": "no logo"})).await,
        }
    }

    async fn events(&self, w: &mut TcpStream, access: Access) -> std::io::Result<()> {
        let mut rx = self.clients.subscribe();
        let mut drafts = self.drafts.subscribe();
        let mut panels = self.panels.subscribe();
        w.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n").await?;
        w.write_all(format!("data: {}\n\n", self.snapshot_for(&access)).as_bytes())
            .await?;
        // the answers being written right now, for a page that opens in the middle of one
        let now: Vec<(String, String)> = self
            .state
            .lock()
            .expect("not poisoned")
            .drafts
            .iter()
            .map(|(a, t)| (a.clone(), t.clone()))
            .collect();
        for (agent, text) in now.into_iter().filter(|(a, _)| self.owned(&access, a)) {
            w.write_all(draft_frame(&agent, &text).as_bytes()).await?;
        }
        let shown: Vec<(String, Value)> = self
            .state
            .lock()
            .expect("not poisoned")
            .panels
            .iter()
            .map(|(a, p)| (a.clone(), p.clone()))
            .collect();
        for (agent, panel) in shown.into_iter().filter(|(a, _)| self.owned(&access, a)) {
            w.write_all(panel_frame(&agent, &panel).as_bytes()).await?;
        }
        w.flush().await?;
        loop {
            let tick = tokio::time::sleep(Duration::from_secs(15));
            let frame = tokio::select! {
                got = rx.recv() => match got {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        format!("data: {}\n\n", self.snapshot_for(&access))
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                got = panels.recv() => match got {
                    Ok((agent, panel)) if self.owned(&access, &agent) => panel_frame(&agent, &panel),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                got = drafts.recv() => match got {
                    Ok((agent, text)) if self.owned(&access, &agent) => draft_frame(&agent, &text),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue, // the next one carries the whole text
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                },
                _ = tick => ": keep-alive\n\n".to_string(),
            };
            if !self.still(&access) {
                return Ok(()); // revoked: nothing more goes to this page
            }
            w.write_all(frame.as_bytes()).await?;
            w.flush().await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_gets_only_what_came_since_its_last_event() {
        let log: Vec<Value> = ["a", "b", "c"].iter().map(|id| json!({"id": id})).collect();
        let since = timeline_since(log.clone(), "b");
        assert_eq!(since["events"], json!([{"id": "c"}]));
        assert_eq!(
            (since["from"].as_u64(), since["count"].as_u64()),
            (Some(2), Some(3))
        );
        assert_eq!(timeline_since(log.clone(), "c")["events"], json!([]));
        let whole = timeline_since(log.clone(), "gone");
        assert_eq!(
            (
                whole["from"].as_u64(),
                whole["events"].as_array().map(Vec::len)
            ),
            (Some(0), Some(3))
        );
        assert_eq!(timeline_since(log, "")["from"].as_u64(), Some(0));
    }

    #[test]
    fn queries_are_decoded() {
        let (path, q) = query_of("/api/agent/a%20b/file?path=docs%2Fcv.pdf&q=hello+world&x");
        assert_eq!(path, "/api/agent/a b/file");
        assert_eq!(q["path"], "docs/cv.pdf");
        assert_eq!(q["q"], "hello world");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }
}
