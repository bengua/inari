//! A harness update from the web page: the node sees a newer version, installs it,
//! runs the restart command, and announces the new version — with a fake npm.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::time::Duration;

const FAKE_NPM: &str = r#"#!/bin/sh
# view <pkg> version | install -g --prefix <dir> <pkg>@<version>
case "$1" in
  view) echo "2.0.0" ;;
  install) pkg=${5%@*}; mkdir -p "$4/lib/node_modules/$pkg"
           echo "{\"version\": \"${5##*@}\"}" > "$4/lib/node_modules/$pkg/package.json" ;;
esac
"#;

async fn call(port: u16, path: &str, body: Option<Value>) -> Value {
    let url = format!("http://127.0.0.1:{port}{path}?token=t");
    let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
    let method = if bytes.is_some() { "POST" } else { "GET" };
    let (_, raw) = http(method, &url, bytes.as_deref(), Duration::from_secs(10))
        .await
        .unwrap();
    serde_json::from_slice(&raw).unwrap_or(Value::Null)
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
async fn a_harness_is_updated_and_restarted_from_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let npm = d.join("npm");
    std::fs::write(&npm, FAKE_NPM).unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&npm)
        .status()
        .unwrap();
    let pkg = d.join("prefix/lib/node_modules/fake-agent");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("package.json"), r#"{"version": "1.0.0"}"#).unwrap();
    for (name, value) in [("mesh.key", "m"), ("device.key", "d"), ("token", "t")] {
        std::fs::write(d.join(name), value).unwrap();
    }
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "hub", "data_dir": d.join("hub"), "key_file": d.join("mesh.key"),
        "device_key_file": d.join("device.key"), "gateway_listen": "127.0.0.1:0",
        "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {
            "fake": {"command": [script, "fake-agent"], "env": {"FAKE_ACP_LOG": d.join("acp.log")},
                     "update": {"npm": "fake-agent", "prefix": d.join("prefix"), "npm_bin": npm,
                                "restart": ["touch", d.join("restarted")]}},
            "broken": {"command": [script, "fake-agent"], "env": {"FAKE_ACP_LOG": d.join("acp.log")},
                       "update": {"npm": "nope", "prefix": d.join("prefix"), "npm_bin": d.join("missing-npm")}},
        },
    }))
    .unwrap();
    let hub = Node::start(cfg).await.unwrap();
    let g = hub.gateway.clone().unwrap();
    let port = hub.gateway_port;
    let updates = |g: &inari_core::gateway::Gateway| {
        g.snapshot()["nodes"][0]["capabilities"]["updates"].clone()
    };
    until("the update offer", || {
        updates(&g)["fake"]["available"] == true
    })
    .await;
    assert_eq!(updates(&g)["fake"]["current"], "1.0.0");
    assert_eq!(updates(&g)["fake"]["latest"], "2.0.0");
    assert!(updates(&g)["broken"]["error"]
        .as_str()
        .unwrap()
        .contains("cannot start"));

    call(
        port,
        "/api/update",
        Some(json!({"node": "hub", "harness": "fake"})),
    )
    .await;
    until("the update", || {
        g.snapshot()["update_jobs"]["hub/fake"]["state"] == "done"
    })
    .await;
    assert_eq!(g.snapshot()["update_jobs"]["hub/fake"]["current"], "2.0.0");
    assert!(d.join("restarted").exists(), "the restart command ran");
    until("the new version announced", || {
        updates(&g)["fake"]["available"] == false
    })
    .await;

    call(
        port,
        "/api/update",
        Some(json!({"node": "hub", "harness": "broken"})),
    )
    .await;
    until("the failure", || {
        g.snapshot()["update_jobs"]["hub/broken"]["state"] == "failed"
    })
    .await;
    call(port, "/api/update/check", Some(json!({}))).await;
}
