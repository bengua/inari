//! A node of the mesh, as the Python reference (`inari_core.node`): one process that
//! holds agents, serves capsules to its peers, drives its agents through their
//! harnesses, and talks to the rest over the bus. A node with no `hub` is the hub: it
//! also runs the coordinator and, when configured, the gateway. The configuration
//! file is the reference's, and so is the data directory, so a Rust node can take
//! over from a Python one in place.

use crate::agent::{summary, AcpHarness, Approver, Publisher, Runner};
use crate::approvals::NodeApprovals;
use crate::bus::{Bus, Message};
use crate::buslink::{BusClient, BusServer};
use crate::capsule::{io, EventKind};
use crate::gateway::{Gateway, GatewayOptions};
use crate::host::CapsuleHost;
use crate::mesh::{Coordinator, NodeInfo, Progress, Registry, Teleporter};
use crate::rpc::{EndpointServer, RemoteEndpoint};
use crate::teleport::{teleport_observed, CallResult, Endpoint};
use crate::updates::UpdateSpec;
use base64::Engine;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct HarnessSpec {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout: f64,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Where the harness comes from, to offer and install its updates.
    #[serde(default)]
    pub update: Option<UpdateSpec>,
}

fn default_timeout() -> f64 {
    600.0
}

/// Named harnesses in the order the file gives them: the first is the default.
#[derive(Debug, Clone, Default)]
pub struct Harnesses(pub Vec<(String, HarnessSpec)>);

impl<'de> Deserialize<'de> for Harnesses {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Ordered;
        impl<'de> Visitor<'de> for Ordered {
            type Value = Harnesses;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a map of harness name to harness")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Harnesses, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, HarnessSpec>()? {
                    out.push((k, v));
                }
                Ok(Harnesses(out))
            }
        }
        d.deserialize_map(Ordered)
    }
}

fn default_role() -> String {
    "heavy".into()
}
fn default_bind() -> String {
    "127.0.0.1".into()
}
fn default_beat() -> f64 {
    2.0
}
fn default_ttl() -> f64 {
    8.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeConfig {
    pub node_id: String,
    pub data_dir: PathBuf,
    pub key_file: PathBuf,
    #[serde(default)]
    pub device_key_file: Option<PathBuf>,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default = "default_bind")]
    pub bind_host: String,
    #[serde(default)]
    pub bus_port: u16,
    #[serde(default)]
    pub endpoint_port: u16,
    #[serde(default)]
    pub advertise_host: String,
    #[serde(default)]
    pub hub: String,
    #[serde(default)]
    pub expected_peers: Vec<String>,
    #[serde(default)]
    pub harness: Option<HarnessSpec>,
    #[serde(default)]
    pub harnesses: Harnesses,
    #[serde(default)]
    pub gateway_listen: String,
    #[serde(default)]
    pub gateway_token_file: Option<PathBuf>,
    #[serde(default)]
    pub gateway_ui: Map<String, Value>,
    /// Hub only: where new cores present their invitation (0: no invitations).
    #[serde(default)]
    pub enroll_port: u16,
    /// Pages opened on this machine (http://127.0.0.1:…) need no token.
    #[serde(default)]
    pub gateway_trust_loopback: bool,
    #[serde(default)]
    pub mascot_dir: Option<PathBuf>,
    #[serde(default)]
    pub usage: Vec<Value>,
    #[serde(default = "default_beat")]
    pub heartbeat: f64,
    #[serde(default = "default_ttl")]
    pub registry_ttl: f64,
    /// Seconds an approval waits for the phone before the default no.
    #[serde(default = "default_approval_ttl")]
    pub approval_ttl: f64,
}

fn default_approval_ttl() -> f64 {
    120.0
}

impl NodeConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Harness specs, the single `harness` (kind acp) first under the name "default".
    pub fn harness_specs(&self) -> Vec<(String, HarnessSpec)> {
        let mut out = Vec::new();
        if let Some(h) = self.harness.as_ref().filter(|h| h.kind == "acp") {
            out.push(("default".to_string(), h.clone()));
        }
        out.extend(self.harnesses.0.iter().cloned());
        out
    }
}

fn read_key(path: &Path) -> std::io::Result<Vec<u8>> {
    Ok(std::fs::read(path)?.trim_ascii().to_vec())
}

fn loadavg() -> f64 {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f64;
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|l| (l / cpus).min(1.0))
        .unwrap_or(0.0)
}

fn packets() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/net/dev") else {
        return 0;
    };
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some(f.get(2)?.parse::<u64>().ok()? + f.get(10)?.parse::<u64>().ok()?)
        })
        .sum()
}

/// This node or a peer, behind one type the teleport driver accepts.
pub enum AnyEndpoint {
    Local(Arc<CapsuleHost>),
    Remote(RemoteEndpoint),
}

macro_rules! both {
    ($self:ident, $e:ident => $call:expr) => {
        match $self {
            AnyEndpoint::Local(h) => {
                let $e = &**h;
                $call.await
            }
            AnyEndpoint::Remote(r) => {
                let $e = r;
                $call.await
            }
        }
    };
}

impl Endpoint for AnyEndpoint {
    fn node_id(&self) -> &str {
        match self {
            AnyEndpoint::Local(h) => &h.node_id,
            AnyEndpoint::Remote(r) => r.node_id(),
        }
    }
    async fn freeze(&self, agent: &str) -> CallResult<(Value, u64)> {
        both!(self, e => e.freeze(agent))
    }
    async fn read(&self, agent: &str, rel: &str) -> CallResult<Vec<u8>> {
        both!(self, e => e.read(agent, rel))
    }
    async fn unfreeze(&self, agent: &str, epoch: u64) -> CallResult<()> {
        both!(self, e => e.unfreeze(agent, epoch))
    }
    async fn tombstone(&self, agent: &str, epoch: u64) -> CallResult<()> {
        both!(self, e => e.tombstone(agent, epoch))
    }
    async fn offer(&self, sid: &str, manifest: &Value, epoch: u64) -> CallResult<Vec<String>> {
        both!(self, e => e.offer(sid, manifest, epoch))
    }
    async fn receive(&self, sid: &str, rel: &str, data: &[u8]) -> CallResult<()> {
        both!(self, e => e.receive(sid, rel, data))
    }
    async fn verify(&self, sid: &str) -> CallResult<()> {
        both!(self, e => e.verify(sid))
    }
    async fn prepare(&self, sid: &str) -> CallResult<()> {
        both!(self, e => e.prepare(sid))
    }
    async fn activate(&self, sid: &str, epoch: u64) -> CallResult<()> {
        both!(self, e => e.activate(sid, epoch))
    }
    async fn abort(&self, sid: &str) -> CallResult<()> {
        both!(self, e => e.abort(sid))
    }
    async fn holder(&self, agent: &str) -> CallResult<Option<(String, u64)>> {
        both!(self, e => e.holder(agent))
    }
}

fn s(p: &Value, k: &str) -> String {
    match p.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub struct Node {
    pub cfg: NodeConfig,
    pub is_hub: bool,
    key: Vec<u8>,
    pub bus: Arc<Bus>,
    pub host: Arc<CapsuleHost>,
    pub registry: Arc<Registry>,
    pub runner: Option<Arc<Runner>>,
    pub endpoint_port: u16,
    pub bus_port: u16,
    pub gateway: Option<Arc<Gateway>>,
    pub gateway_port: u16,
    /// Where new cores present their invitation (hub only, 0 when off).
    pub enroll_port: u16,
    _client: Mutex<Option<BusClient>>,
    /// Per harness: installed and latest version, and the state of an update in progress.
    updates: Mutex<Map<String, Value>>,
    /// Set after an update: the next heartbeat re-reads the model catalogue at once.
    refresh: std::sync::atomic::AtomicBool,
}

impl Node {
    /// Build and start everything the configuration asks for.
    pub async fn start(cfg: NodeConfig) -> Result<Arc<Self>, String> {
        let is_hub = cfg.hub.is_empty();
        let key = read_key(&cfg.key_file).map_err(|e| format!("mesh key: {e}"))?;
        let device_key = match &cfg.device_key_file {
            Some(p) => read_key(p).map_err(|e| format!("device key: {e}"))?,
            None => vec![],
        };
        let bus = Bus::new(&cfg.node_id);
        let mut host = CapsuleHost::new(&cfg.node_id, &cfg.data_dir.join("host"))
            .map_err(|e| e.to_string())?;
        let specs = cfg.harness_specs();
        let names: Vec<String> = specs.iter().map(|(n, _)| n.clone()).collect();
        if !names.is_empty() {
            // refuse an agent whose harness this core lacks, before the point of no return
            let known = names.clone();
            host.on_prepare = Some(Box::new(move |c| {
                match c.manifest.labels().get("harness").and_then(Value::as_str) {
                    Some(h) if !h.is_empty() && !known.iter().any(|k| k == h) => {
                        Err(format!("this core has no {h:?} harness"))
                    }
                    _ => Ok(()),
                }
            }));
        }
        let host = Arc::new(host);
        host.recover();
        let endpoint = EndpointServer::new(Arc::clone(&host), &key);
        let endpoint_port = endpoint
            .start(&cfg.bind_host, cfg.endpoint_port)
            .await
            .map_err(|e| format!("endpoint: {e}"))?;
        let registry = Arc::new(Registry::new(cfg.registry_ttl));
        let mut bus_port = 0;
        let mut client = None;
        if is_hub {
            let expected: Vec<&str> = cfg.expected_peers.iter().map(String::as_str).collect();
            let server = BusServer::new(Arc::clone(&bus), &key, &expected);
            bus_port = server
                .start(&cfg.bind_host, cfg.bus_port)
                .await
                .map_err(|e| format!("bus: {e}"))?;
            let (h, r, k, me) = (
                Arc::clone(&host),
                Arc::clone(&registry),
                key.clone(),
                cfg.node_id.clone(),
            );
            let teleporter: Teleporter =
                Arc::new(move |agent, source, target, progress: Progress| {
                    let ends = |id: &str| -> AnyEndpoint {
                        if id == me {
                            return AnyEndpoint::Local(Arc::clone(&h));
                        }
                        let addr = r
                            .get(id)
                            .and_then(|n| {
                                n.capabilities
                                    .get("endpoint")
                                    .and_then(Value::as_str)
                                    .map(String::from)
                            })
                            .unwrap_or_default();
                        AnyEndpoint::Remote(RemoteEndpoint::new(
                            id,
                            &addr,
                            &k,
                            Duration::from_secs(30),
                        ))
                    };
                    let (src, dst) = (ends(&source), ends(&target));
                    Box::pin(async move {
                        teleport_observed(&src, &dst, &agent, None, Some(progress)).await
                    })
                });
            Coordinator::start(Arc::clone(&bus), Arc::clone(&registry), teleporter);
        } else {
            client = Some(BusClient::start(Arc::clone(&bus), &key, &cfg.hub));
        }
        if is_hub {
            crate::passes::PassBook::start(&bus, &cfg.data_dir);
        }
        let mut enroll_port = 0;
        if is_hub && cfg.enroll_port > 0 {
            let counter = crate::enroll::Enroll::start(&bus, &key, &device_key, bus_port);
            enroll_port = counter
                .listen(&cfg.bind_host, cfg.enroll_port)
                .await
                .map_err(|e| format!("enroll: {e}"))?;
        }
        let approver: Option<Approver> = if device_key.is_empty() {
            None
        } else {
            let gate = NodeApprovals::start(Arc::clone(&bus), &device_key);
            let (node, ttl) = (cfg.node_id.clone(), cfg.approval_ttl);
            Some(Arc::new(move |agent: String, command: String| {
                let (gate, node) = (Arc::clone(&gate), node.clone());
                Box::pin(async move { gate.ask(&agent, &node, &command, ttl).await })
            }))
        };
        let runner = if specs.is_empty() {
            None
        } else {
            let publish_bus = Arc::clone(&bus);
            let publish: Publisher = Arc::new(move |subject: &str, payload: Value| {
                publish_bus.publish(subject, payload);
            });
            let built = specs
                .iter()
                .map(|(name, spec)| {
                    let file = cfg
                        .data_dir
                        .join("host")
                        .join(format!("acp_sessions.{name}.json"));
                    let mut h = AcpHarness::new(spec.command.clone(), file);
                    h.timeout = Duration::from_secs_f64(spec.timeout.max(1.0));
                    h.extra_env = spec.env.clone();
                    (name.clone(), h)
                })
                .collect();
            Some(Runner::new(
                Arc::clone(&host),
                built,
                Some(publish),
                approver,
            ))
        };
        let mut gateway_port = 0;
        // any node may serve the interface: its bus sees the whole mesh through the hub
        let gateway = if !cfg.gateway_listen.is_empty() {
            let token = match &cfg.gateway_token_file {
                Some(p) => std::fs::read_to_string(p)
                    .map_err(|e| format!("gateway token: {e}"))?
                    .trim()
                    .to_string(),
                None => String::new(),
            };
            let g = Gateway::new(
                Arc::clone(&bus),
                GatewayOptions {
                    token,
                    key: device_key.clone(),
                    ui: cfg.gateway_ui.clone(),
                    mascot_dir: cfg.mascot_dir.clone(),
                    usage: cfg.usage.clone(),
                    data_dir: cfg.data_dir.clone(),
                    trust_loopback: cfg.gateway_trust_loopback,
                },
            );
            let (lhost, lport) = cfg
                .gateway_listen
                .rsplit_once(':')
                .ok_or("gateway_listen is HOST:PORT")?;
            gateway_port = g
                .start(
                    lhost,
                    lport.parse().map_err(|_| "gateway_listen is HOST:PORT")?,
                )
                .await
                .map_err(|e| format!("gateway: {e}"))?;
            Some(g)
        } else {
            None
        };
        let node = Arc::new(Self {
            cfg,
            is_hub,
            key,
            bus,
            host,
            registry,
            runner,
            endpoint_port,
            bus_port,
            gateway,
            gateway_port,
            enroll_port,
            _client: Mutex::new(client),
            updates: Mutex::default(),
            refresh: std::sync::atomic::AtomicBool::new(false),
        });
        node.subscribe();
        let beat = Arc::clone(&node);
        tokio::spawn(async move { beat.beat().await });
        let watch = Arc::clone(&node);
        tokio::spawn(async move {
            loop {
                watch.check_updates().await;
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            }
        });
        Ok(node)
    }

    /// The key the mesh shares (for tools that talk to this node's endpoint).
    pub fn mesh_key(&self) -> &[u8] {
        &self.key
    }

    fn held(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.host.agents)
            .map(|d| {
                d.filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .filter(|a| self.host.status(a) == "running")
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    /// What a card shows about an agent: cheap facts from its capsule.
    fn meta(&self, agent: &str) -> Map<String, Value> {
        let Ok(c) = io::load(&self.host.agents.join(agent)) else {
            return Map::new();
        };
        let events = c.timeline.ordered();
        let last = events
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::AssistantMessage);
        let labels = c.manifest.labels();
        let label = |k: &str| {
            labels
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let title = c
            .manifest
            .extra
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .unwrap_or(agent);
        let harness = Some(label("harness"))
            .filter(|h| !h.is_empty())
            .or_else(|| self.runner.as_ref().map(|r| r.default_harness.clone()))
            .unwrap_or_default();
        let mut m = Map::new();
        m.insert("title".into(), json!(title));
        m.insert("events".into(), json!(events.len()));
        m.insert("files".into(), json!(c.snapshot.tree.len()));
        m.insert(
            "last".into(),
            json!(last
                .map(|e| summary(e).chars().take(160).collect::<String>())
                .unwrap_or_default()),
        );
        m.insert("model".into(), json!(label("model")));
        m.insert("harness".into(), json!(harness));
        m.insert("pass".into(), json!(label("pass")));
        m.insert("archived".into(), json!(label("archived") == "1"));
        m
    }

    async fn harness_models(&self) -> Map<String, Value> {
        let scratch = self.cfg.data_dir.join("scratch");
        let _ = std::fs::create_dir_all(&scratch);
        let mut out = Map::new();
        if let Some(r) = &self.runner {
            for (name, h) in &r.harnesses {
                out.insert(name.clone(), json!(h.list_models(&scratch).await));
            }
        }
        out
    }

    async fn beat(self: Arc<Self>) {
        let cfg = &self.cfg;
        let advertise = format!(
            "{}:{}",
            if cfg.advertise_host.is_empty() {
                &cfg.bind_host
            } else {
                &cfg.advertise_host
            },
            self.endpoint_port
        );
        let incarnation = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1);
        let mut info = NodeInfo {
            node_id: cfg.node_id.clone(),
            role: cfg.role.clone(),
            incarnation,
            seen_at: 0.0,
            capabilities: Map::new(),
        };
        info.capabilities
            .insert("endpoint".into(), json!(advertise));
        if self.enroll_port > 0 {
            let host = advertise
                .rsplit_once(':')
                .map_or(advertise.as_str(), |(h, _)| h);
            info.capabilities.insert(
                "enroll".into(),
                json!(format!("{host}:{}", self.enroll_port)),
            );
        }
        let mut models_at: Option<std::time::Instant> = None;
        let mut last_packets = packets();
        let mut first = true;
        let period = Duration::from_secs_f64(cfg.heartbeat.max(0.05));
        loop {
            let forced = self
                .refresh
                .swap(false, std::sync::atomic::Ordering::Relaxed);
            if forced || models_at.is_none_or(|t| t.elapsed() > Duration::from_secs(6 * 3600)) {
                // the catalogue changes rarely: look at start, after an update, and every six hours
                let harnesses = self.harness_models().await;
                let default = self
                    .runner
                    .as_ref()
                    .map(|r| r.default_harness.clone())
                    .unwrap_or_default();
                info.capabilities.insert(
                    "models".into(),
                    harnesses.get(&default).cloned().unwrap_or(json!([])),
                );
                info.capabilities
                    .insert("harnesses".into(), Value::Object(harnesses));
                models_at = Some(std::time::Instant::now());
            }
            let updates = self.updates.lock().expect("not poisoned").clone();
            if !updates.is_empty() {
                info.capabilities
                    .insert("updates".into(), Value::Object(updates));
            }
            self.bus.publish(
                if first {
                    "node.announce"
                } else {
                    "node.heartbeat"
                },
                info.to_json(),
            );
            first = false;
            let now = packets();
            let rate = now.saturating_sub(last_packets) as f64 / period.as_secs_f64();
            last_packets = now;
            self.bus.publish(
                &format!("node.{}.load", cfg.node_id),
                json!({"node": cfg.node_id, "gpu": loadavg(), "packets": rate}),
            );
            for agent in self.held() {
                let phase = self
                    .runner
                    .as_ref()
                    .and_then(|r| r.phases.lock().expect("not poisoned").get(&agent).cloned())
                    .unwrap_or_else(|| "idle".into());
                let mut p = self.meta(&agent);
                p.insert("agent".into(), json!(agent));
                p.insert("node".into(), json!(cfg.node_id));
                p.insert("phase".into(), json!(phase));
                self.bus
                    .publish(&format!("agent.{agent}.state"), Value::Object(p));
            }
            if self.is_hub {
                for lost in self.registry.expired() {
                    self.bus.publish("node.lost", json!({"node": lost}));
                }
            }
            tokio::time::sleep(period).await;
        }
    }

    fn update_spec(&self, harness: &str) -> Option<UpdateSpec> {
        self.cfg
            .harness_specs()
            .into_iter()
            .find(|(n, _)| n == harness)
            .and_then(|(_, s)| s.update)
    }

    fn update_running(&self, harness: &str) -> bool {
        self.updates
            .lock()
            .expect("not poisoned")
            .get(harness)
            .is_some_and(|u| u["state"] == "running")
    }

    /// Ask the registry what is new for every harness that names its package.
    pub async fn check_updates(&self) {
        for (name, spec) in self.cfg.harness_specs() {
            let Some(update) = spec.update else { continue };
            if self.update_running(&name) {
                continue;
            }
            let status = update.status().await;
            self.updates
                .lock()
                .expect("not poisoned")
                .insert(name, status);
        }
    }

    fn update_state(&self, harness: &str, fields: Value) {
        let state = {
            let mut all = self.updates.lock().expect("not poisoned");
            let entry = all.entry(harness.to_string()).or_insert_with(|| json!({}));
            if let (Some(e), Some(f)) = (entry.as_object_mut(), fields.as_object()) {
                e.extend(f.clone());
            }
            entry.clone()
        };
        let mut state = state;
        state["node"] = json!(self.cfg.node_id);
        state["harness"] = json!(harness);
        self.bus
            .publish(&format!("node.{}.update.state", self.cfg.node_id), state);
    }

    /// Install the latest version of a harness, restart what needs it, announce the result.
    pub async fn update_harness(&self, harness: &str) {
        let Some(spec) = self.update_spec(harness) else {
            self.update_state(
                harness,
                json!({"state": "failed", "error": "this harness names no package to update"}),
            );
            return;
        };
        if self.update_running(harness) {
            return;
        }
        self.update_state(harness, json!({"state": "running", "error": ""}));
        let result = match spec.latest().await {
            Ok(latest) => spec.install(&latest).await,
            Err(e) => Err(e),
        };
        let mut status = spec.status().await;
        match result {
            Ok(()) => status["state"] = json!("done"),
            Err(e) => {
                status["state"] = json!("failed");
                status["error"] = json!(e);
            }
        }
        self.update_state(harness, status);
        self.refresh
            .store(true, std::sync::atomic::Ordering::Relaxed); // new version, maybe new models
    }

    fn on<F, Fut>(self: &Arc<Self>, pattern: &str, f: F)
    where
        F: Fn(Arc<Self>, Message) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut rx = self.bus.subscribe(pattern);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                f(Arc::clone(&me), m).await;
            }
        });
    }

    fn running(&self, agent: &str) -> bool {
        self.host.status(agent) == "running"
    }

    fn created(&self, agent: &str, ok: bool, error: &str) {
        self.bus.publish(
            "agent.created",
            json!({"agent": agent, "node": self.cfg.node_id, "ok": ok, "error": error}),
        );
    }

    fn subscribe(self: &Arc<Self>) {
        self.on("agent.create", |n, m| async move {
            let p = &m.payload;
            let Some(r) = n.runner.as_ref().filter(|_| s(p, "node") == n.cfg.node_id) else {
                return;
            };
            match r.create(&s(p, "thread_id"), &s(p, "harness"), &s(p, "title")) {
                Ok(agent) => {
                    let pass = s(p, "pass");
                    if !pass.is_empty() {
                        let _ = r.label(&agent, "pass", &pass);
                    }
                    n.created(&agent, true, "")
                }
                Err(e) => n.created(&s(p, "thread_id"), false, &e),
            }
        });
        self.on("agent.*.run", |n, m| async move {
            let agent = m.subject.split('.').nth(1).unwrap_or("").to_string();
            let Some(r) = n.runner.clone().filter(|_| n.running(&agent)) else { return };
            let text = s(&m.payload, "text");
            let paths: Vec<String> = m.payload.get("attachments").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let model = s(&m.payload, "model");
            // detached: a turn may wait for an approval that arrives on this very bus
            tokio::spawn(async move {
                if !model.is_empty() {
                    let _ = r.set_model(&agent, &model);
                }
                let attached = r.attached(&agent, &paths);
                let out = r.run(&agent, &text, attached).await;
                let text: String = out.final_text.chars().take(400).collect();
                n.bus.publish(&format!("agent.{agent}.done"), json!({"agent": agent, "stopped": out.stopped, "error": out.error, "text": text}));
            });
        });
        self.on("agent.*.fork", |n, m| async move {
            let agent = m.subject.split('.').nth(1).unwrap_or("").to_string();
            if !n.running(&agent) {
                return;
            }
            let new = s(&m.payload, "new_thread");
            let upto = m
                .payload
                .get("upto")
                .and_then(Value::as_str)
                .map(String::from);
            let staging = n.host.root.join("new").join(&new);
            let _ = std::fs::remove_dir_all(&staging);
            let made = crate::capsule::fork::fork(
                &n.host.agents.join(&agent),
                &staging,
                &new,
                upto.as_deref(),
            )
            .map_err(|e| e.to_string())
            .and_then(|_| n.host.adopt(&staging).map_err(|e| e.to_string()));
            let _ = std::fs::remove_dir_all(&staging);
            // a clone: the same journal and files, thought on by another harness
            let harness = s(&m.payload, "harness");
            let made = match (made, n.runner.as_ref()) {
                (Ok(id), Some(r)) if !harness.is_empty() => {
                    let title = s(&m.payload, "title");
                    r.set_harness(&id, &harness)
                        .and_then(|_| {
                            if title.is_empty() {
                                Ok(())
                            } else {
                                r.set_title(&id, &title)
                            }
                        })
                        .map(|_| id)
                }
                (made, _) => made,
            };
            match made {
                Ok(id) => n.created(&id, true, ""),
                Err(e) => n.created(&new, false, &e),
            }
        });
        self.on("agent.*.label", |n, m| async move {
            // the labels a person may set from the interface: who it is shared with, asleep or not
            let agent = m.subject.split('.').nth(1).unwrap_or("").to_string();
            let key = s(&m.payload, "key");
            if let Some(r) = n
                .runner
                .as_ref()
                .filter(|_| n.running(&agent) && ["pass", "archived"].contains(&key.as_str()))
            {
                let _ = r.label(&agent, &key, &s(&m.payload, "value"));
            }
        });
        self.on("agent.merge", |n, m| async move {
            let (target, source) = (s(&m.payload, "target"), s(&m.payload, "source"));
            if !n.running(&target) || !n.running(&source) {
                return;
            }
            let reply = match crate::capsule::fork::merge(
                &n.host.agents.join(&target),
                &n.host.agents.join(&source),
            ) {
                Ok(added) => {
                    json!({"target": target, "source": source, "added": added, "ok": true})
                }
                Err(e) => {
                    json!({"target": target, "source": source, "ok": false, "error": e.to_string()})
                }
            };
            n.bus.publish("agent.merged", reply);
        });
        self.on("agent.history.request", |n, m| async move {
            let agent = s(&m.payload, "agent");
            if !n.running(&agent) {
                return;
            }
            let Ok(c) = io::load(&n.host.agents.join(&agent)) else { return };
            let events: Vec<Value> = c
                .timeline
                .ordered()
                .iter()
                .map(|e| json!({"id": e.id, "kind": e.kind.as_str(), "text": summary(e).chars().take(400).collect::<String>()}))
                .collect();
            n.bus.publish(&format!("agent.{agent}.history"), json!({"agent": agent, "events": events}));
        });
        self.on("agent.files.request", |n, m| async move {
            let agent = s(&m.payload, "agent");
            if !n.running(&agent) {
                return;
            }
            let Ok(c) = io::load(&n.host.agents.join(&agent)) else { return };
            let blobs = n.host.agents.join(&agent).join(io::BLOBS);
            let files: Vec<Value> = c
                .snapshot
                .tree
                .iter()
                .filter_map(|(path, sha)| Some(json!({"path": path, "size": std::fs::metadata(blobs.join(sha)).ok()?.len()})))
                .collect();
            n.bus.publish(&s(&m.payload, "reply"), json!({"agent": agent, "files": files}));
        });
        self.on("agent.file.request", |n, m| async move {
            let (agent, rel) = (s(&m.payload, "agent"), s(&m.payload, "path"));
            if !n.running(&agent) {
                return;
            }
            let mut reply = json!({"agent": agent, "path": rel, "size": 0, "b64": "", "text": "", "binary": false});
            if let Ok(c) = io::load(&n.host.agents.join(&agent)) {
                if let Some(raw) = c.snapshot.tree.get(&rel).and_then(|sha| std::fs::read(n.host.agents.join(&agent).join(io::BLOBS).join(sha)).ok()) {
                    reply["size"] = json!(raw.len());
                    match std::str::from_utf8(&raw[..raw.len().min(200_000)]) {
                        Ok(t) => reply["text"] = json!(t),
                        Err(e) if e.error_len().is_none() && raw.len() > 200_000 => {
                            reply["text"] = json!(String::from_utf8_lossy(&raw[..e.valid_up_to()])) // cut inside a character
                        }
                        Err(_) => reply["binary"] = json!(true),
                    }
                    if reply["binary"] == true && raw.len() <= 3_000_000 {
                        reply["b64"] = json!(base64::engine::general_purpose::STANDARD.encode(&raw)); // images and the like: whole
                    }
                }
            }
            n.bus.publish(&s(&m.payload, "reply"), reply);
        });
        self.on("agent.exec.request", |n, m| async move {
            let agent = s(&m.payload, "agent");
            let Some(r) = n.runner.clone().filter(|_| n.running(&agent)) else {
                return;
            };
            let (cmd, reply) = (s(&m.payload, "cmd"), s(&m.payload, "reply"));
            tokio::spawn(async move {
                let mut out = r.exec_user(&agent, &cmd, Duration::from_secs(60)).await;
                out["agent"] = json!(agent);
                n.bus.publish(&reply, out);
            });
        });
        self.on("agent.upload.request", |n, m| async move {
            let agent = s(&m.payload, "agent");
            let Some(r) = n.runner.clone().filter(|_| n.running(&agent)) else { return };
            let reply = s(&m.payload, "reply");
            let data = base64::engine::general_purpose::STANDARD.decode(s(&m.payload, "b64"));
            let out = match data {
                Ok(bytes) => match r.attach(&agent, &s(&m.payload, "path"), &bytes).await {
                    Ok(a) => json!({"agent": agent, "ok": true, "path": a.path, "size": a.size, "mime": a.mime}),
                    Err(e) => json!({"agent": agent, "ok": false, "error": e}),
                },
                Err(e) => json!({"agent": agent, "ok": false, "error": format!("bad base64: {e}")}),
            };
            n.bus.publish(&reply, out);
        });
        self.on("agent.*.model", |n, m| async move {
            let agent = m.subject.split('.').nth(1).unwrap_or("").to_string();
            if let Some(r) = n.runner.as_ref().filter(|_| n.running(&agent)) {
                let _ = r.set_model(&agent, &s(&m.payload, "model"));
            }
        });
        let me = self.cfg.node_id.clone();
        self.on(&format!("node.{me}.update"), |n, m| async move {
            let harness = s(&m.payload, "harness");
            tokio::spawn(async move { n.update_harness(&harness).await }); // npm can take minutes
        });
        for subject in [
            format!("node.{me}.update.check"),
            "node.all.update.check".to_string(),
        ] {
            self.on(&subject, |n, _m| async move {
                tokio::spawn(async move { n.check_updates().await });
            });
        }
        self.on("agent.*.harness", |n, m| async move {
            let agent = m.subject.split('.').nth(1).unwrap_or("").to_string();
            if let Some(r) = n.runner.as_ref().filter(|_| n.running(&agent)) {
                let _ = r.set_harness(&agent, &s(&m.payload, "harness"));
            }
        });
    }
}
