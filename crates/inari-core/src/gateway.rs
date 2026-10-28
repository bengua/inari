//! HTTP/SSE gateway between a browser (desktop or phone) and the bus, as the Python
//! reference (`inari_core.gateway`): it gathers the mesh into one snapshot, pushes it
//! as server-sent events, serves the web interface, and turns the person's actions
//! into bus messages. It holds the approval key; browsers present a bearer token.
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

const INDEX: &str = include_str!("../../../src/inari_core/web/index.html");

/// What the page loads besides itself, under `/assets/`: its style, its script, its fonts.
/// Public like the page (they hold no data), and cached: their URLs carry the page's version.
const ASSETS: &[(&str, &str, &[u8])] = &[
    (
        "app.css",
        "text/css; charset=utf-8",
        include_bytes!("../../../src/inari_core/web/app.css"),
    ),
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../../../src/inari_core/web/app.js"),
    ),
    (
        "fonts/ibm_plex_sans.woff2",
        "font/woff2",
        include_bytes!("../../../src/inari_core/web/fonts/ibm_plex_sans.woff2"),
    ),
    (
        "fonts/ibm_plex_mono_regular.woff2",
        "font/woff2",
        include_bytes!("../../../src/inari_core/web/fonts/ibm_plex_mono_regular.woff2"),
    ),
    (
        "fonts/ibm_plex_mono_medium.woff2",
        "font/woff2",
        include_bytes!("../../../src/inari_core/web/fonts/ibm_plex_mono_medium.woff2"),
    ),
    (
        "fonts/bricolage_grotesque.woff2",
        "font/woff2",
        include_bytes!("../../../src/inari_core/web/fonts/bricolage_grotesque.woff2"),
    ),
];

/// The page's version: a hash of what this binary serves (the page and its assets). A page
/// that sees another version in the snapshot (the core was updated) reloads itself — a
/// phone's app keeps its page open for days.
fn page_version() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let mut all = INDEX.as_bytes().to_vec();
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
    pub usage_interval: Duration,
}

fn s(p: &Value, k: &str) -> String {
    match p.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

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
}

async fn send(w: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; connect-src 'self'\r\nConnection: close\r\n\r\n",
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
            usage_interval: Duration::from_secs(60),
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
        for k in ["moves", "update_jobs"] {
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
        tokio::spawn(async move {
            // One failed accept (out of file descriptors, a reset) must not stop the page for good.
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let me = Arc::clone(&me);
                        tokio::spawn(async move { me.serve(stream).await });
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

    fn access(&self, r: &Request) -> Option<Access> {
        let pass = r
            .headers
            .get("x-inari-pass")
            .or_else(|| r.query.get("pass"))
            .map(String::as_str)
            .filter(|t| !t.is_empty());
        self.access_of(pass, pass.is_none() && self.authorized(r))
    }

    fn authorized(&self, r: &Request) -> bool {
        if self.local_page(r) {
            return true;
        }
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
        let loopback = stream.peer_addr().is_ok_and(|a| a.ip().is_loopback());
        Some(Ok(Request {
            method,
            path,
            query,
            headers,
            body,
            loopback,
        }))
    }

    async fn serve(self: Arc<Self>, mut stream: TcpStream) {
        let r = match Self::read_request(&mut stream).await {
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
        let Some(access) = self.access(&r) else {
            return send_json(w, 401, &json!({"error": "unauthorized"})).await;
        };
        let parts: Vec<&str> = r.path.split('/').collect(); // "", "api", "agent", id, what
        if get && r.path == "/events" {
            return self.events(w, access).await;
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
                    let events = self
                        .state
                        .lock()
                        .expect("not poisoned")
                        .log
                        .get(&agent)
                        .cloned()
                        .unwrap_or_default();
                    send_json(w, 200, &json!({"events": events})).await
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
        if get && r.path.starts_with("/api/logo/") {
            return self.logo(w, r.path.rsplit('/').next().unwrap_or("")).await;
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
            "/api/message" | "/api/upload" | "/api/model" | "/api/mode" | "/api/archive"
            | "/api/cancel" | "/api/rename" | "/api/pin" => {
                if self.owned(access, &s(&d, "agent")) {
                    Ok(d)
                } else {
                    Err((403, "not your thread"))
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
        w.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n").await?;
        w.write_all(format!("data: {}\n\n", self.snapshot_for(&access)).as_bytes())
            .await?;
        w.flush().await?;
        loop {
            let frame = match tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
                Ok(Ok(())) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    format!("data: {}\n\n", self.snapshot_for(&access))
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => return Ok(()),
                Err(_) => ": keep-alive\n\n".to_string(),
            };
            w.write_all(frame.as_bytes()).await?;
            w.flush().await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
