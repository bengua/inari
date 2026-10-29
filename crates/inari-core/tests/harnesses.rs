//! A core's agents and subscriptions, set from the interface: added, tried, used by a thread,
//! kept across a restart, taken away; never by editing the configuration file.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::time::Duration;

async fn call(port: u16, path: &str, body: Value) -> (u16, Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let (status, raw) = http(
        "POST",
        &format!("http://127.0.0.1:{port}{path}?token=owner"),
        Some(&bytes),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..300 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_are_added_tried_used_kept_and_taken_away_from_the_interface() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_path_buf();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let config = json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {"local": {"command": [script, "fake-agent"]}, "old": {"command": [script, "fake-agent"]}},
    });
    let home = Node::start(serde_json::from_value::<NodeConfig>(config.clone()).unwrap())
        .await
        .unwrap();
    let g = home.gateway.clone().unwrap();
    let port = home.gateway_port;
    let result = |g: &inari_core::gateway::Gateway| g.snapshot()["harness_results"]["home"].clone();
    let specs = |g: &inari_core::gateway::Gateway| {
        g.snapshot()["nodes"][0]["capabilities"]["harness_specs"].clone()
    };

    // tried before it is added: the agent answers with its models
    let command = format!("{script} fake-agent");
    call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "test", "name": "kimi", "command": command}),
    )
    .await;
    until("the test", || result(&g)["action"] == "test").await;
    assert_eq!(result(&g)["ok"], true, "{}", result(&g));
    assert!(result(&g)["detail"]
        .as_str()
        .unwrap()
        .starts_with("It answers"));
    // a program that is not there is said so
    call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "test", "name": "x", "command": "no-such-agent-acp"}),
    )
    .await;
    until("the second test", || {
        result(&g)["error"]
            .as_str()
            .is_some_and(|e| e.contains("not installed"))
    })
    .await;

    // added: listed, and a thread runs on it at once
    let (status, _) = call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "add", "name": "kimi", "preset": "kimi", "command": command, "npm": "kimi-acp"}),
    )
    .await;
    assert_eq!(status, 200);
    until("kimi listed", || specs(&g)["kimi"]["source"] == "interface").await;
    assert_eq!(specs(&g)["kimi"]["npm"], "kimi-acp");
    assert_eq!(specs(&g)["local"]["default"], true);
    let runner = home.runner.clone().unwrap();
    let agent = runner.create("t1", "kimi", "a thread").unwrap();
    let out = runner.run(&agent, "hello", vec![]).await;
    assert_eq!(out.final_text, "fake reply to: hello");
    // a bad one never reaches the core
    call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "add", "name": "../x", "command": command}),
    )
    .await;
    until("the refusal", || result(&g)["ok"] == false).await;

    // one of the configuration's taken away, and the default kept
    call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "remove", "name": "old"}),
    )
    .await;
    until("old gone", || specs(&g).get("old").is_none()).await;
    call(
        port,
        "/api/harnesses",
        json!({"node": "home", "action": "remove", "name": "local"}),
    )
    .await;
    until("the default kept", || {
        result(&g)["error"]
            .as_str()
            .is_some_and(|e| e.contains("default"))
    })
    .await;

    // a restart keeps what the interface chose
    let kept: Value =
        serde_json::from_slice(&std::fs::read(d.join("home").join("harnesses.json")).unwrap())
            .unwrap();
    assert_eq!(kept["old"], Value::Null);
    assert_eq!(kept["kimi"]["command"][1], "fake-agent");
    // a guest never reaches it
    let (status, _) = http(
        "POST",
        &format!("http://127.0.0.1:{port}/api/harnesses?pass=nothing"),
        Some(b"{}"),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_ne!(status, 200);
}
