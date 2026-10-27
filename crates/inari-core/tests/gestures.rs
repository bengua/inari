//! What the interface's gestures ask of the core: clone a thread onto another harness,
//! share a thread with a pass, put a thread to sleep and wake it.

use inari_core::gateway::Access;
use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

async fn post(port: u16, auth: &str, path: &str, body: Value) -> (u16, Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let (status, raw) = http(
        "POST",
        &format!("http://127.0.0.1:{port}{path}?{auth}"),
        Some(&bytes),
        Duration::from_secs(20),
    )
    .await
    .unwrap();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn clone_share_and_sleep() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake_acp.py");
    let h =
        |log: &str| json!({"command": ["python3", script], "env": {"FAKE_ACP_LOG": d.join(log)}});
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {"local": h("local.log"), "frontier": h("frontier.log")},
    }))
    .unwrap();
    let home = Node::start(cfg).await.unwrap();
    let g = home.gateway.clone().unwrap();
    let port = home.gateway_port;
    let owner = "token=owner";
    post(
        port,
        owner,
        "/api/agent",
        json!({"thread_id": "essay", "node": "home", "harness": "local"}),
    )
    .await;
    until("the thread", || {
        g.snapshot()["agents"]["essay"]["harness"] == "local"
    })
    .await;
    let mut done = home.bus.subscribe("agent.essay.done");
    post(
        port,
        owner,
        "/api/message",
        json!({"agent": "essay", "text": "draft it"}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(15), done.recv())
        .await
        .unwrap();

    // clone onto another harness: same journal, other mind
    let (_, made) = post(
        port,
        owner,
        "/api/clone",
        json!({"agent": "essay", "harness": "frontier", "label": "Frontier"}),
    )
    .await;
    let copy = made["thread_id"].as_str().unwrap().to_string();
    until("the clone", || {
        g.snapshot()["agents"][&copy]["harness"] == "frontier"
    })
    .await;
    let timeline = |agent: &str| {
        let c = inari_core::capsule::io::load(&home.host.agents.join(agent)).unwrap();
        c.timeline
            .ordered()
            .iter()
            .map(|e| {
                e.data
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    assert!(
        timeline(&copy).iter().any(|t| t == "draft it"),
        "the clone carries the journal"
    );

    until("its title", || {
        g.snapshot()["agents"][&copy]["title"] == "essay · Frontier"
    })
    .await;

    // share with a pass: the guest now sees and uses that thread
    let (_, pass) = post(
        port,
        owner,
        "/api/pass",
        json!({"name": "Ana", "role": "collaboration", "harnesses": ["local"]}),
    )
    .await;
    let (pid, token) = (
        pass["id"].as_str().unwrap().to_string(),
        pass["token"].as_str().unwrap().to_string(),
    );
    let guest = Access::Guest(
        json!({"id": pid, "name": "Ana", "role": "collaboration", "harnesses": ["local"]}),
    );
    until("the pass", || g.access_of(Some(&token), false).is_some()).await;
    assert!(
        g.snapshot_for(&guest)["agents"].get("essay").is_none(),
        "not shared yet"
    );
    assert_eq!(
        post(
            port,
            &format!("pass={token}"),
            "/api/message",
            json!({"agent": "essay", "text": "hi"})
        )
        .await
        .0,
        403
    );
    post(
        port,
        owner,
        "/api/share",
        json!({"agent": "essay", "pass": pid}),
    )
    .await;
    until("the share", || {
        g.snapshot_for(&guest)["agents"].get("essay").is_some()
    })
    .await;
    assert_eq!(
        post(
            port,
            &format!("pass={token}"),
            "/api/message",
            json!({"agent": "essay", "text": "hi"})
        )
        .await
        .0,
        200
    );
    assert!(
        g.snapshot_for(&guest)["agents"].get(&copy).is_none(),
        "only what was shared"
    );

    // merge: the target's journal, as the page reads it, holds the source's events
    post(
        port,
        owner,
        "/api/merge",
        json!({"target": "essay", "source": copy}),
    )
    .await;
    until("the merged journal", || {
        !g.search("draft it", "essay").is_empty()
            && g.snapshot()["notices"]
                .as_array()
                .is_some_and(|n| n.iter().any(|x| x["subject"] == "agent.merged"))
    })
    .await;

    // delete: gone from the mesh, kept in the core's trash
    post(
        port,
        owner,
        "/api/agent",
        json!({"thread_id": "scratch", "node": "home", "harness": "local"}),
    )
    .await;
    until("the scratch thread", || {
        g.snapshot()["agents"].get("scratch").is_some()
    })
    .await;
    let (status, _) = post(
        port,
        &format!("pass={token}"),
        "/api/delete",
        json!({"agent": "essay"}),
    )
    .await;
    assert_eq!(status, 403, "a pass holder deletes nothing");
    post(port, owner, "/api/delete", json!({"agent": "scratch"})).await;
    until("the deletion", || {
        g.snapshot()["agents"].get("scratch").is_none()
    })
    .await;
    assert!(!home.host.agents.join("scratch").exists());
    let trashed: Vec<_> = std::fs::read_dir(home.host.root.join("trash"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        trashed.iter().any(|n| n.starts_with("scratch-")),
        "kept in the trash: {trashed:?}"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        g.snapshot()["agents"].get("scratch").is_none(),
        "heartbeats do not bring it back"
    );

    // sleep and wake
    post(
        port,
        owner,
        "/api/archive",
        json!({"agent": copy, "archived": true}),
    )
    .await;
    until("asleep", || {
        g.snapshot()["agents"][&copy]["archived"] == true
    })
    .await;
    post(
        port,
        owner,
        "/api/archive",
        json!({"agent": copy, "archived": false}),
    )
    .await;
    until("awake", || {
        g.snapshot()["agents"][&copy]["archived"] == false
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clone_goes_to_the_core_that_has_the_harness() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake_acp.py");
    let h =
        |log: &str| json!({"command": ["python3", script], "env": {"FAKE_ACP_LOG": d.join(log)}});
    let home: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1, "expected_peers": ["away"],
        "harnesses": {"local": h("a.log")},
    }))
    .unwrap();
    let home = Node::start(home).await.unwrap();
    let away: NodeConfig = serde_json::from_value(json!({
        "node_id": "away", "data_dir": d.join("away"), "key_file": d.join("mesh.key"), "hub": format!("127.0.0.1:{}", home.bus_port),
        "heartbeat": 0.1, "harnesses": {"local": h("b.log"), "gem": h("c.log")},
    }))
    .unwrap();
    let _away = Node::start(away).await.unwrap();
    let g = home.gateway.clone().unwrap();
    let port = home.gateway_port;
    until("both cores", || {
        g.snapshot()["nodes"]
            .as_array()
            .is_some_and(|n| n.len() == 2)
    })
    .await;
    post(
        port,
        "token=owner",
        "/api/agent",
        json!({"thread_id": "essay", "node": "home"}),
    )
    .await;
    until("the thread", || {
        g.snapshot()["agents"].get("essay").is_some()
    })
    .await;
    let (_, made) = post(
        port,
        "token=owner",
        "/api/clone",
        json!({"agent": "essay", "harness": "gem", "label": "Gem"}),
    )
    .await;
    let copy = made["thread_id"].as_str().unwrap().to_string();
    until("the copy on the core that has the harness", || {
        let a = &g.snapshot()["agents"][&copy];
        a["node"] == "away" && a["harness"] == "gem" && a["title"] == "essay · Gem"
    })
    .await;
    assert_eq!(
        g.snapshot()["agents"]["essay"]["node"],
        "home",
        "the original stays"
    );
    let (status, _) = post(
        port,
        "token=owner",
        "/api/clone",
        json!({"agent": "essay", "harness": "nowhere"}),
    )
    .await;
    assert_eq!(status, 404);
}
