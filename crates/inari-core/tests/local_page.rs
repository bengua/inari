//! Each machine serves the interface itself: a worker's gateway sees the whole mesh,
//! and a page opened on the machine needs no token, while another site's page and a
//! rebinding host name are refused.

use inari_core::node::{Node, NodeConfig};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn raw(port: u16, head: &str, body: &str) -> (u16, String) {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (
        status,
        text.split_once("\r\n\r\n")
            .map(|x| x.1.to_string())
            .unwrap_or_default(),
    )
}

fn config(d: &Path, id: &str, hub: &str, gateway: bool) -> NodeConfig {
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let mut cfg = json!({
        "node_id": id, "data_dir": d.join(id), "key_file": d.join("mesh.key"), "hub": hub,
        "heartbeat": 0.1, "expected_peers": ["worker"],
        "harnesses": {"fake": {"command": [script, "fake-agent"], "env": {"FAKE_ACP_LOG": d.join("acp.log")}}},
    });
    if gateway {
        cfg["gateway_listen"] = json!("127.0.0.1:0");
        cfg["gateway_token_file"] = json!(d.join("token"));
        cfg["gateway_trust_loopback"] = json!(true);
    }
    serde_json::from_value(cfg).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_serves_its_own_page_to_this_machine_only() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("mesh.key"), "m").unwrap();
    std::fs::write(d.join("token"), "tok").unwrap();
    let hub = Node::start(config(d, "hub", "", false)).await.unwrap();
    let worker = Node::start(config(
        d,
        "worker",
        &format!("127.0.0.1:{}", hub.bus_port),
        true,
    ))
    .await
    .unwrap();
    let port = worker.gateway_port;
    let local = format!("Host: 127.0.0.1:{port}");

    // the worker's page sees the hub too
    let mut seen = Value::Null;
    for _ in 0..100 {
        let (status, body) =
            raw(port, &format!("GET /api/search?q= HTTP/1.1\r\n{local}"), "").await;
        assert_eq!(status, 200, "a local page needs no token");
        seen = worker.gateway.as_ref().unwrap().snapshot()["nodes"].clone();
        if seen.as_array().map(Vec::len) == Some(2) {
            assert!(body.contains("hits"));
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(seen.as_array().map(Vec::len), Some(2), "{seen}");

    let post = |extra: &str, ctype: &str| {
        format!("POST /api/update/check HTTP/1.1\r\n{local}\r\nContent-Type: {ctype}{extra}")
    };
    assert_eq!(raw(port, &post("", "application/json"), "{}").await.0, 200);
    // another site's page, a rebinding name, a form-like POST: no
    assert_eq!(
        raw(
            port,
            &post("\r\nOrigin: http://evil.example", "application/json"),
            "{}"
        )
        .await
        .0,
        401
    );
    assert_eq!(
        raw(
            port,
            &post("\r\nSec-Fetch-Site: cross-site", "application/json"),
            "{}"
        )
        .await
        .0,
        401
    );
    let rebound = format!("GET /api/search?q= HTTP/1.1\r\nHost: evil.example:{port}");
    assert_eq!(raw(port, &rebound, "").await.0, 401);
    assert_eq!(raw(port, &post("", "text/plain"), "{}").await.0, 400);
    // the token still works from anywhere
    let with_token = format!("GET /api/search?q=&token=tok HTTP/1.1\r\nHost: evil.example:{port}");
    assert_eq!(raw(port, &with_token, "").await.0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_and_its_assets_are_served_with_their_version() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("mesh.key"), "m").unwrap();
    std::fs::write(d.join("token"), "tok").unwrap();
    let hub = Node::start(config(d, "hub", "", true)).await.unwrap();
    let port = hub.gateway_port;
    let host = format!("Host: 127.0.0.1:{port}");

    let (status, page) = raw(port, &format!("GET / HTTP/1.1\r\n{host}"), "").await;
    assert_eq!(status, 200);
    assert!(
        !page.contains("__INARI_PAGE__"),
        "every version mark is filled in"
    );
    let (status, css) = raw(port, &format!("GET /assets/app.css HTTP/1.1\r\n{host}"), "").await;
    assert_eq!(status, 200);
    // everything the page and its style ask for under /assets/ is there
    for text in [&page, &css] {
        for part in text.split("/assets/").skip(1) {
            let name: String = part
                .chars()
                .take_while(|c| !matches!(c, '?' | '"' | ')'))
                .collect();
            let (status, body) =
                raw(port, &format!("GET /assets/{name} HTTP/1.1\r\n{host}"), "").await;
            assert_eq!(status, 200, "{name}");
            assert!(!body.is_empty(), "{name}");
        }
    }
    let (status, _) = raw(
        port,
        &format!("GET /assets/../Cargo.toml HTTP/1.1\r\n{host}"),
        "",
    )
    .await;
    assert_eq!(status, 404);
}
