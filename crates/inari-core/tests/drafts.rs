//! The answer a model is writing reaches the pages as it comes (SSE `draft` events): the owner's
//! page gets it, a page opened in the middle of it gets it at once, a guest never gets a thread
//! that is not theirs, and an empty draft (the answer is journaled) ends it.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What the event stream says within `wait`.
async fn listen(port: u16, auth: String, wait: Duration) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(format!("GET /events?{auth} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (mut got, mut buf, end) = (Vec::new(), [0u8; 8192], Instant::now() + wait);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        match tokio::time::timeout(left, s.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => got.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_reaches_only_the_pages_that_may_see_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"),
        "device_key_file": d.join("device.key"), "gateway_listen": "127.0.0.1:0",
        "gateway_token_file": d.join("token"), "heartbeat": 0.1,
    }))
    .unwrap();
    let home = Node::start(cfg).await.unwrap();
    let port = home.gateway_port;
    let (_, raw) = http(
        "POST",
        &format!("http://127.0.0.1:{port}/api/pass?token=owner"),
        Some(br#"{"name": "Leo", "role": "supervision", "harnesses": []}"#),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    let made: Value = serde_json::from_slice(&raw).unwrap();
    let guest = format!("pass={}", made["token"].as_str().unwrap());
    let whoami = format!("http://127.0.0.1:{port}/api/whoami?{guest}");
    for _ in 0..100 {
        if http("GET", &whoami, None, Duration::from_secs(5))
            .await
            .map(|r| r.0)
            == Ok(200)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // the owner's page and the guest's page are open while the model writes in the owner's thread
    let (bus, owner_page, guest_page) = (
        home.bus.clone(),
        tokio::spawn(listen(
            port,
            "token=owner".into(),
            Duration::from_millis(1500),
        )),
        tokio::spawn(listen(port, guest.clone(), Duration::from_millis(1500))),
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    bus.publish(
        "draft.mine",
        json!({"agent": "mine", "text": "Hello, I am wri"}),
    );
    let (owner_saw, guest_saw) = (owner_page.await.unwrap(), guest_page.await.unwrap());
    assert!(
        owner_saw.contains("event: draft\ndata: {\"agent\":\"mine\",\"text\":\"Hello, I am wri\"}"),
        "{owner_saw}"
    );
    assert!(
        guest_saw.contains("data: {"),
        "the guest's page is open: {guest_saw}"
    );
    assert!(
        !guest_saw.contains("event: draft"),
        "never another's thread: {guest_saw}"
    );

    // a page opened in the middle of the answer gets it at once
    let late = listen(port, "token=owner".into(), Duration::from_millis(500)).await;
    assert!(late.contains("\"text\":\"Hello, I am wri\""), "{late}");

    // journaled: the draft ends, and a page opened afterwards has none
    bus.publish("draft.mine", json!({"agent": "mine", "text": ""}));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = listen(port, "token=owner".into(), Duration::from_millis(500)).await;
    assert!(!after.contains("event: draft"), "{after}");
}
