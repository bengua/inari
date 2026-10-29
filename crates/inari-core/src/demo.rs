//! A whole mesh in one process, for trying the interface: three real nodes talking over
//! loopback TCP, each running the demo agent (`inari-core-rs demo-agent`), live turns and an
//! approval question now and then. Every drag in the browser performs a genuine teleportation.

use crate::gateway::Gateway;
use crate::node::{Node, NodeConfig};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub const NAMES: [&str; 3] = ["atlas", "borealis", "cirrus"];
/// The threads the demo starts with, and the core each starts on.
pub const START: [(&str, &str); 3] = [
    ("scribe", "atlas"),
    ("builder", "atlas"),
    ("reviewer", "borealis"),
];

pub struct DemoOptions {
    pub host: String,
    pub port: u16,
    pub token: String,
    /// Where the nodes keep their capsules (a fresh temporary directory when absent).
    pub workdir: Option<PathBuf>,
    /// FoxBot's folder, served as the mascot (the orb otherwise).
    pub mascot_dir: Option<PathBuf>,
    /// The command that runs the demo agent; this binary's `demo-agent` by default.
    pub agent: Vec<String>,
}

/// The running demo: its nodes (the hub first) and the hub's gateway.
pub struct Demo {
    pub nodes: Vec<Arc<Node>>,
    pub gateway: Arc<Gateway>,
    pub workdir: PathBuf,
}

/// Start the three nodes and the first turns; the background turns run until the process ends.
pub async fn start(o: DemoOptions) -> Result<Demo, String> {
    let base = match o.workdir {
        Some(d) => d,
        None => std::env::temp_dir().join(format!("inari-demo-{}", crate::wire::random_hex(4))),
    };
    std::fs::create_dir_all(&base).map_err(|e| format!("{}: {e}", base.display()))?;
    for (name, text) in [
        ("mesh.key", "demo-mesh-key"),
        ("device.key", "demo-device-key"),
        ("token", o.token.as_str()),
    ] {
        std::fs::write(base.join(name), text).map_err(|e| format!("{name}: {e}"))?;
    }
    let config = |name: &str, hub: &str, gateway: bool| -> Result<NodeConfig, String> {
        let mut cfg = json!({
            "node_id": name, "data_dir": base.join(name), "key_file": base.join("mesh.key"),
            "device_key_file": base.join("device.key"), "heartbeat": 1.0, "hub": hub,
            "bus_port": 0, "endpoint_port": 0,
            "harnesses": {"demo": {"command": o.agent}},
        });
        if gateway {
            cfg["gateway_listen"] = json!(format!("{}:{}", o.host, o.port));
            cfg["gateway_token_file"] = json!(base.join("token"));
            cfg["expected_peers"] = json!(NAMES[1..]);
            if let Some(m) = &o.mascot_dir {
                cfg["mascot_dir"] = json!(m);
            }
        }
        serde_json::from_value(cfg).map_err(|e| format!("demo configuration: {e}"))
    };
    let hub = Node::start(config(NAMES[0], "", true)?)
        .await
        .map_err(|e| e.to_string())?;
    let gateway = hub.gateway.clone().ok_or("the hub has no gateway")?;
    let hub_bus = format!("127.0.0.1:{}", hub.bus_port);
    let mut nodes = vec![hub];
    for name in &NAMES[1..] {
        nodes.push(
            Node::start(config(name, &hub_bus, false)?)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    // every core has announced itself before the threads are made
    for _ in 0..200 {
        if gateway.snapshot()["nodes"].as_array().map_or(0, Vec::len) >= NAMES.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let bus = Arc::clone(&nodes[0].bus);
    for (agent, node) in START {
        bus.publish(
            "agent.create",
            json!({"node": node, "thread_id": agent, "harness": "demo", "title": agent}),
        );
    }
    for _ in 0..100 {
        if START
            .iter()
            .all(|(a, _)| gateway.snapshot()["agents"].get(a).is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for (agent, _) in START {
        bus.publish(
            &format!("agent.{agent}.run"),
            json!({"text": format!("write notes.txt: hello from {agent}")}),
        );
    }
    // now and then a thread runs a command, and the builder asks to do something only a person allows
    let chatter = Arc::clone(&bus);
    tokio::spawn(async move {
        let mut seed: u32 = 7;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let (agent, _) = START[seed as usize % START.len()];
            chatter.publish(
                &format!("agent.{agent}.run"),
                json!({"text": "run: sleep 1; ls"}),
            );
        }
    });
    let asker = Arc::clone(&bus);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        loop {
            asker.publish(
                "agent.builder.run",
                json!({"text": "run: sudo systemctl restart demo"}),
            );
            tokio::time::sleep(Duration::from_secs(25)).await;
        }
    });
    Ok(Demo {
        nodes,
        gateway,
        workdir: base,
    })
}
