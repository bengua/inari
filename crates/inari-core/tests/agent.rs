//! The ACP harness against the scripted fake agent shared with the Python tests.

use inari_core::agent::{AcpHarness, Approver, Runner};
use inari_core::capsule::attach::Attached;
use inari_core::capsule::io;
use inari_core::host::CapsuleHost;
use inari_core::teleport::Endpoint;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

struct Lab {
    _dir: tempfile::TempDir,
    log: PathBuf,
    runner: Arc<Runner>,
    agent: String,
}

fn fake(log: &Path, sessions: &Path, images: bool) -> AcpHarness {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake_acp.py");
    let mut h = AcpHarness::new(
        vec!["python3".into(), script.display().to_string()],
        sessions.to_path_buf(),
    );
    h.extra_env
        .insert("FAKE_ACP_LOG".into(), log.display().to_string());
    if images {
        h.extra_env.insert("FAKE_ACP_IMAGES".into(), "1".into());
    }
    h
}

fn lab(images: bool, approver: Option<Approver>) -> Lab {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("acp.log");
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let harness = fake(&log, &dir.path().join("sessions.json"), images);
    let runner = Runner::new(host, vec![("fake".into(), harness)], None, approver);
    let agent = runner.create("t1", "", "be useful").unwrap();
    Lab {
        _dir: dir,
        log,
        runner,
        agent,
    }
}

fn prompts(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v.get("prompt").is_some())
        .collect()
}

fn journal(l: &Lab) -> Vec<(String, Value)> {
    let c = io::load(&l.runner.host.agents.join(&l.agent)).unwrap();
    c.timeline
        .ordered()
        .iter()
        .map(|e| (e.kind.as_str().to_string(), e.data.clone()))
        .collect()
}

#[tokio::test]
async fn a_turn_journals_the_reply_and_tools_but_not_thoughts() {
    let l = lab(false, None);
    let out = l.runner.run(&l.agent, "please make a file", vec![]).await;
    assert_eq!(out.stopped, "done", "{out:?}");
    assert_eq!(out.final_text, "fake reply to: please make a file");
    let kinds: Vec<String> = journal(&l).into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        kinds,
        [
            "system",
            "user_message",
            "tool_call",
            "tool_result",
            "assistant_message"
        ]
    );
    let text = serde_json::to_string(&journal(&l)).unwrap();
    assert!(!text.contains("secret thoughts"));
    let c = io::load(&l.runner.host.agents.join(&l.agent)).unwrap();
    assert!(
        c.snapshot.tree.contains_key("made.txt"),
        "the workspace was not kept"
    );
    assert!(io::verify(&l.runner.host.agents.join(&l.agent)).is_empty());
}

#[tokio::test]
async fn an_image_goes_as_image_content_when_the_agent_accepts_it() {
    let l = lab(true, None);
    let a = l.runner.attach(&l.agent, "photo.png", PNG).await.unwrap();
    assert_eq!(a.mime, "image/png");
    l.runner.run(&l.agent, "what is this", vec![a]).await;
    let p = &prompts(&l.log)[0];
    assert_eq!(p["blocks"], serde_json::json!(["text", "image"]));
    assert!(p["prompt"]
        .as_str()
        .unwrap()
        .contains("photo.png (image/png"));
    let (_, user) = journal(&l)
        .into_iter()
        .find(|(k, _)| k == "user_message")
        .unwrap();
    assert_eq!(user["attachments"][0]["path"], "photo.png");
}

#[tokio::test]
async fn files_go_as_links_into_the_workspace() {
    let l = lab(false, None);
    let png = l.runner.attach(&l.agent, "photo.png", PNG).await.unwrap();
    let pdf = l
        .runner
        .attach(&l.agent, "docs/cv.pdf", b"%PDF-1.7 body")
        .await
        .unwrap();
    l.runner.run(&l.agent, "read these", vec![png, pdf]).await;
    let p = &prompts(&l.log)[0];
    // no image capability: the photo is a link too, and the agent finds both files
    assert_eq!(
        p["blocks"],
        serde_json::json!(["text", "resource_link", "resource_link"])
    );
    assert!(p["prompt"]
        .as_str()
        .unwrap()
        .contains("docs/cv.pdf (application/pdf"));
}

#[tokio::test]
async fn permissions_go_through_the_approver() {
    let yes: Approver = Arc::new(|_, cmd| Box::pin(async move { cmd == "bash: rm -rf x" }));
    let l = lab(false, Some(yes));
    assert_eq!(
        l.runner.run(&l.agent, "perm", vec![]).await.final_text,
        "permission result: allow"
    );
    let l = lab(false, None);
    assert_eq!(
        l.runner.run(&l.agent, "perm", vec![]).await.final_text,
        "permission result: deny"
    );
}

#[tokio::test]
async fn the_session_is_reused_and_a_new_one_is_primed() {
    let l = lab(false, None);
    l.runner.run(&l.agent, "first question", vec![]).await;
    l.runner.run(&l.agent, "second question", vec![]).await;
    let p = prompts(&l.log);
    assert_eq!(
        p[1]["prompt"], "second question",
        "the known session was not reused"
    );
    // another node (no session for this thread) primes the agent with the journal
    let sessions = l._dir.path().join("other.json");
    let mut other = fake(&l.log, &sessions, false);
    other.prime_chars = 1000;
    let runner = Runner::new(
        Arc::clone(&l.runner.host),
        vec![("fake".into(), other)],
        None,
        None,
    );
    runner.run(&l.agent, "third", vec![]).await;
    let primed = prompts(&l.log)[2]["prompt"].as_str().unwrap().to_string();
    assert!(primed.contains("continuing a conversation"));
    assert!(primed.contains("[user_message] first question"));
    assert!(primed.ends_with("third"));
}

#[tokio::test]
async fn the_model_choice_reaches_the_agent() {
    let l = lab(false, None);
    assert!(l
        .runner
        .set_choice(&l.agent, None, Some("fam/beta"))
        .unwrap());
    l.runner.run(&l.agent, "hi", vec![]).await;
    assert!(std::fs::read_to_string(&l.log)
        .unwrap()
        .contains(r#"{"set_model": "fam/beta"}"#));
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "provider_switch" && d["to"] == "fake:fam/beta"));
    let models = l.runner.harnesses["fake"].list_models(l._dir.path()).await;
    assert_eq!(models, ["fam/alpha", "fam/beta"]);
}

#[tokio::test]
async fn a_freeze_cancels_the_turn_and_a_crash_is_an_error() {
    let l = lab(false, None);
    let runner = Arc::clone(&l.runner);
    let agent = l.agent.clone();
    let turn = tokio::spawn(async move { runner.run(&agent, "slow", vec![]).await });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    l.runner.host.freeze(&l.agent).await.unwrap();
    let started = std::time::Instant::now();
    let out = turn.await.unwrap();
    assert_eq!(out.stopped, "frozen");
    assert!(started.elapsed().as_secs() < 9);
    let l = lab(false, None);
    let out = l.runner.run(&l.agent, "crash", vec![]).await;
    assert_eq!(out.stopped, "error");
}

#[test]
fn unused_attached_fields_stay_public() {
    let a = inari_core::agent::attached_from("a.txt", b"hello");
    assert_eq!(
        a,
        Attached {
            path: "a.txt".into(),
            sha256: inari_core::canon::sha256_hex(b"hello"),
            size: 5,
            mime: a.mime
        }
    );
}
