//! OpenFox driven natively, against a fake OpenFox (its REST API and WebSocket, as OpenFox 2.0
//! serves them): a turn streams into the journal, a confirmation reaches the person, a message
//! sent meanwhile goes to OpenFox's queue and lands where OpenFox takes it, a dropped WebSocket
//! comes back with the whole answer, an end OpenFox never said is found by asking, and the
//! settings and sign-in are the interface's.

use inari_core::agent::{Approver, Runner};
use inari_core::capsule::io;
use inari_core::host::CapsuleHost;
use inari_core::openfox::{OpenFoxHarness, Settings};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

// ── the fake OpenFox ─────────────────────────────────────────────────────────

#[derive(Default)]
struct Fox {
    running: bool,
    mode: String,
    provider: Option<(String, String)>,
    /// What OpenFox keeps of the conversation (GET /api/sessions/:id).
    messages: Vec<Value>,
    /// Messages posted while running (OpenFox's asap queue).
    queue: VecDeque<String>,
    /// The subscribed WebSocket's outbox.
    ws: Option<mpsc::UnboundedSender<Value>>,
    confirmed: Option<bool>,
    answered: Option<String>,
    /// The session's criteria as OpenFox keeps them (PUT /criteria).
    criteria: Vec<Value>,
    stops: usize,
    log: Vec<String>,
    /// What the REST calls and WebSocket messages carried, for the tests to read.
    seen: Vec<(String, Value)>,
}

type Shared = Arc<Mutex<Fox>>;

fn say(fox: &Shared, kind: &str, payload: Value) {
    let tx = fox.lock().unwrap().ws.clone();
    if let Some(tx) = tx {
        let _ = tx.send(json!({"type": kind, "sessionId": "s1", "payload": payload}));
    }
}

async fn pause(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// What OpenFox does with a message: scripted by its words.
async fn work(fox: Shared, content: String) {
    fox.lock().unwrap().running = true;
    say(&fox, "session.running", json!({"isRunning": true}));
    let mut answer = String::new();
    let mut piece = |fox: &Shared, text: &str| {
        answer.push_str(text);
        say(
            fox,
            "chat.delta",
            json!({"messageId": "m", "content": text}),
        );
    };
    if content.contains("tool") {
        say(
            &fox,
            "chat.tool_call",
            json!({"messageId": "m", "callId": "k1", "tool": "run_command", "args": {"command": "ls"}}),
        );
        say(
            &fox,
            "chat.tool_result",
            json!({"messageId": "m", "callId": "k1", "tool": "run_command", "result": {"success": true, "output": "a.txt"}}),
        );
        piece(&fox, "Listed.");
    } else if content.contains("confirm") {
        say(
            &fox,
            "chat.path_confirmation",
            json!({"callId": "c1", "tool": "write_file", "paths": ["/etc/x"], "reason": "outside_workdir"}),
        );
        let mut answered = None;
        for _ in 0..100 {
            answered = fox.lock().unwrap().confirmed;
            if answered.is_some() {
                break;
            }
            pause(50).await;
        }
        piece(
            &fox,
            if answered == Some(true) {
                "allowed"
            } else {
                "denied"
            },
        );
    } else if content.contains("slow") {
        piece(&fox, "Working");
        pause(1500).await; // the person sends another message meanwhile
        loop {
            let next = fox.lock().unwrap().queue.pop_front();
            let Some(next) = next else { break };
            say(
                &fox,
                "chat.message",
                json!({"message": {"id": "u2", "role": "user", "content": next}}),
            );
            say(
                &fox,
                "chat.done",
                json!({"messageId": "u2", "reason": "complete"}),
            );
            piece(&fox, &format!(" and: {next}"));
        }
    } else if content.contains("drop") {
        piece(&fox, "Part one");
        pause(100).await;
        fox.lock().unwrap().ws = None; // the WebSocket drops (its writer ends)
        pause(300).await;
        answer.push_str(", part two.");
        let mut f = fox.lock().unwrap();
        f.messages
            .push(json!({"role": "assistant", "content": answer}));
        f.running = false;
        return; // the end is said to nobody
    } else if content.contains("plan") {
        say(
            &fox,
            "chat.thinking",
            json!({"messageId": "m1", "content": "Let me think about "}),
        );
        say(
            &fox,
            "chat.thinking",
            json!({"messageId": "m1", "content": "the layout."}),
        );
        say(
            &fox,
            "mode.changed",
            json!({"mode": "builder", "auto": true}),
        );
        say(
            &fox,
            "chat.todo",
            json!({"todos": [{"content": "write the parser", "status": "in_progress"}, {"content": "test it", "status": "pending"}]}),
        );
        say(
            &fox,
            "metadata.updated",
            json!({"key": "criteria", "entries": [{"id": "c1", "description": "parses empty input", "status": "pending"}]}),
        );
        fox.lock().unwrap().criteria =
            vec![json!({"id": "c1", "description": "parses empty input", "status": "pending"})]; // OpenFox keeps what it says
        say(
            &fox,
            "chat.delta",
            json!({"messageId": "s", "content": "sub-agent notes", "subAgentType": "explorer"}),
        );
        say(
            &fox,
            "chat.done",
            json!({"messageId": "s", "reason": "complete", "agentType": "sub-agent"}),
        );
        say(
            &fox,
            "context.state",
            json!({"context": {"currentTokens": 1200, "maxTokens": 32000, "compactionCount": 1, "dangerZone": false, "canCompact": true}}),
        );
        piece(&fox, "Planned.");
    } else if content.contains("ask") {
        say(
            &fox,
            "chat.ask_user",
            json!({"callId": "q1", "question": "Which one?", "type": "choice",
            "options": [{"value": "a", "label": "Option A"}, {"value": "b", "label": "Option B"}]}),
        );
        let mut answer = None;
        for _ in 0..200 {
            answer = fox.lock().unwrap().answered.clone();
            if answer.is_some() {
                break;
            }
            pause(25).await;
        }
        piece(&fox, &format!("You chose {}.", answer.unwrap_or_default()));
    } else if content.contains("fail") {
        say(
            &fox,
            "chat.llm_retry",
            json!({"attempt": 1, "retryInMs": 2000, "error": "connection refused"}),
        );
        say(
            &fox,
            "chat.llm_retry_failed",
            json!({"attempts": 3, "error": "connection refused"}),
        );
    } else if content.contains("edit") {
        say(
            &fox,
            "chat.tool_call",
            json!({"messageId": "m", "callId": "e1", "tool": "edit_file",
            "args": {"path": "src/x.rs", "old_string": "let a = 1;\nlet b = 2;", "new_string": "let a = 10;"}}),
        );
        say(
            &fox,
            "chat.tool_result",
            json!({"messageId": "m", "callId": "e1", "tool": "edit_file", "result": {"success": true, "output": "Edited src/x.rs"}}),
        );
        say(
            &fox,
            "git.status",
            json!({"branch": "main", "diff": {"files": [{"path": "src/x.rs", "status": "modified", "additions": 1, "deletions": 2}]}}),
        );
        piece(&fox, "Edited.");
    } else if content.contains("retried") {
        piece(&fox, "Second try worked.");
    } else if content.contains("lost") {
        piece(&fox, "Quiet");
        pause(200).await;
        let mut f = fox.lock().unwrap();
        f.messages
            .push(json!({"role": "assistant", "content": "Quiet answer."}));
        f.running = false;
        return; // "not running" is never sent
    } else {
        piece(&fox, "Hello ");
        piece(&fox, "world");
    }
    {
        let mut f = fox.lock().unwrap();
        f.messages
            .push(json!({"role": "assistant", "content": answer}));
        f.running = false;
    }
    say(&fox, "session.running", json!({"isRunning": false}));
}

async fn read_head(s: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..end]).to_string();
            let mut body = raw[end + 4..].to_vec();
            let len = head
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while body.len() < len {
                let n = s.read(&mut buf).await.ok()?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..n]);
            }
            return Some((head, body));
        }
        let n = s.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
    }
}

fn server_frame(text: &str) -> Vec<u8> {
    let p = text.as_bytes();
    let mut out = vec![0x81];
    if p.len() < 126 {
        out.push(p.len() as u8);
    } else {
        out.push(126);
        out.extend_from_slice(&(p.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(p);
    out
}

/// One client frame's text (clients mask theirs).
async fn client_text(s: &mut tokio::net::tcp::OwnedReadHalf) -> Option<String> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).await.ok()?;
    let mut n = (h[1] & 0x7F) as usize;
    if n == 126 {
        let mut l = [0u8; 2];
        s.read_exact(&mut l).await.ok()?;
        n = u16::from_be_bytes(l) as usize;
    }
    let mut mask = [0u8; 4];
    s.read_exact(&mut mask).await.ok()?;
    let mut p = vec![0u8; n];
    s.read_exact(&mut p).await.ok()?;
    p.iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b ^= mask[i % 4]);
    Some(String::from_utf8_lossy(&p).into_owned())
}

async fn serve(fox: Shared, mut s: TcpStream) {
    let Some((head, body)) = read_head(&mut s).await else {
        return;
    };
    let first = head.lines().next().unwrap_or_default().to_string();
    let mut parts = first.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or("").to_string(),
        parts.next().unwrap_or("").to_string(),
    );
    let token_ok =
        head.to_lowercase().contains("x-session-token: tok") || target.contains("token=tok");
    fox.lock().unwrap().log.push(format!(
        "{method} {}",
        target.split('?').next().unwrap_or("")
    ));
    if head.to_lowercase().contains("upgrade: websocket") {
        if !token_ok {
            let _ = s
                .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
        let _ = s
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: x\r\n\r\n")
            .await;
        let (mut r, mut w) = s.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
        let writer = tokio::spawn(async move {
            while let Some(v) = rx.recv().await {
                if w.write_all(&server_frame(&v.to_string())).await.is_err() {
                    return;
                }
            }
            let _ = w.shutdown().await; // the outbox was dropped: the WebSocket drops
        });
        while let Some(text) = client_text(&mut r).await {
            let m: Value = serde_json::from_str(&text).unwrap_or_default();
            fox.lock().unwrap().seen.push((
                format!("WS {}", m["type"].as_str().unwrap_or("")),
                m.clone(),
            ));
            if m["type"] == "session.load" {
                fox.lock().unwrap().ws = Some(tx.clone());
                let _ =
                    tx.send(json!({"type": "ack", "id": m["id"], "payload": {"sessionId": "s1"}}));
            } else if m["type"] == "chat.retry" {
                tokio::spawn(work(Arc::clone(&fox), "retried".into()));
            }
        }
        drop(tx);
        let _ = writer.await;
        return;
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let path = target.split('?').next().unwrap_or("").to_string();
    fox.lock()
        .unwrap()
        .seen
        .push((format!("{method} {path}"), body.clone()));
    let (status, reply) = if path == "/api/auth" {
        (200, json!({"requiresAuth": true, "hasPassword": true}))
    } else if path == "/api/auth/login" {
        if body["password"] == "pw" {
            (200, json!({"token": "tok"}))
        } else {
            (401, json!({"error": "Invalid password"}))
        }
    } else if !token_ok {
        (401, json!({"error": "Unauthorized"}))
    } else if path == "/api/providers/test" {
        if body["url"] == "http://127.0.0.1:8000/v1" {
            (200, json!({"success": true, "model": "qwen3-coder"}))
        } else {
            (
                400,
                json!({"success": false, "error": "nothing answers there"}),
            )
        }
    } else if (path == "/api/providers" && method == "POST")
        || (path == "/api/providers/local" && method == "DELETE")
    {
        (200, json!({"success": true}))
    } else if path == "/api/providers/local/refresh" {
        (200, json!({"success": true, "models": []}))
    } else if path == "/api/providers" {
        (
            200,
            json!({"activeProviderId": "local", "providers": [
            {"id": "local", "name": "vLLM", "backend": "vllm", "isLocal": true, "apiKey": "sk-never-shown",
             "models": [{"id": "qwen3", "contextWindow": 32768}]}]}),
        )
    } else if path == "/api/agents" {
        (
            200,
            json!({"defaults": [{"id": "planner", "name": "Plan", "description": "reads", "subagent": false},
                                   {"id": "builder", "name": "Build", "description": "acts", "subagent": false},
                                   {"id": "explorer", "name": "Explorer", "description": "a sub-agent", "subagent": true}],
                     "userItems": [{"id": "reviewer", "name": "Reviewer", "description": "my own", "subagent": false}],
                     "projectItems": []}),
        )
    } else if path == "/api/sessions/s1/answer" {
        fox.lock().unwrap().answered = body["answer"].as_str().map(String::from);
        (200, json!({}))
    } else if path == "/api/sessions/s1/criteria" {
        let kept: Vec<Value> = body["criteria"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, c)| json!({"id": c["id"].as_str().map(String::from).unwrap_or(format!("n{i}")), "description": c["description"], "status": "pending"}))
            .collect();
        fox.lock().unwrap().criteria = kept;
        (200, json!({}))
    } else if [
        "/api/sessions/s1/criteria",
        "/api/sessions/s1/danger-level",
        "/api/sessions/s1/pause",
    ]
    .contains(&path.as_str())
    {
        (200, json!({}))
    } else if path == "/api/projects" && method == "GET" {
        (200, json!({"projects": []}))
    } else if path == "/api/projects" {
        (
            200,
            json!({"project": {"id": "p1", "workdir": body["workdir"]}}),
        )
    } else if path == "/api/sessions" {
        fox.lock().unwrap().mode = "planner".into();
        (200, json!({"session": {"id": "s1", "mode": "planner"}}))
    } else if path == "/api/sessions/s1" {
        let f = fox.lock().unwrap();
        let (pid, pm) = f.provider.clone().unzip();
        (
            200,
            json!({"session": {"id": "s1", "mode": f.mode, "isRunning": f.running, "providerId": pid, "providerModel": pm,
                               "metadataEntries": {"criteria": f.criteria}},
                     "sessionStats": {"totalTime": 12.5, "toolTime": 2.0, "prefillTokens": 900, "generationTokens": 120,
                                      "avgGenerationSpeed": 40.0, "responseCount": 2, "llmCallCount": 3},
                     "messages": f.messages}),
        )
    } else if path.starts_with("/api/sessions/") && !path.starts_with("/api/sessions/s1/") {
        (404, json!({"error": "Session not found"}))
    } else if path == "/api/sessions/s1/mode" {
        fox.lock().unwrap().mode = body["mode"].as_str().unwrap_or("").into();
        (200, json!({}))
    } else if path == "/api/sessions/s1/provider" {
        fox.lock().unwrap().provider = Some((
            body["providerId"].as_str().unwrap_or("").into(),
            body["model"].as_str().unwrap_or("").into(),
        ));
        (200, json!({}))
    } else if path == "/api/sessions/s1/confirm-path" {
        fox.lock().unwrap().confirmed = body["approved"].as_bool();
        (200, json!({}))
    } else if path == "/api/sessions/s1/stop" {
        fox.lock().unwrap().stops += 1;
        (200, json!({}))
    } else if path == "/api/sessions/s1/message" {
        let content = body["content"].as_str().unwrap_or("").to_string();
        let busy = {
            let mut f = fox.lock().unwrap();
            f.messages.push(json!({"role": "user", "content": content}));
            if f.running {
                f.queue.push_back(content.clone());
            }
            f.running
        };
        if !busy {
            tokio::spawn(work(Arc::clone(&fox), content));
        }
        (200, json!({"success": true}))
    } else {
        (404, json!({"error": "no route"}))
    };
    let text = reply.to_string();
    let _ = s
        .write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).as_bytes())
        .await;
}

async fn fake_openfox() -> (Shared, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fox: Shared = Arc::default();
    let f = Arc::clone(&fox);
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(serve(Arc::clone(&f), s));
        }
    });
    (fox, port)
}

// ── the lab ─────────────────────────────────────────────────────────────────

struct Lab {
    _dir: tempfile::TempDir,
    runner: Arc<Runner>,
    agent: String,
    fox: Shared,
    /// Every panel the harness sent to the pages, in order.
    panels: Arc<Mutex<Vec<Value>>>,
}

async fn lab(approve: bool) -> Lab {
    let (fox, port) = fake_openfox().await;
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("OPENFOX_ACP_DIR", dir.path().join("no-openfox-acp")); // no fallback token here
    let mut h = OpenFoxHarness::new(dir.path().join("openfox"));
    h.quiet = Duration::from_millis(800);
    h.set_settings(Settings {
        url: format!("http://127.0.0.1:{port}"),
        mode: "builder".into(),
        turn_timeout: 60,
    })
    .unwrap();
    assert!(
        h.login("wrong").await.is_err(),
        "a wrong password is refused"
    );
    h.login("pw").await.unwrap();
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let approver: Approver =
        Arc::new(move |_a: String, _c: String| Box::pin(async move { approve }));
    let panels: Arc<Mutex<Vec<Value>>> = Arc::default();
    let kept = Arc::clone(&panels);
    let publish: inari_core::agent::Publisher = Arc::new(move |subject: &str, payload: Value| {
        if subject.starts_with("panel.") {
            kept.lock().unwrap().push(payload["panel"].clone());
        }
    });
    let runner = Runner::new(
        host,
        vec![("openfox".to_string(), h)],
        Some(publish),
        Some(approver),
    );
    let agent = runner.create("t1", "", "a title").unwrap();
    Lab {
        _dir: dir,
        runner,
        agent,
        fox,
        panels,
    }
}

fn journal(l: &Lab) -> Vec<(String, Value)> {
    let c = io::load(&l.runner.host.agents.join(&l.agent)).unwrap();
    c.timeline
        .ordered()
        .iter()
        .map(|e| (e.kind.as_str().to_string(), e.data.clone()))
        .collect()
}

fn said(l: &Lab) -> Vec<String> {
    journal(l)
        .into_iter()
        .filter(|(k, _)| k == "user_message" || k == "assistant_message")
        .map(|(k, d)| format!("{}: {}", &k[..1], d["text"].as_str().unwrap_or("")))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_streams_into_the_journal_in_the_mode_the_settings_chose() {
    let l = lab(true).await;
    let out = l.runner.run(&l.agent, "hello", vec![]).await;
    assert_eq!(
        (out.stopped.as_str(), out.final_text.as_str()),
        ("done", "Hello world"),
        "{}",
        out.error
    );
    assert_eq!(said(&l), ["u: hello", "a: Hello world"]);
    assert_eq!(
        l.fox.lock().unwrap().mode,
        "builder",
        "OpenFox's planner start was switched to the settings' agent"
    );
    // the next turn reuses the session and its WebSocket: no new subscription
    let opened = |l: &Lab| {
        l.fox
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|e| e.starts_with("GET /") && !e.contains("/api"))
            .count()
    };
    let before = opened(&l);
    l.runner.run(&l.agent, "hello again", vec![]).await;
    assert_eq!(opened(&l), before, "the WebSocket was kept");
    let models = l.runner.harnesses["openfox"]
        .list_models(l._dir.path())
        .await;
    assert_eq!(models, ["local/qwen3"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_and_confirmations_go_through_inari() {
    let l = lab(false).await;
    l.runner.run(&l.agent, "use a tool", vec![]).await;
    let kinds: Vec<String> = journal(&l).into_iter().map(|(k, _)| k).collect();
    assert!(
        kinds.windows(2).any(|w| w == ["tool_call", "tool_result"]),
        "{kinds:?}"
    );
    let out = l.runner.run(&l.agent, "confirm this", vec![]).await;
    assert_eq!(
        out.final_text, "denied",
        "the person (here: the approver) said no"
    );
    assert_eq!(l.fox.lock().unwrap().confirmed, Some(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_sent_during_a_turn_goes_to_openfoxs_queue() {
    let l = lab(true).await;
    let (runner, agent) = (Arc::clone(&l.runner), l.agent.clone());
    let first = tokio::spawn(async move { runner.run(&agent, "something slow", vec![]).await });
    tokio::time::sleep(Duration::from_millis(600)).await;
    let joined = l.runner.run(&l.agent, "also this", vec![]).await;
    assert_eq!(
        joined.stopped, "joined",
        "posted into the running turn, not a turn of its own"
    );
    let first = first.await.unwrap();
    assert_eq!(first.stopped, "done", "{}", first.error);
    assert_eq!(
        said(&l),
        [
            "u: something slow",
            "a: Working",
            "u: also this",
            "a: and: also this"
        ]
    );
    assert_eq!(l.fox.lock().unwrap().stops, 0, "nothing was cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_websocket_comes_back_with_the_whole_answer() {
    let l = lab(true).await;
    let out = l.runner.run(&l.agent, "drop it", vec![]).await;
    assert_eq!(
        (out.stopped.as_str(), out.final_text.as_str()),
        ("done", "Part one, part two."),
        "{}",
        out.error
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_end_openfox_never_said_is_found_by_asking() {
    let l = lab(true).await;
    let started = std::time::Instant::now();
    let out = l.runner.run(&l.agent, "lost end", vec![]).await;
    assert_eq!(
        (out.stopped.as_str(), out.final_text.as_str()),
        ("done", "Quiet answer."),
        "{}",
        out.error
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "not the turn's limit: {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_status_shows_openfox_without_its_keys() {
    let l = lab(true).await;
    let h = l.runner.harnesses["openfox"].openfox().unwrap();
    let status = h.status().await;
    assert_eq!(status["reachable"], true);
    assert_eq!(status["active_provider"], "local");
    assert_eq!(status["providers"][0]["models"], json!(["qwen3"]));
    assert!(!status.to_string().contains("sk-never-shown"), "{status}");
    assert!(h
        .set_settings(Settings {
            url: "https://elsewhere".into(),
            ..h.settings()
        })
        .is_err());
}

fn last_panel(l: &Lab) -> Value {
    l.panels.lock().unwrap().last().cloned().unwrap_or_default()
}

fn fox_of(l: &Lab) -> &OpenFoxHarness {
    l.runner.harnesses["openfox"].openfox().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_panel_shows_what_openfox_does_beside_the_answer() {
    let l = lab(true).await;
    let out = l.runner.run(&l.agent, "make a plan", vec![]).await;
    assert_eq!(
        out.final_text, "Planned.",
        "a sub-agent's words stay out of the answer"
    );
    let p = last_panel(&l);
    assert_eq!(p["agent"], "builder");
    assert_eq!(
        p["todos"][0],
        json!({"id": "", "text": "write the parser", "status": "in_progress"})
    );
    assert_eq!(p["criteria"][0]["text"], "parses empty input");
    assert_eq!(
        (p["context"]["used"].as_u64(), p["context"]["max"].as_u64()),
        (Some(1200), Some(32000))
    );
    assert_eq!(p["subagents"][0]["text"], "sub-agent notes");
    assert_eq!(p["subagents"][0]["done"], true);
    assert_eq!(p["running"], false);
    let thought = l
        .panels
        .lock()
        .unwrap()
        .iter()
        .any(|x| x["thinking"] == "Let me think about the layout.");
    assert!(thought, "the thinking was shown while it came");
    assert!(
        !journal(&l)
            .iter()
            .any(|(_, d)| d.to_string().contains("think about")),
        "thinking is not journaled"
    );
    let agents: Vec<String> = p["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        agents,
        ["planner", "builder", "reviewer"],
        "OpenFox's own agents, sub-agents left out"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_choice_question_waits_for_the_page() {
    let l = lab(true).await;
    let (runner, agent) = (Arc::clone(&l.runner), l.agent.clone());
    let turn = tokio::spawn(async move { runner.run(&agent, "ask me", vec![]).await });
    let mut asked = Value::Null;
    for _ in 0..100 {
        asked = last_panel(&l)["question"].clone();
        if !asked.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(asked["options"][1]["label"], "Option B", "{asked}");
    fox_of(&l)
        .thread_action(
            &l.runner,
            &l.agent,
            "answer",
            &json!({"call_id": "q1", "answer": "b"}),
        )
        .await
        .unwrap();
    let out = turn.await.unwrap();
    assert_eq!(out.final_text, "You chose b.");
    assert!(
        last_panel(&l)["question"].is_null(),
        "answered: the question is gone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_model_is_retried_and_the_retry_followed() {
    let l = lab(true).await;
    l.runner.run(&l.agent, "this will fail", vec![]).await;
    assert_eq!(last_panel(&l)["retry"]["failed"], true);
    fox_of(&l)
        .thread_action(&l.runner, &l.agent, "retry", &json!({}))
        .await
        .unwrap();
    let mut got = Vec::new();
    for _ in 0..100 {
        got = said(&l);
        if got.last().is_some_and(|x| x.contains("Second try")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(
        got,
        ["u: this will fail", "a: Second try worked."],
        "followed, and no new message from the person"
    );
    assert!(l
        .fox
        .lock()
        .unwrap()
        .seen
        .iter()
        .any(|(k, _)| k == "WS chat.retry"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_panels_actions_reach_openfox() {
    let l = lab(true).await;
    l.runner.run(&l.agent, "hello", vec![]).await;
    let fox = fox_of(&l);
    let act = |a: &'static str, p: Value| {
        let l = &l;
        async move { fox.thread_action(&l.runner, &l.agent, a, &p).await }
    };
    act("agent", json!({"mode": "reviewer"})).await.unwrap();
    assert_eq!(last_panel(&l)["agent"], "reviewer");
    act(
        "criteria",
        json!({"criteria": [{"id": "c1", "text": "fast"}, {"text": "  "}, {"text": "safe"}]}),
    )
    .await
    .unwrap();
    // outside a turn, the panel shows the criteria as OpenFox now keeps them (read back)
    let texts: Vec<Value> = last_panel(&l)["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["text"].clone())
        .collect();
    assert_eq!(texts, [json!("fast"), json!("safe")]);
    act("danger", json!({"level": "dangerous"})).await.unwrap();
    act("effort", json!({"effort": "high"})).await.unwrap();
    act("compact", json!({})).await.unwrap();
    assert!(act("explode", json!({})).await.is_err());
    assert_eq!(
        last_panel(&l)["error"],
        "unknown OpenFox action \"explode\"",
        "the page is told"
    );
    let seen = l.fox.lock().unwrap().seen.clone();
    let body = |k: &str| {
        seen.iter()
            .rev()
            .find(|(x, _)| x == k)
            .map(|(_, b)| b.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        body("PUT /api/sessions/s1/mode"),
        json!({"mode": "reviewer"})
    );
    assert_eq!(
        body("PUT /api/sessions/s1/criteria"),
        json!({"criteria": [{"id": "c1", "description": "fast"}, {"description": "safe"}]})
    );
    assert_eq!(
        body("PUT /api/sessions/s1/danger-level"),
        json!({"dangerLevel": "dangerous"})
    );
    assert_eq!(
        body("POST /api/sessions/s1/provider")["reasoningEffort"],
        "high"
    );
    assert!(seen.iter().any(|(k, _)| k == "WS context.compact"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_openfox_switched_to_is_kept_on_the_next_turn() {
    let l = lab(true).await;
    l.runner.run(&l.agent, "hello", vec![]).await;
    assert_eq!(
        l.fox.lock().unwrap().mode,
        "builder",
        "a new session starts with the settings' agent"
    );
    l.fox.lock().unwrap().mode = "reviewer".into(); // OpenFox (or the person, in OpenFox) switched
    l.runner.run(&l.agent, "hello again", vec![]).await;
    assert_eq!(
        l.fox.lock().unwrap().mode,
        "reviewer",
        "not put back to the starting agent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_shows_its_diff_and_the_panel_its_git_and_totals() {
    let l = lab(true).await;
    l.runner.run(&l.agent, "edit the file", vec![]).await;
    let result = journal(&l)
        .into_iter()
        .find(|(k, _)| k == "tool_result")
        .map(|(_, d)| d["output"].as_str().unwrap_or("").to_string())
        .unwrap();
    assert_eq!(
        result,
        "Edited src/x.rs\n\n@@ src/x.rs\n- let a = 1;\n- let b = 2;\n+ let a = 10;"
    );
    let p = last_panel(&l);
    assert_eq!(p["git"]["branch"], "main");
    assert_eq!(p["git"]["files"][0]["deletions"], 2);
    assert_eq!(
        p["session_stats"]["answers"], 2,
        "the session's totals, read at the end of the turn"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_are_managed_and_their_keys_only_reach_openfox() {
    let l = lab(true).await;
    let fox = fox_of(&l);
    assert_eq!(
        fox.test_provider("http://127.0.0.1:8000/v1", "vllm")
            .await
            .unwrap(),
        "qwen3-coder"
    );
    assert!(fox
        .test_provider("http://127.0.0.1:9/v1", "vllm")
        .await
        .is_err());
    assert!(
        fox.add_provider(&json!({"name": "", "url": "x", "backend": "vllm"}))
            .await
            .is_err(),
        "a name is needed"
    );
    fox.add_provider(
        &json!({"name": "Kimi", "url": "https://api.moonshot.ai/v1", "backend": "openai",
                             "local": false, "api_key": "sk-kimi-secret"}),
    )
    .await
    .unwrap();
    fox.refresh_provider("local").await.unwrap();
    fox.remove_provider("local").await.unwrap();
    let seen = l.fox.lock().unwrap().seen.clone();
    let added = seen
        .iter()
        .find(|(k, _)| k == "POST /api/providers")
        .map(|(_, b)| b.clone())
        .unwrap();
    assert_eq!(
        added,
        json!({"name": "Kimi", "url": "https://api.moonshot.ai/v1", "backend": "openai", "isLocal": false, "apiKey": "sk-kimi-secret"})
    );
    assert!(seen.iter().any(|(k, _)| k == "DELETE /api/providers/local"));
    assert!(seen
        .iter()
        .any(|(k, _)| k == "POST /api/providers/local/refresh"));
    assert!(
        !fox.status().await.to_string().contains("sk-kimi-secret"),
        "never said back"
    );
}
