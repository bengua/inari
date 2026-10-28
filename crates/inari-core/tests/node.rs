//! Two Rust nodes, the hub's gateway, the scripted fake ACP agent: the whole path a
//! person takes from the web page or the phone — create, attach in parts, talk,
//! approve, look at files, run a command, teleport, talk again on the other core.

use base64::Engine;
use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-token";

fn config(dir: &Path, id: &str, hub: &str, log: &Path) -> NodeConfig {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake_acp.py");
    std::fs::write(dir.join("mesh.key"), "mesh-secret").unwrap();
    std::fs::write(dir.join("device.key"), "device-secret").unwrap();
    std::fs::write(dir.join("token"), TOKEN).unwrap();
    let mut cfg = json!({
        "node_id": id, "data_dir": dir.join(id), "key_file": dir.join("mesh.key"),
        "device_key_file": dir.join("device.key"), "hub": hub, "heartbeat": 0.2, "registry_ttl": 3.0,
        "expected_peers": ["peer"], "approval_ttl": 10.0,
        "harnesses": {"fake": {"command": ["python3", script], "env": {"FAKE_ACP_LOG": log}}},
    });
    if hub.is_empty() {
        cfg["gateway_listen"] = json!("127.0.0.1:0");
        cfg["gateway_token_file"] = json!(dir.join("token"));
        cfg["usage"] = json!([{"kind": "static", "items": [{"provider_id": "local", "name": "Local models"}]}]);
    }
    serde_json::from_value(cfg).unwrap()
}

async fn call(port: u16, path: &str, body: Option<Value>) -> (u16, Value) {
    let sep = if path.contains('?') { '&' } else { '?' };
    let url = format!("http://127.0.0.1:{port}{path}{sep}token={TOKEN}");
    let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
    let method = if bytes.is_some() { "POST" } else { "GET" };
    let (status, raw) = http(method, &url, bytes.as_deref(), Duration::from_secs(80))
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

async fn done(rx: &mut tokio::sync::mpsc::UnboundedReceiver<inari_core::bus::Message>) -> Value {
    tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("the turn ended")
        .unwrap()
        .payload
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_drives_two_rust_cores_through_the_gateway() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("acp.log");
    let hub = Node::start(config(dir.path(), "hub", "", &log))
        .await
        .unwrap();
    let peer = Node::start(config(
        dir.path(),
        "peer",
        &format!("127.0.0.1:{}", hub.bus_port),
        &log,
    ))
    .await
    .unwrap();
    let g = Arc::clone(hub.gateway.as_ref().unwrap());
    let port = hub.gateway_port;
    until("both nodes", || {
        g.snapshot()["nodes"].as_array().map(Vec::len) == Some(2)
    })
    .await;
    until("the usage", || {
        g.snapshot()["usage"][0]["provider_id"] == "local"
    })
    .await;

    // the page is public, everything else needs the token
    let (status, page) = http(
        "GET",
        &format!("http://127.0.0.1:{port}/"),
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&page).contains("<title>"));
    // the page carries its version, the snapshot names the one served: an older open page reloads
    let version = g.snapshot()["ui"]["page"].as_str().unwrap().to_string();
    assert_eq!(version.len(), 12);
    assert!(String::from_utf8_lossy(&page)
        .contains(&format!("<meta name=\"inari-page\" content=\"{version}\">")));
    let (status, _) = http(
        "GET",
        &format!("http://127.0.0.1:{port}/api/search?q=x"),
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(status, 401);

    call(
        port,
        "/api/agent",
        Some(json!({"thread_id": "t1", "node": "hub", "title": "first"})),
    )
    .await;
    until("the agent", || {
        g.snapshot()["agents"]["t1"]["node"] == "hub"
    })
    .await;
    assert_eq!(g.snapshot()["agents"]["t1"]["title"], "first");
    assert_eq!(
        g.snapshot()["ui"]["self"],
        "hub",
        "the page knows which core serves it"
    );

    // a photo in two parts, as the phone sends it
    let photo: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 3000]].concat();
    let b64 = base64::engine::general_purpose::STANDARD;
    let (head, tail) = photo.split_at(1000);
    let part = |offset: usize, bytes: &[u8]| json!({"agent": "t1", "path": "photo.png", "upload_id": "up1", "offset": offset, "total": photo.len(), "b64": b64.encode(bytes)});
    let (_, first) = call(port, "/api/upload", Some(part(0, head))).await;
    assert_eq!(first["received"], 1000, "{first}");
    let (_, last) = call(port, "/api/upload", Some(part(1000, tail))).await;
    assert_eq!(
        (last["ok"].as_bool(), last["mime"].as_str()),
        (Some(true), Some("image/png")),
        "{last}"
    );

    let mut turns = hub.bus.subscribe("agent.t1.done");
    call(
        port,
        "/api/message",
        Some(json!({"agent": "t1", "text": "please make a file", "attachments": ["photo.png"]})),
    )
    .await;
    let out = done(&mut turns).await;
    assert_eq!(out["stopped"], "done", "{out}");
    let sent: Value = serde_json::from_str(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sent["blocks"], json!(["text", "resource_link"]));
    assert!(sent["prompt"]
        .as_str()
        .unwrap()
        .contains("photo.png (image/png, 3008 bytes)"));

    // the fake agent echoes the last line of its prompt: the list of attachments
    until("the journal", || {
        g.search("photo.png", "t1")
            .iter()
            .any(|h| h["kind"] == "assistant_message")
    })
    .await;
    let (_, timeline) = call(port, "/api/agent/t1/timeline", None).await;
    let kinds: Vec<&str> = timeline["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"user_message") && kinds.contains(&"assistant_message"),
        "{kinds:?}"
    );
    let (_, files) = call(port, "/api/agent/t1/files", None).await;
    let names: Vec<&str> = files["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert_eq!(names, ["made.txt", "photo.png"]);
    let (_, file) = call(port, "/api/agent/t1/file?path=made.txt", None).await;
    assert_eq!(file["text"], "made by the fake agent");
    let (_, image) = call(port, "/api/agent/t1/file?path=photo.png", None).await;
    assert_eq!(b64.decode(image["b64"].as_str().unwrap()).unwrap(), photo);
    let (_, exec) = call(
        port,
        "/api/exec",
        Some(json!({"agent": "t1", "cmd": "wc -c < made.txt"})),
    )
    .await;
    assert_eq!(exec["output"].as_str().map(str::trim), Some("22"), "{exec}");

    // the agent asks; the person says yes from the page; the signed verdict lets it through
    call(
        port,
        "/api/message",
        Some(json!({"agent": "t1", "text": "perm"})),
    )
    .await;
    until("the question", || {
        g.snapshot()["approvals"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    })
    .await;
    let question = g.snapshot()["approvals"][0].clone();
    assert_eq!(question["command"], "bash: rm -rf x");
    let (status, _) = call(
        port,
        "/api/approval",
        Some(json!({"request_id": question["request_id"], "approved": true})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(done(&mut turns).await["text"], "permission result: allow");

    // to the other core, and on with the conversation there
    let (_, req) = call(
        port,
        "/api/teleport",
        Some(json!({"agent": "t1", "source": "hub", "target": "peer"})),
    )
    .await;
    until("the move", || {
        g.snapshot()["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["request"] == req["request"])
    })
    .await;
    let result = g.snapshot()["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["request"] == req["request"])
        .cloned()
        .unwrap();
    assert_eq!(result["ok"], true, "{result}");
    until("the agent on the peer", || {
        g.snapshot()["agents"]["t1"]["node"] == "peer"
    })
    .await;
    assert_eq!(peer.host.status("t1"), "running");
    assert_ne!(hub.host.status("t1"), "running");
    call(
        port,
        "/api/message",
        Some(json!({"agent": "t1", "text": "still there?"})),
    )
    .await;
    let out = done(&mut turns).await;
    assert_eq!(out["text"], "fake reply to: still there?", "{out}");
    let (_, files) = call(port, "/api/agent/t1/files", None).await;
    assert_eq!(
        files["files"].as_array().unwrap().len(),
        2,
        "the workspace travelled"
    );
}
