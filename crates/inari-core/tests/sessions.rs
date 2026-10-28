//! A page signs in once and holds a cookie, not a token: the owner's session lasts while the
//! token does, a pass's while the pass does; wrong secrets make the address wait; and every
//! page carries a policy that forbids inline script and framing.

use inari_core::node::{Node, NodeConfig};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One request; the answer's status, head and body.
async fn raw(
    port: u16,
    method: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (u16, String, Value) {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let cookie = if cookie.is_empty() {
        String::new()
    } else {
        format!("Cookie: {cookie}\r\n")
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: inari.example\r\n{cookie}Content-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (
        status,
        head.to_string(),
        serde_json::from_str(rest).unwrap_or(Value::Null),
    )
}

/// The `name=value` a `Set-Cookie` header carries.
fn cookie_of(head: &str) -> String {
    let line = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("a cookie is set");
    let value = line.split_once(':').unwrap().1.trim();
    value.split(';').next().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_open_with_a_secret_and_end_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [
        ("mesh.key", "m"),
        ("device.key", "d"),
        ("token", "owner-token"),
    ] {
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

    // the page and its answers carry the policy
    let (_, head, _) = raw(port, "GET", "/", "", "").await;
    assert!(
        head.contains("script-src 'self';"),
        "no inline script: {head}"
    );
    assert!(head.contains("frame-ancestors 'none'"));
    assert!(head.contains("Referrer-Policy: no-referrer"));

    // nothing without a session
    assert_eq!(raw(port, "GET", "/api/whoami", "", "").await.0, 401);

    // the owner signs in: a cookie the page's script cannot read, never sent by another site
    let (status, head, me) = raw(
        port,
        "POST",
        "/api/session",
        "",
        r#"{"secret":"owner-token"}"#,
    )
    .await;
    assert_eq!((status, me["role"].as_str()), (200, Some("owner")));
    assert!(head.contains("HttpOnly") && head.contains("SameSite=Strict"));
    assert!(
        !head.contains("Secure"),
        "plain HTTP here: the cookie must still be sent back"
    );
    let owner = cookie_of(&head);
    assert!(
        !owner.contains("owner-token"),
        "the token itself is never in the cookie"
    );
    let (status, _, me) = raw(port, "GET", "/api/whoami", &owner, "").await;
    assert_eq!((status, me["role"].as_str()), (200, Some("owner")));

    // a forged or altered cookie opens nothing
    let forged = owner.replacen(".o.", ".p.", 1);
    assert_eq!(raw(port, "GET", "/api/whoami", &forged, "").await.0, 401);

    // a pass signs in as a guest, and its session ends when the pass is revoked
    let (_, _, made) = raw(
        port,
        "POST",
        "/api/pass",
        &owner,
        r#"{"name":"Léo","role":"supervision","harnesses":[]}"#,
    )
    .await;
    let (pid, token) = (
        made["id"].as_str().unwrap(),
        made["token"].as_str().unwrap(),
    );
    let mut guest_head = String::new();
    for _ in 0..100 {
        let (status, head, _) = raw(
            port,
            "POST",
            "/api/session",
            "",
            &json!({"secret": token}).to_string(),
        )
        .await;
        if status == 200 {
            guest_head = head;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await; // the hub tells the gateway
    }
    let guest = cookie_of(&guest_head);
    let (_, _, me) = raw(port, "GET", "/api/whoami", &guest, "").await;
    assert_eq!(me["role"].as_str(), Some("supervision"));
    raw(
        port,
        "POST",
        "/api/pass/revoke",
        &owner,
        &json!({"id": pid}).to_string(),
    )
    .await;
    let mut ended = false;
    for _ in 0..100 {
        if raw(port, "GET", "/api/whoami", &guest, "").await.0 == 401 {
            ended = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ended, "a revoked pass ends its session");

    // signing out clears the cookie
    let (status, head, _) = raw(port, "POST", "/api/logout", &owner, "{}").await;
    assert_eq!(status, 200);
    assert!(head.contains("Max-Age=0"));

    // wrong secrets: a few are answered, then the address waits
    let mut last = 0;
    for _ in 0..7 {
        last = raw(port, "POST", "/api/session", "", r#"{"secret":"guess"}"#)
            .await
            .0;
    }
    assert_eq!(last, 429);
    let (status, _, _) = raw(
        port,
        "POST",
        "/api/session",
        "",
        r#"{"secret":"owner-token"}"#,
    )
    .await;
    assert_eq!(status, 429, "even the right secret waits its turn");
}
