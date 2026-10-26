use inari_core::capsule::{io, EventKind, Timeline};
use inari_core::host::CapsuleHost;
use inari_core::rpc::{EndpointServer, RemoteEndpoint};
use inari_core::teleport::{teleport, CallError, CallResult, Endpoint};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn seed(dir: &Path, thread: &str) -> std::path::PathBuf {
    let root = dir.join(format!("seed-{thread}"));
    let mut t = Timeline::new();
    t.append(
        &format!("node-a@{thread}"),
        EventKind::UserMessage,
        json!({"text": "build it"}),
        1.0,
    );
    io::create(&root, thread, "c1", Some("node-a"), &t, 1.0).unwrap();
    inari_core::capsule::attach::attach(&root, "a.txt", b"alpha").unwrap();
    root
}

fn running(hosts: &[&CapsuleHost], agent: &str) -> Vec<String> {
    hosts
        .iter()
        .filter(|h| h.status(agent) == "running")
        .map(|h| h.node_id.clone())
        .collect()
}

#[tokio::test]
async fn the_agent_moves_and_only_one_node_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
    let b = CapsuleHost::new("node-b", &dir.path().join("b")).unwrap();
    let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
    let out = teleport(&a, &b, &agent, None).await;
    assert_eq!(out.state, "committed", "{out:?}");
    assert_eq!(running(&[&a, &b], &agent), vec!["node-b"]);
    assert!(io::verify(&b.agents.join(&agent)).is_empty());
    // back again: the blobs are already there
    let back = teleport(&b, &a, &agent, None).await;
    assert_eq!(back.state, "committed");
    assert_eq!(back.blobs_skipped, 1);
    assert_eq!(back.epoch, 3);
}

/// Fails one named call, before it runs (request lost) or after (reply lost).
struct Faulty<'a> {
    inner: &'a CapsuleHost,
    fail_on: &'static str,
    after: bool,
}

macro_rules! faulty {
    ($self:ident, $name:literal, $call:expr) => {{
        if $self.fail_on == $name && !$self.after {
            return Err(CallError::transport(concat!($name, ": request lost")));
        }
        let r = $call;
        if $self.fail_on == $name && $self.after {
            return Err(CallError::transport(concat!($name, ": reply lost")));
        }
        r
    }};
}

impl Endpoint for Faulty<'_> {
    fn node_id(&self) -> &str {
        self.inner.node_id()
    }
    async fn freeze(&self, a: &str) -> CallResult<(Value, u64)> {
        faulty!(self, "freeze", self.inner.freeze(a).await)
    }
    async fn read(&self, a: &str, r: &str) -> CallResult<Vec<u8>> {
        faulty!(self, "read", self.inner.read(a, r).await)
    }
    async fn unfreeze(&self, a: &str, e: u64) -> CallResult<()> {
        self.inner.unfreeze(a, e).await
    }
    async fn tombstone(&self, a: &str, e: u64) -> CallResult<()> {
        self.inner.tombstone(a, e).await
    }
    async fn offer(&self, s: &str, m: &Value, e: u64) -> CallResult<Vec<String>> {
        faulty!(self, "offer", self.inner.offer(s, m, e).await)
    }
    async fn receive(&self, s: &str, r: &str, d: &[u8]) -> CallResult<()> {
        faulty!(self, "receive", self.inner.receive(s, r, d).await)
    }
    async fn verify(&self, s: &str) -> CallResult<()> {
        faulty!(self, "verify", self.inner.verify(s).await)
    }
    async fn prepare(&self, s: &str) -> CallResult<()> {
        faulty!(self, "prepare", self.inner.prepare(s).await)
    }
    async fn activate(&self, s: &str, e: u64) -> CallResult<()> {
        faulty!(self, "activate", self.inner.activate(s, e).await)
    }
    async fn abort(&self, s: &str) -> CallResult<()> {
        self.inner.abort(s).await
    }
    async fn holder(&self, a: &str) -> CallResult<Option<(String, u64)>> {
        self.inner.holder(a).await
    }
}

#[tokio::test]
async fn a_failure_before_commit_resumes_the_source() {
    for step in ["offer", "receive", "verify", "prepare"] {
        let dir = tempfile::tempdir().unwrap();
        let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
        let b = CapsuleHost::new("node-b", &dir.path().join("b")).unwrap();
        let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
        let out = teleport(
            &a,
            &Faulty {
                inner: &b,
                fail_on: step,
                after: false,
            },
            &agent,
            None,
        )
        .await;
        assert_eq!(out.state, "rolled_back", "{step}: {out:?}");
        assert_eq!(running(&[&a, &b], &agent), vec!["node-a"], "{step}");
        assert_eq!(std::fs::read_dir(&b.staging).unwrap().count(), 0, "{step}");
    }
}

#[tokio::test]
async fn a_lost_reply_after_activation_is_settled_by_the_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
    let b = CapsuleHost::new("node-b", &dir.path().join("b")).unwrap();
    let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
    let out = teleport(
        &a,
        &Faulty {
            inner: &b,
            fail_on: "activate",
            after: true,
        },
        &agent,
        None,
    )
    .await;
    assert_eq!(out.state, "committed");
    assert_eq!(running(&[&a, &b], &agent), vec!["node-b"]);
}

#[tokio::test]
async fn a_core_that_cannot_host_the_agent_refuses_it_before_the_point_of_no_return() {
    let dir = tempfile::tempdir().unwrap();
    let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
    let mut b = CapsuleHost::new("node-b", &dir.path().join("b")).unwrap();
    b.on_prepare = Some(Box::new(
        |_| Err("this core has no 'claude' harness".into()),
    ));
    let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
    let out = teleport(&a, &b, &agent, None).await;
    assert_eq!(out.state, "rolled_back");
    assert!(out.error.contains("no 'claude' harness"));
    assert_eq!(a.status(&agent), "running");
}

#[tokio::test]
async fn teleport_works_over_tcp_and_refuses_the_wrong_key() {
    let dir = tempfile::tempdir().unwrap();
    let a = CapsuleHost::new("node-a", &dir.path().join("a")).unwrap();
    let b = Arc::new(CapsuleHost::new("node-b", &dir.path().join("b")).unwrap());
    let port = EndpointServer::new(Arc::clone(&b), b"mesh")
        .start("127.0.0.1", 0)
        .await
        .unwrap();
    let agent = a.adopt(&seed(dir.path(), "t1")).unwrap();
    let wrong = RemoteEndpoint::new(
        "node-b",
        &format!("127.0.0.1:{port}"),
        b"nope",
        Duration::from_secs(5),
    );
    assert_eq!(
        teleport(&a, &wrong, &agent, None).await.state,
        "rolled_back"
    );
    let right = RemoteEndpoint::new(
        "node-b",
        &format!("127.0.0.1:{port}"),
        b"mesh",
        Duration::from_secs(5),
    );
    let out = teleport(&a, &right, &agent, None).await;
    assert_eq!(out.state, "committed", "{out:?}");
    assert_eq!(b.status(&agent), "running");
}
