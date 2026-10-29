//! A supervision pass end to end: the owner creates it, the guest gets a thread of its
//! own under its allowed harness, sees nothing else and cannot reach the owner's tools;
//! the owner follows the thread and answers the agent's permission request; a revoked
//! pass opens nothing.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
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
async fn a_supervised_guest_has_its_threads_and_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let h = |log: &str| json!({"command": [script, "fake-agent"], "env": {"FAKE_ACP_LOG": d.join(log)}});
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1, "approval_ttl": 10.0,
        "harnesses": {"local": h("local.log"), "frontier": h("frontier.log")},
    }))
    .unwrap();
    let home = Node::start(cfg).await.unwrap();
    let g = home.gateway.clone().unwrap();
    let port = home.gateway_port;
    let owner = "token=owner";

    let (_, made) = call(
        port,
        owner,
        "/api/pass",
        Some(json!({"name": "Léo", "role": "supervision", "harnesses": ["local"]})),
    )
    .await;
    let (pid, token) = (
        made["id"].as_str().unwrap().to_string(),
        made["token"].as_str().unwrap().to_string(),
    );
    let guest = format!("pass={token}");
    until("the pass", || {
        g.snapshot_for(&inari_core::gateway::Access::Owner)["passes"]
            .as_array()
            .is_some_and(|p| p.len() == 1)
    })
    .await;
    assert!(
        g.snapshot_for(&inari_core::gateway::Access::Owner)["passes"][0]
            .get("token_sha")
            .is_none(),
        "never the hash"
    );
    let (status, me) = call(port, &guest, "/api/whoami", None).await;
    assert_eq!((status, me["role"].as_str()), (200, Some("supervision")));

    // the guest's thread: named after the pass, on an allowed harness only
    let (status, _) = call(
        port,
        &guest,
        "/api/agent",
        Some(json!({"thread_id": "maths", "node": "home", "harness": "frontier"})),
    )
    .await;
    assert_eq!(status, 403, "a harness the pass does not allow");
    call(
        port,
        &guest,
        "/api/agent",
        Some(json!({"thread_id": "maths", "node": "home"})),
    )
    .await;
    let thread = format!("{pid}-maths");
    call(
        port,
        owner,
        "/api/agent",
        Some(json!({"thread_id": "taxes", "node": "home"})),
    )
    .await;
    until("both threads", || {
        g.snapshot()["agents"]
            .as_object()
            .is_some_and(|a| a.len() == 2)
    })
    .await;
    let guest_view = g.snapshot_for(&inari_core::gateway::Access::Guest(
        json!({"id": pid, "harnesses": ["local"], "role": "supervision", "name": "Léo"}),
    ));
    let seen: Vec<&String> = guest_view["agents"].as_object().unwrap().keys().collect();
    assert_eq!(seen, [&thread], "the guest sees its own thread only");
    assert_eq!(
        g.snapshot()["agents"][&thread]["pass"],
        pid.as_str(),
        "the owner sees whose thread it is"
    );
    let harnesses: Vec<&String> = guest_view["nodes"][0]["capabilities"]["harnesses"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(harnesses, ["local"]);
    assert_eq!(
        guest_view["ui"]["pass"]["role"], "supervision",
        "the page shows the guest it is supervised"
    );

    // nothing of the owner's
    assert_eq!(
        call(port, &guest, "/api/agent/taxes/timeline", None)
            .await
            .0,
        403
    );
    assert_eq!(
        call(
            port,
            &guest,
            "/api/exec",
            Some(json!({"agent": thread, "cmd": "id"}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            port,
            &guest,
            "/api/teleport",
            Some(json!({"agent": thread, "source": "home", "target": "x"}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            port,
            &guest,
            "/api/pass",
            Some(json!({"name": "x", "role": "collaboration"}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            port,
            &guest,
            "/api/message",
            Some(json!({"agent": "taxes", "text": "hi"}))
        )
        .await
        .0,
        403
    );
    // on its own thread too, what runs outside the supervision's sight is the owner's: an OpenFox
    // workflow, a task, a dev server, the project's instructions, the library
    assert_eq!(
        call(
            port,
            &guest,
            "/api/message",
            Some(json!({"agent": thread, "text": "", "workflow": {"id": "review"}}))
        )
        .await
        .0,
        403
    );
    for action in ["task_add", "dev_config", "dev_start", "instructions"] {
        assert_eq!(
            call(
                port,
                &guest,
                "/api/openfox/thread",
                Some(json!({"agent": thread, "action": action, "prompt": "x"}))
            )
            .await
            .0,
            403,
            "{action}"
        );
    }
    assert_eq!(
        call(
            port,
            &guest,
            "/api/openfox/library?node=home&harness=openfox&kind=mcp",
            None
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(port, &format!("pass=wrong&{owner}"), "/api/whoami", None)
            .await
            .0,
        401,
        "a pass never falls back to the owner"
    );

    // the agent asks; under supervision the owner answers, not the guest
    let mut done = home.bus.subscribe(&format!("agent.{thread}.done"));
    assert_eq!(
        call(
            port,
            &guest,
            "/api/message",
            Some(json!({"agent": thread, "text": "perm"}))
        )
        .await
        .0,
        200
    );
    until("the question", || {
        g.snapshot()["approvals"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    })
    .await;
    let q = g.snapshot()["approvals"][0]["request_id"].clone();
    assert_eq!(
        call(
            port,
            &guest,
            "/api/approval",
            Some(json!({"request_id": q, "approved": true}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            port,
            owner,
            "/api/approval",
            Some(json!({"request_id": q, "approved": true}))
        )
        .await
        .0,
        200
    );
    let out = tokio::time::timeout(Duration::from_secs(15), done.recv())
        .await
        .unwrap()
        .unwrap()
        .payload;
    assert_eq!(out["text"], "permission result: allow");
    let (status, timeline) =
        call(port, owner, &format!("/api/agent/{thread}/timeline"), None).await;
    assert_eq!(status, 200);
    assert!(
        timeline["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "user_message"),
        "the owner follows the thread"
    );

    call(port, owner, "/api/pass/revoke", Some(json!({"id": pid}))).await;
    until("the revocation", || {
        g.snapshot_for(&inari_core::gateway::Access::Owner)["passes"]
            .as_array()
            .is_some_and(|p| p.is_empty())
    })
    .await;
    assert_eq!(call(port, &guest, "/api/whoami", None).await.0, 401);
}
