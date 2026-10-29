//! The mascot's persona is each person's own choice: the owner and a pass holder pick theirs
//! from the personas FoxBot publishes, every page of theirs follows, and nobody else's changes.

use inari_core::gateway::Access;
use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

async fn call(port: u16, auth: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let sep = if path.contains('?') { '&' } else { '?' };
    let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
    let method = if bytes.is_some() { "POST" } else { "GET" };
    let (status, raw) = http(
        method,
        &format!("http://127.0.0.1:{port}{path}{sep}{auth}"),
        bytes.as_deref(),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

fn frames(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("f_00.webp"), b"x").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn each_person_shows_the_persona_they_chose() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let fox = d.join("fox");
    for a in ["respire", "cligne", "tourne"] {
        frames(&fox.join("sprites").join(a));
    }
    frames(&fox.join("sprites_persona/rebel/respire"));
    frames(&fox.join("sprites_persona/rebel/fume_clope"));
    frames(&fox.join("sprites_persona/zen/respire"));
    frames(&fox.join("sprites_persona/zen/cligne"));
    frames(&fox.join("sprites_persona/draft/respire")); // work in progress: not published
    std::fs::write(
        fox.join("personas.json"),
        r#"{"published": ["rebel", "zen"]}"#,
    )
    .unwrap();
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"),
        "device_key_file": d.join("device.key"), "gateway_listen": "127.0.0.1:0",
        "gateway_token_file": d.join("token"), "heartbeat": 0.1, "mascot_dir": fox,
    }))
    .unwrap();
    let home = Node::start(cfg).await.unwrap();
    let (g, port) = (home.gateway.clone().unwrap(), home.gateway_port);
    let owner = "token=owner";

    let (status, got) = call(port, owner, "/api/personas", None).await;
    assert_eq!(status, 200);
    assert_eq!(got["current"], "", "no choice yet: the original FoxBot");
    assert_eq!(got["personas"][0]["id"], "rebel");
    assert_eq!(
        (
            got["personas"][1]["id"].as_str(),
            got["personas"][1]["shared"].as_u64(),
            got["personas"][1]["of"].as_u64()
        ),
        (Some("zen"), Some(2), Some(3))
    );
    assert_eq!(got["personas"][0]["signatures"], json!(["fume_clope"]));
    assert_eq!(
        got["personas"].as_array().unwrap().len(),
        2,
        "only what FoxBot publishes"
    );
    assert_eq!(
        call(
            port,
            owner,
            "/api/persona",
            Some(json!({"persona": "draft"}))
        )
        .await
        .0,
        400
    );

    // the owner picks zen: their snapshot says so, and it survives a restart of the choice store
    assert_eq!(
        call(port, owner, "/api/persona", Some(json!({"persona": "zen"})))
            .await
            .0,
        200
    );
    assert_eq!(g.snapshot_for(&Access::Owner)["ui"]["persona"], "zen");
    assert_eq!(
        call(
            port,
            owner,
            "/api/persona",
            Some(json!({"persona": "nope"}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            port,
            owner,
            "/api/persona",
            Some(json!({"persona": "../sprites"}))
        )
        .await
        .0,
        400
    );

    // a pass holder has their own choice, and it does not move the owner's
    let (_, made) = call(
        port,
        owner,
        "/api/pass",
        Some(json!({"name": "Léo", "role": "supervision", "harnesses": []})),
    )
    .await;
    let guest = format!("pass={}", made["token"].as_str().unwrap());
    let mut ok = false;
    for _ in 0..100 {
        if call(port, &guest, "/api/whoami", None).await.0 == 200 {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "the pass reaches the gateway");
    assert_eq!(
        call(port, &guest, "/api/personas", None).await.1["current"],
        ""
    );
    assert_eq!(
        call(
            port,
            &guest,
            "/api/persona",
            Some(json!({"persona": "rebel"}))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(port, &guest, "/api/personas", None).await.1["current"],
        "rebel"
    );
    assert_eq!(
        call(port, owner, "/api/personas", None).await.1["current"],
        "zen"
    );

    // a persona FoxBot stops publishing: whoever chose it sees the original FoxBot again
    std::fs::write(fox.join("personas.json"), r#"{"published": ["zen"]}"#).unwrap();
    assert_eq!(
        call(port, &guest, "/api/personas", None).await.1["current"],
        ""
    );
    assert_eq!(g.snapshot_for(&Access::Owner)["ui"]["persona"], "zen");
    std::fs::write(
        fox.join("personas.json"),
        r#"{"published": ["rebel", "zen"]}"#,
    )
    .unwrap();

    // back to the original FoxBot
    assert_eq!(
        call(port, owner, "/api/persona", Some(json!({"persona": ""})))
            .await
            .0,
        200
    );
    assert_eq!(g.snapshot_for(&Access::Owner)["ui"]["persona"], "");
    let kept: Value =
        serde_json::from_slice(&std::fs::read(d.join("home/personas.json")).unwrap()).unwrap();
    assert_eq!(kept["owner"], "");
    assert!(kept
        .as_object()
        .unwrap()
        .keys()
        .any(|k| k.starts_with("pass:")));
}
