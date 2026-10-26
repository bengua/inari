//! Discovery, liveness and telemetry, as the Python reference (`inari_core.mesh`):
//! nodes announce and heartbeat, the registry keeps the live ones, the coordinator
//! answers teleport requests, and the mascot is a pure function of the telemetry.

use crate::bus::Bus;
use crate::teleport::Outcome;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub struct NodeInfo {
    pub node_id: String,
    pub role: String, // heavy | control | mobile
    pub incarnation: u64,
    pub seen_at: f64,
    pub capabilities: Map<String, Value>,
}

impl NodeInfo {
    pub fn to_json(&self) -> Value {
        json!({"node_id": self.node_id, "role": self.role, "incarnation": self.incarnation,
               "seen_at": self.seen_at, "capabilities": self.capabilities})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            node_id: v.get("node_id")?.as_str()?.into(),
            role: v.get("role")?.as_str()?.into(),
            incarnation: v.get("incarnation").and_then(Value::as_u64).unwrap_or(1),
            seen_at: v.get("seen_at").and_then(Value::as_f64).unwrap_or(0.0),
            capabilities: v
                .get("capabilities")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        })
    }
}

/// Live nodes, by sightings; a node that stops heartbeating expires after `ttl` seconds.
pub struct Registry {
    pub ttl: f64,
    start: Instant,
    nodes: Mutex<BTreeMap<String, NodeInfo>>,
}

impl Registry {
    pub fn new(ttl: f64) -> Self {
        Self {
            ttl,
            start: Instant::now(),
            nodes: Mutex::default(),
        }
    }

    fn clock(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Record a sighting; true when it changed what we know.
    pub fn observe(&self, mut info: NodeInfo) -> bool {
        info.seen_at = self.clock();
        let mut nodes = self.nodes.lock().expect("not poisoned");
        let known = nodes.get(&info.node_id);
        if known.is_some_and(|k| k.incarnation > info.incarnation) {
            return false; // a stale announcement from before a restart
        }
        let changed = known.is_none_or(|k| {
            k.incarnation != info.incarnation || k.capabilities != info.capabilities
        });
        nodes.insert(info.node_id.clone(), info);
        changed
    }

    pub fn alive(&self) -> Vec<NodeInfo> {
        let now = self.clock();
        self.nodes
            .lock()
            .expect("not poisoned")
            .values()
            .filter(|n| now - n.seen_at <= self.ttl)
            .cloned()
            .collect()
    }

    pub fn get(&self, node_id: &str) -> Option<NodeInfo> {
        self.alive().into_iter().find(|n| n.node_id == node_id)
    }

    pub fn expired(&self) -> Vec<String> {
        let now = self.clock();
        self.nodes
            .lock()
            .expect("not poisoned")
            .values()
            .filter(|n| now - n.seen_at > self.ttl)
            .map(|n| n.node_id.clone())
            .collect()
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.alive().iter().map(NodeInfo::to_json).collect()
    }
}

pub const PHASES: [&str; 5] = ["idle", "listening", "inference", "success", "error"];
const PRIORITY: [&str; 5] = ["error", "inference", "listening", "success", "idle"];

fn colour(phase: &str) -> &'static str {
    match phase {
        "listening" => "#e879f9",
        "inference" => "#fbbf24",
        "success" => "#34d399",
        "error" => "#f87171",
        _ => "#22d3ee",
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[derive(Default)]
pub struct Telemetry {
    pub agents: BTreeMap<String, Map<String, Value>>,
    pub nodes: BTreeMap<String, (f64, f64)>, // gpu, packets
}

impl Telemetry {
    pub fn update_agent(
        &mut self,
        agent: &str,
        node: &str,
        phase: &str,
        extra: &Map<String, Value>,
    ) {
        let entry = self.agents.entry(agent.into()).or_default();
        entry.insert("node".into(), json!(node));
        let phase = if PHASES.contains(&phase) {
            phase
        } else {
            "idle"
        };
        entry.insert("phase".into(), json!(phase));
        for (k, v) in extra {
            entry.insert(k.clone(), v.clone());
        }
    }

    pub fn update_node(&mut self, node: &str, gpu: f64, packets: f64) {
        self.nodes.insert(node.into(), (gpu, packets));
    }

    pub fn drop_node(&mut self, node: &str) {
        self.nodes.remove(node);
    }

    /// The creature drawn from the numbers: phase by priority, pulse from the load,
    /// turbulence from the traffic, glow from the agents at work.
    pub fn mascot(&self) -> Value {
        let phases: HashSet<&str> = self
            .agents
            .values()
            .filter_map(|a| a.get("phase").and_then(Value::as_str))
            .collect();
        let phase = PRIORITY
            .iter()
            .find(|p| phases.contains(*p))
            .copied()
            .unwrap_or("idle");
        let gpu = self.nodes.values().map(|n| n.0).fold(0.0, f64::max);
        let packets: f64 = self.nodes.values().map(|n| n.1).sum();
        let busy = self
            .agents
            .values()
            .filter(|a| a.get("phase").and_then(Value::as_str) != Some("idle"))
            .count() as f64;
        json!({"phase": phase, "colour": colour(phase), "pulse_hz": round3(0.25 + 1.75 * gpu.min(1.0)),
               "turbulence": round3((packets / 2000.0).min(1.0)), "glow": round3((0.2 + 0.2 * busy).min(1.0))})
    }

    pub fn load(&self) -> Value {
        Value::Object(
            self.nodes
                .iter()
                .map(|(k, (g, p))| (k.clone(), json!({"gpu": g, "packets": p})))
                .collect(),
        )
    }
}

pub type Progress = Arc<dyn Fn(&str) + Send + Sync>;
pub type Teleporter = Arc<
    dyn Fn(String, String, String, Progress) -> Pin<Box<dyn Future<Output = Outcome> + Send>>
        + Send
        + Sync,
>;

/// The hub's coordinator: keeps the registry from announcements, answers teleport
/// requests. Crash-only: the next round of heartbeats rebuilds everything.
pub struct Coordinator {
    pub bus: Arc<Bus>,
    pub registry: Arc<Registry>,
    teleporter: Teleporter,
    busy: Mutex<HashSet<String>>,
}

impl Coordinator {
    pub fn start(bus: Arc<Bus>, registry: Arc<Registry>, teleporter: Teleporter) -> Arc<Self> {
        let me = Arc::new(Self {
            bus: Arc::clone(&bus),
            registry,
            teleporter,
            busy: Mutex::default(),
        });
        let mut nodes = bus.subscribe("node.announce");
        let mut beats = bus.subscribe("node.heartbeat");
        let c = Arc::clone(&me);
        tokio::spawn(async move {
            loop {
                let m = tokio::select! {
                    Some(m) = nodes.recv() => m,
                    Some(m) = beats.recv() => m,
                    else => break,
                };
                if let Some(info) = NodeInfo::from_json(&m.payload) {
                    if c.registry.observe(info) {
                        c.bus
                            .publish("mesh.nodes", json!({"nodes": c.registry.snapshot()}));
                    }
                }
            }
        });
        let mut requests = bus.subscribe("teleport.request");
        let c = Arc::clone(&me);
        tokio::spawn(async move {
            while let Some(m) = requests.recv().await {
                let c = Arc::clone(&c);
                tokio::spawn(async move { c.handle(m.payload).await }); // a move must not stall the link
            }
        });
        me
    }

    fn refusal(&self, p: &Value) -> String {
        let s = |k| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (source, target) = (
            self.registry.get(&s("source")),
            self.registry.get(&s("target")),
        );
        match (source, target) {
            (Some(a), Some(b)) if a.node_id == b.node_id => {
                "source and target are the same node".into()
            }
            (Some(_), Some(b)) if b.role != "heavy" => "only a heavy node can host an agent".into(),
            (Some(_), Some(_))
                if self
                    .busy
                    .lock()
                    .expect("not poisoned")
                    .contains(&s("agent")) =>
            {
                "this agent is already moving".into()
            }
            (Some(_), Some(_)) => String::new(),
            _ => "source or target is not alive".into(),
        }
    }

    async fn handle(self: Arc<Self>, p: Value) {
        let request = p.get("request").cloned().unwrap_or(Value::Null);
        let refusal = self.refusal(&p);
        if !refusal.is_empty() {
            self.bus.publish(
                "teleport.result",
                json!({"request": request, "ok": false, "error": refusal}),
            );
            return;
        }
        let s = |k| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (agent, source, target) = (s("agent"), s("source"), s("target"));
        self.busy
            .lock()
            .expect("not poisoned")
            .insert(agent.clone());
        let bus = Arc::clone(&self.bus);
        let (a, so, ta, rq) = (
            agent.clone(),
            source.clone(),
            target.clone(),
            request.clone(),
        );
        let progress: Progress = Arc::new(move |state: &str| {
            bus.publish(
                "teleport.state",
                json!({"request": rq, "agent": a, "source": so, "target": ta, "state": state}),
            );
        });
        let outcome = (self.teleporter)(agent.clone(), source, target, progress).await;
        self.busy.lock().expect("not poisoned").remove(&agent);
        self.bus.publish("teleport.result", json!({"request": request, "ok": outcome.state == "committed",
                                                     "state": outcome.state, "error": outcome.error}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_incarnation_is_ignored_and_a_change_is_reported() {
        let r = Registry::new(10.0);
        let mut n = NodeInfo {
            node_id: "a".into(),
            role: "heavy".into(),
            incarnation: 5,
            seen_at: 0.0,
            capabilities: Map::new(),
        };
        assert!(r.observe(n.clone()));
        assert!(!r.observe(n.clone()));
        n.incarnation = 4;
        assert!(!r.observe(n.clone()));
        n.incarnation = 6;
        assert!(r.observe(n));
        assert_eq!(r.alive().len(), 1);
    }

    #[test]
    fn the_mascot_follows_the_numbers() {
        let mut t = Telemetry::default();
        assert_eq!(t.mascot()["phase"], "idle");
        t.update_agent("x", "n", "inference", &Map::new());
        t.update_agent("y", "n", "error", &Map::new());
        t.update_node("n", 0.5, 1000.0);
        let m = t.mascot();
        assert_eq!(
            (m["phase"].as_str(), m["colour"].as_str()),
            (Some("error"), Some("#f87171"))
        );
        assert_eq!(
            (
                m["pulse_hz"].as_f64(),
                m["turbulence"].as_f64(),
                m["glow"].as_f64()
            ),
            (Some(1.125), Some(0.5), Some(0.6))
        );
    }
}
