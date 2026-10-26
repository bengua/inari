use inari_core::bus::Bus;
use inari_core::buslink::{BusClient, BusServer};
use inari_core::capsule::fork::{fork, merge};
use inari_core::capsule::{io, EventKind, Timeline};
use inari_core::host::CapsuleHost;
use inari_core::teleport::clone_to;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn seed(dir: &Path, thread: &str) -> std::path::PathBuf {
    let root = dir.join(format!("seed-{thread}"));
    let mut t = Timeline::new();
    t.append(
        &format!("node-a@{thread}"),
        EventKind::UserMessage,
        json!({"text": "start"}),
        1.0,
    );
    t.append(
        &format!("node-a@{thread}"),
        EventKind::AssistantMessage,
        json!({"text": "ok"}),
        2.0,
    );
    io::create(&root, thread, "c1", Some("node-a"), &t, 1.0).unwrap();
    root
}

#[test]
fn a_fork_stops_at_the_chosen_event_and_two_forks_merge_back() {
    let dir = tempfile::tempdir().unwrap();
    let base = seed(dir.path(), "t1");
    let first = io::load(&base).unwrap().timeline.ordered()[0].id.clone();
    let child = fork(&base, &dir.path().join("child"), "t2", Some(&first)).unwrap();
    assert_eq!(child.timeline.len(), 1);
    assert_eq!(child.manifest.thread_id, "t2");
    assert_eq!(
        child.manifest.extra["lineage"]["fork_event_id"],
        json!(first)
    );
    assert!(child.manifest.extra["holder"].is_null());
    assert!(io::verify(&child.root).is_empty());

    let a = fork(&base, &dir.path().join("a"), "ta", None).unwrap();
    let b = fork(&base, &dir.path().join("b"), "tb", None).unwrap();
    io::append_event(
        &a.root,
        "n@ta",
        EventKind::Decision,
        json!({"text": "left"}),
        3.0,
    )
    .unwrap();
    io::append_event(
        &b.root,
        "n@tb",
        EventKind::Decision,
        json!({"text": "right"}),
        3.0,
    )
    .unwrap();
    assert_eq!(merge(&a.root, &b.root).unwrap(), 1);
    assert!(io::verify(&a.root).is_empty());
    assert_eq!(io::load(&a.root).unwrap().timeline.len(), 4);
}

#[tokio::test]
async fn a_clone_goes_elsewhere_with_another_harness_and_the_original_stays() {
    let dir = tempfile::tempdir().unwrap();
    let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
    let mut b = CapsuleHost::new("node-b", &dir.path().join("b")).unwrap();
    b.on_prepare = Some(Box::new(|c| {
        match c.manifest.labels().get("harness").and_then(|h| h.as_str()) {
            Some("claude") => Ok(()),
            other => Err(format!("this core has no {other:?} harness")),
        }
    }));
    let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
    // the original cannot go (no harness label the target accepts)…
    assert_eq!(
        inari_core::teleport::teleport(&a, &b, &agent, None)
            .await
            .state,
        "rolled_back"
    );
    // …a Claude clone can
    let out = clone_to(&a, &b, &agent, "t1-claude", Some("claude")).await;
    assert_eq!(out.state, "committed", "{out:?}");
    assert_eq!(a.status(&agent), "running");
    assert_eq!(b.status("t1-claude"), "running");
    let moved = io::load(&b.agents.join("t1-claude")).unwrap();
    assert_eq!(moved.manifest.labels()["harness"], json!("claude"));
    assert_eq!(
        moved.manifest.extra["lineage"]["parent_capsule"],
        json!("c1")
    );
    assert!(io::verify(&moved.root).is_empty());
}

async fn wait_for<F: Fn() -> bool>(f: F) -> bool {
    for _ in 0..100 {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    f()
}

#[tokio::test]
async fn buses_talk_over_tcp_and_durable_messages_wait_for_a_late_peer() {
    let home = Bus::new("home");
    let phone = Bus::new("phone");
    let server = BusServer::new(Arc::clone(&home), b"k", &["phone"]);
    let port = server.start("127.0.0.1", 0).await.unwrap();
    let mut at_home = home.subscribe("teleport.>");
    let mut at_phone = phone.subscribe("approval.>");
    // a durable question published before the phone ever connected
    home.publish("approval.request", json!({"q": 1}));
    let client = BusClient::start(Arc::clone(&phone), b"k", &format!("127.0.0.1:{port}"));
    assert!(wait_for(|| client.link.connected()).await);
    let got = tokio::time::timeout(Duration::from_secs(3), at_phone.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.payload["q"], 1);
    phone.publish("teleport.request", json!({"n": 1}));
    let got = tokio::time::timeout(Duration::from_secs(3), at_home.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.payload["n"], 1);
    // a wrong key never connects
    let evil = BusClient::start(Bus::new("evil"), b"wrong", &format!("127.0.0.1:{port}"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!evil.link.connected());
}
