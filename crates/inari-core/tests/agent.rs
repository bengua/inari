//! The ACP harness against the scripted fake agent (`inari-core-rs fake-agent`).

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
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let mut h = AcpHarness::new(
        vec![script.to_string(), "fake-agent".into()],
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
    let agent = runner.create("t1", "", "a title").unwrap();
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
    assert!(l.runner.set_model(&l.agent, "fam/beta").unwrap());
    assert!(!l.runner.set_model(&l.agent, "fam/beta").unwrap());
    l.runner.run(&l.agent, "hi", vec![]).await;
    assert!(std::fs::read_to_string(&l.log)
        .unwrap()
        .contains(r#"{"set_model": "fam/beta"}"#));
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "provider_switch" && d["to"] == "fam/beta"));
    let models = l.runner.harnesses["fake"].list_models(l._dir.path()).await;
    assert_eq!(models, ["fam/alpha", "fam/beta"]);
}

#[tokio::test]
async fn the_chosen_model_is_made_ready_before_the_agent_runs() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("acp.log");
    let ready = dir.path().join("ready.log");
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let mut harness = fake(&log, &dir.path().join("sessions.json"), false);
    // records the model, and refuses one called fam/bad
    harness.prepare = vec![
        "/bin/sh".into(),
        "-c".into(),
        format!(
            "echo \"$1\" >> {}; [ \"$1\" != fam/bad ] || {{ echo no card free >&2; exit 3; }}",
            ready.display()
        ),
        "sh".into(),
        "{model}".into(),
    ];
    let runner = Runner::new(host, vec![("fake".into(), harness)], None, None);
    let agent = runner.create("t1", "", "a title").unwrap();
    let out = runner.run(&agent, "no model chosen", vec![]).await;
    assert_eq!(out.stopped, "done");
    assert!(!ready.exists(), "nothing to prepare without a chosen model");

    runner.set_model(&agent, "fam/beta").unwrap();
    let out = runner.run(&agent, "hi", vec![]).await;
    assert_eq!(out.stopped, "done", "{out:?}");
    assert_eq!(std::fs::read_to_string(&ready).unwrap(), "fam/beta\n");

    runner.set_model(&agent, "fam/bad").unwrap();
    let before = prompts(&log).len();
    let out = runner.run(&agent, "hello", vec![]).await;
    assert_eq!(out.stopped, "error");
    assert!(out.error.contains("no card free"), "{}", out.error);
    assert_eq!(prompts(&log).len(), before, "the agent was not asked");
    let c = io::load(&runner.host.agents.join(&agent)).unwrap();
    assert!(c
        .timeline
        .ordered()
        .iter()
        .any(|e| e.kind.as_str() == "checkpoint" && e.data["reason"] == "model_unavailable"));
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

#[tokio::test]
async fn a_typed_command_runs_scrubbed_is_journaled_and_kept() {
    let l = lab(false, None);
    std::env::set_var("INARI_TEST_SECRET", "leak");
    let out = l
        .runner
        .exec_user(
            &l.agent,
            "echo hi > out.txt; cat out.txt; echo ${INARI_TEST_SECRET:-clean}",
            std::time::Duration::from_secs(10),
        )
        .await;
    assert_eq!(out["output"], "hi\nclean\n", "{out}");
    assert_eq!(out["ok"], true);
    let kinds: Vec<String> = journal(&l).into_iter().map(|(k, _)| k).collect();
    assert_eq!(&kinds[1..], ["tool_call", "tool_result"]);
    let c = io::load(&l.runner.host.agents.join(&l.agent)).unwrap();
    assert!(c.snapshot.tree.contains_key("out.txt"));
    let started = std::time::Instant::now();
    let slow = l
        .runner
        .exec_user(
            &l.agent,
            "sleep 30 & sleep 30",
            std::time::Duration::from_millis(500),
        )
        .await;
    assert_eq!(slow["timed_out"], true);
    assert!(started.elapsed().as_secs() < 5);
}

#[tokio::test]
async fn the_harness_choice_is_journaled_and_checked() {
    let l = lab(false, None);
    assert!(l.runner.set_harness(&l.agent, "nope").is_err());
    assert!(
        !l.runner.set_harness(&l.agent, "fake").unwrap(),
        "already the harness"
    );
}

#[tokio::test]
async fn whatever_the_agent_started_ends_with_it() {
    let l = lab(false, None);
    l.runner.run(&l.agent, "orphan", vec![]).await;
    let pid: i32 = std::fs::read_to_string(l.log.with_extension("child"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let alive =
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "));
    assert!(!alive, "the agent's child {pid} outlived the turn");
}

#[test]
fn changes_made_at_once_to_a_capsule_are_all_kept() {
    let l = lab(false, None);
    let root = l.runner.host.agents.join(&l.agent);
    let before = journal(&l).len();
    std::thread::scope(|s| {
        for w in 0..6 {
            let root = &root;
            s.spawn(move || {
                for i in 0..15 {
                    io::append_event(
                        root,
                        "n1@t1",
                        inari_core::capsule::model::EventKind::Checkpoint,
                        serde_json::json!({"reason": format!("w{w}-{i}")}),
                        1.0,
                    )
                    .unwrap();
                }
            });
        }
        s.spawn(|| {
            for i in 0..15 {
                l.runner.set_model(&l.agent, &format!("fam/m{i}")).unwrap();
            }
        });
    });
    // 90 checkpoints and 15 model switches, none lost; the label is the last model chosen
    assert_eq!(journal(&l).len(), before + 90 + 15);
    let c = io::load(&root).unwrap();
    assert_eq!(c.manifest.labels()["model"], "fam/m14");
    assert!(io::verify(&root).is_empty(), "{:?}", io::verify(&root));
}

#[tokio::test]
async fn a_thread_under_a_pass_runs_in_a_home_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("acp.log");
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let mut harness = fake(&log, &dir.path().join("sessions.json"), false);
    harness
        .extra_env
        .insert("HOME".into(), "/owner/home".into());
    harness.pass_env.insert(
        "HOME".into(),
        format!("{}/pass-homes/{{pass}}", dir.path().display()),
    );
    let runner = Runner::new(host, vec![("fake".into(), harness)], None, None);
    let own = runner.create("own", "", "mine").unwrap();
    let out = runner.run(&own, "whereami", vec![]).await;
    assert_eq!(out.final_text, "fake reply to: /owner/home", "{out:?}");
    let guest = runner.create("guest", "", "theirs").unwrap();
    runner.label(&guest, "pass", "leo").unwrap();
    let out = runner.run(&guest, "whereami", vec![]).await;
    let want = format!("{}/pass-homes/leo", dir.path().display());
    assert_eq!(out.final_text, format!("fake reply to: {want}"), "{out:?}");
    assert!(std::path::Path::new(&want).is_dir(), "its home was made");
}

#[tokio::test]
async fn a_thread_deleted_during_its_turn_does_not_come_back() {
    let l = lab(false, None);
    let runner = Arc::clone(&l.runner);
    let agent = l.agent.clone();
    let turn = tokio::spawn(async move { runner.run(&agent, "slow", vec![]).await });
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    l.runner.host.discard(&l.agent).unwrap();
    let phase = |r: &Runner| r.phases.lock().unwrap().get(&l.agent).cloned();
    let before = phase(&l.runner);
    let _ = turn.await;
    assert_eq!(l.runner.host.status(&l.agent), "deleted");
    assert_eq!(
        phase(&l.runner),
        before,
        "the phase of a deleted thread stays as it was"
    );
    assert_ne!(phase(&l.runner).as_deref(), Some("idle"));
}

#[tokio::test]
async fn modes_and_commands_are_read_and_the_chosen_mode_reaches_the_agent() {
    let l = lab(false, None);
    let probe = l.runner.harnesses["fake"].probe(l._dir.path()).await;
    let ids: Vec<&str> = probe
        .modes
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert_eq!(ids, ["default", "plan"]);
    assert_eq!(probe.modes[1]["name"], "Plan");
    let names: Vec<&str> = probe
        .commands
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(names, ["summarize", "review"]);

    assert!(l.runner.set_mode(&l.agent, "plan").unwrap());
    assert!(!l.runner.set_mode(&l.agent, "plan").unwrap());
    l.runner.run(&l.agent, "hi", vec![]).await;
    assert!(std::fs::read_to_string(&l.log)
        .unwrap()
        .contains(r#"{"set_mode": "plan"}"#));
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "checkpoint" && d["reason"] == "mode_changed" && d["detail"] == "plan"));
    let reply = journal(&l)
        .into_iter()
        .filter(|(k, _)| k == "assistant_message")
        .map(|(_, d)| d["text"].as_str().unwrap_or("").to_string())
        .next()
        .unwrap_or_default();
    assert!(
        !reply.contains("MODE_UPDATE"),
        "the mode notice stays out of the answer: {reply}"
    );
    // a second turn does not ask again
    l.runner.run(&l.agent, "again", vec![]).await;
    let asked = std::fs::read_to_string(&l.log)
        .unwrap()
        .matches("set_mode")
        .count();
    assert_eq!(asked, 1, "the mode is set once per session");
}

#[tokio::test]
async fn stop_ends_a_running_turn_and_says_so() {
    let l = lab(false, None);
    let runner = Arc::clone(&l.runner);
    let agent = l.agent.clone();
    let turn = tokio::spawn(async move { runner.run(&agent, "slow", vec![]).await });
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let started = std::time::Instant::now();
    l.runner.cancel(&l.agent);
    let out = turn.await.unwrap();
    assert_eq!(out.stopped, "cancelled", "{out:?}");
    assert!(started.elapsed().as_secs() < 9, "it stopped at once");
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "checkpoint" && d["reason"] == "stopped"));
    // the thread is usable again, and a stop with nothing running does nothing
    l.runner.cancel(&l.agent);
    let out = l.runner.run(&l.agent, "hello", vec![]).await;
    assert_eq!(out.stopped, "done", "{out:?}");
}

#[tokio::test]
async fn the_context_use_an_agent_reports_is_kept_and_told() {
    let l = lab(false, None);
    l.runner.run(&l.agent, "hi", vec![]).await;
    let u = l
        .runner
        .usage
        .lock()
        .unwrap()
        .get(&l.agent)
        .cloned()
        .unwrap();
    assert_eq!(
        (u["used"].as_u64(), u["size"].as_u64()),
        (Some(1200), Some(8000))
    );
    assert_eq!(u["cost"], 0.02);
}

#[tokio::test]
async fn a_permission_request_with_a_string_id_reaches_the_approver() {
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&asked);
    let approver: Approver = Arc::new(move |_agent: String, what: String| {
        seen.lock().unwrap().push(what);
        Box::pin(async { true })
    });
    let l = lab(false, Some(approver));
    let out = l.runner.run(&l.agent, "permstr", vec![]).await;
    assert_eq!(out.stopped, "done", "{out:?}");
    assert_eq!(asked.lock().unwrap().len(), 1, "the approver was asked");
    assert!(
        out.final_text.contains("permission result: allow"),
        "{out:?}"
    );
}

fn counting_approver(allow: bool) -> (Approver, Arc<std::sync::atomic::AtomicUsize>) {
    let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = Arc::clone(&n);
    let approver: Approver = Arc::new(move |_agent: String, _what: String| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move { allow })
    });
    (approver, n)
}

#[tokio::test]
async fn auto_mode_lets_the_thread_act_without_asking_and_says_so() {
    let (approver, asked) = counting_approver(false); // asked, it would say no
    let l = lab(false, Some(approver));
    l.runner.label(&l.agent, "auto", "1").unwrap();
    let out = l.runner.run(&l.agent, "perm", vec![]).await;
    assert_eq!(out.stopped, "done", "{out:?}");
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nobody was asked"
    );
    assert!(
        out.final_text.contains("permission result: allow"),
        "{out:?}"
    );
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "checkpoint" && d["reason"] == "auto_approved"));
}

#[tokio::test]
async fn a_thread_under_a_pass_is_never_in_auto_mode() {
    let (approver, asked) = counting_approver(false);
    let l = lab(false, Some(approver));
    l.runner.label(&l.agent, "pass", "leo").unwrap();
    l.runner.label(&l.agent, "auto", "1").unwrap();
    let out = l.runner.run(&l.agent, "perm", vec![]).await;
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parent is asked"
    );
    assert!(
        out.final_text.contains("permission result: deny"),
        "{out:?}"
    );
}
