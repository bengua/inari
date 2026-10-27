//! Attaching a new core: the page asks for an invitation, the new machine joins with the
//! code, writes its configuration, starts, and the mesh sees it.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_core_joins_with_an_invitation_from_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("mesh.key"), "mesh").unwrap();
    std::fs::write(d.join("device.key"), "device").unwrap();
    std::fs::write(d.join("token"), "tok").unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake_acp.py");
    let fake = json!({"fake": {"command": ["python3", script], "env": {"FAKE_ACP_LOG": d.join("acp.log")}}});
    let hub_cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "hub", "data_dir": d.join("hub"), "key_file": d.join("mesh.key"),
        "device_key_file": d.join("device.key"), "gateway_listen": "127.0.0.1:0",
        "gateway_token_file": d.join("token"), "enroll_port": free_port(), "heartbeat": 0.1,
        "harnesses": fake,
    }))
    .unwrap();
    let hub = Node::start(hub_cfg).await.unwrap();
    let g = hub.gateway.clone().unwrap();
    let port = hub.gateway_port;
    let mut enroll = String::new();
    for _ in 0..100 {
        enroll = g.snapshot()["nodes"][0]["capabilities"]["enroll"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if !enroll.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        enroll,
        format!("127.0.0.1:{}", hub.enroll_port),
        "the hub tells where to join"
    );

    let ask = |node: &str| serde_json::to_vec(&json!({"node_id": node})).unwrap();
    let url = format!("http://127.0.0.1:{port}/api/invite?token=tok");
    let (status, _) = http(
        "POST",
        &url,
        Some(&ask("bad name!")),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(status, 400);
    let (_, raw) = http("POST", &url, Some(&ask("vps")), Duration::from_secs(5))
        .await
        .unwrap();
    let invite: Value = serde_json::from_slice(&raw).unwrap();
    let code = invite["code"].as_str().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let grant = inari_core::enroll::join(&enroll, code).await.unwrap();
    assert_eq!(grant.hub, format!("127.0.0.1:{}", hub.bus_port));
    let conf = d.join("vps-config");
    let file = inari_core::enroll::write_config(&grant, &conf, &d.join("vps-data"), 0, 0).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(conf.join("mesh.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut joined: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    joined["harnesses"] = fake; // this test machine's harnesses
    let vps = Node::start(serde_json::from_value(joined).unwrap())
        .await
        .unwrap();
    let mut names = Vec::new();
    for _ in 0..100 {
        names = g.snapshot()["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["node_id"].as_str().map(String::from))
            .collect();
        if names.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(names, ["hub", "vps"]);
    assert!(
        inari_core::enroll::join(&enroll, code).await.is_err(),
        "the code is spent"
    );
    drop(vps);
}
