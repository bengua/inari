//! Inari's local engine against a model server that streams as vLLM and llama.cpp do (SSE over
//! chunked HTTP): the model writes a file through a tool, then answers; its reasoning is not
//! journaled; a mode keeps to its tools; a path outside the folder is the person's to allow; the
//! history goes back to the model; a long conversation is summarised; the models are listed.

use inari_core::agent::{Approver, Runner};
use inari_core::capsule::io;
use inari_core::engine::{LocalEngine, Provider, Settings};
use inari_core::host::CapsuleHost;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Default)]
struct Model {
    /// The streamed events of each next answer.
    script: VecDeque<Vec<Value>>,
    /// The requests it received.
    asked: Vec<Value>,
}

type Shared = Arc<Mutex<Model>>;

fn text(t: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"content": t}}]})
}

fn call(id: &str, name: &str, args: Value) -> Vec<Value> {
    let a = args.to_string();
    let (one, two) = a.split_at(a.len() / 2);
    vec![
        json!({"choices": [{"index": 0, "delta": {"reasoning_content": "I should use a tool."}}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": id, "type": "function", "function": {"name": name, "arguments": one}}]}}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": two}}]}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
    ]
}

fn answer(words: &[&str]) -> Vec<Value> {
    let mut out: Vec<Value> = words.iter().map(|w| text(w)).collect();
    out.push(json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 900, "completion_tokens": 20}}));
    out
}

async fn serve(model: Shared, mut s: tokio::net::TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let (head, body) = loop {
        let n = s.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).to_string();
            let len: usize = head
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap_or(0))
                })
                .unwrap_or(0);
            while buf.len() < i + 4 + len {
                let n = s.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            break (head, buf[i + 4..].to_vec());
        }
    };
    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
    if path.ends_with("/models") {
        let b = json!({"object": "list", "data": [{"id": "qwen3-coder"}, {"id": "gemma4"}]})
            .to_string();
        let _ = s
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len()).as_bytes())
            .await;
        return;
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let events = {
        let mut m = model.lock().unwrap();
        m.asked.push(request);
        m.script
            .pop_front()
            .unwrap_or_else(|| answer(&["(nothing scripted)"]))
    };
    let _ = s
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
        .await;
    for e in events {
        let piece = format!("data: {e}\n\n");
        // split each event across two chunks, as a busy server may
        let (a, b) = piece.as_bytes().split_at(piece.len() / 2);
        for part in [a, b] {
            let _ = s
                .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                .await;
            let _ = s.write_all(part).await;
            let _ = s.write_all(b"\r\n").await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let done = "data: [DONE]\n\n";
    let _ = s
        .write_all(format!("{:x}\r\n{done}\r\n0\r\n\r\n", done.len()).as_bytes())
        .await;
}

async fn model_server() -> (Shared, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let model: Shared = Arc::default();
    let m = Arc::clone(&model);
    tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            tokio::spawn(serve(Arc::clone(&m), s));
        }
    });
    (model, port)
}

struct Lab {
    _dir: tempfile::TempDir,
    runner: Arc<Runner>,
    agent: String,
    model: Shared,
    asked_to_allow: Arc<Mutex<Vec<String>>>,
}

async fn lab(window: u64) -> Lab {
    let (model, port) = model_server().await;
    let dir = tempfile::tempdir().unwrap();
    let engine = LocalEngine::new(dir.path().join("engine"));
    engine
        .set_settings(Settings {
            providers: vec![Provider {
                id: "gpu".into(),
                name: "vLLM".into(),
                url: format!("http://127.0.0.1:{port}/v1"),
            }],
            model: "gpu/qwen3-coder".into(),
            context_window: window,
            ..Settings::default()
        })
        .unwrap();
    let host = Arc::new(CapsuleHost::new("n1", &dir.path().join("node")).unwrap());
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = Arc::clone(&asked);
    let approver: Approver = Arc::new(move |_a: String, what: String| {
        seen.lock().unwrap().push(what);
        Box::pin(async { false })
    });
    let runner = Runner::new(
        host,
        vec![("local".to_string(), engine)],
        None,
        Some(approver),
    );
    let agent = runner.create("t1", "", "a thread").unwrap();
    Lab {
        _dir: dir,
        runner,
        agent,
        model,
        asked_to_allow: asked,
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

#[tokio::test(flavor = "multi_thread")]
async fn the_model_writes_a_file_through_a_tool_then_answers() {
    let l = lab(32_768).await;
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(call(
            "c1",
            "write_file",
            json!({"path": "hello.txt", "content": "hi there"}),
        ));
        m.script.push_back(answer(&["Wrote ", "hello.txt."]));
    }
    let out = l.runner.run(&l.agent, "write hello.txt", vec![]).await;
    assert_eq!(
        (out.stopped.as_str(), out.final_text.as_str()),
        ("done", "Wrote hello.txt."),
        "{out:?}"
    );
    let c = io::load(&l.runner.host.agents.join(&l.agent)).unwrap();
    assert!(
        c.snapshot.tree.contains_key("hello.txt"),
        "the file is the thread's"
    );
    let j = journal(&l);
    let kinds: Vec<&str> = j.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        &kinds[kinds.len() - 4..],
        [
            "user_message",
            "tool_call",
            "tool_result",
            "assistant_message"
        ]
    );
    let result = &j[j.len() - 2].1;
    assert!(
        result["output"].as_str().unwrap().contains("Wrote 8 bytes"),
        "{result}"
    );
    assert!(
        result["output"].as_str().unwrap().contains("+ hi there"),
        "the diff is shown"
    );
    assert!(
        !j.iter()
            .any(|(_, d)| d.to_string().contains("I should use a tool")),
        "reasoning is not journaled"
    );
    // what the model was sent: the system prompt, the mode's reminder with the message, its tools;
    // then its own call and the tool's result
    let asked = l.model.lock().unwrap().asked.clone();
    assert_eq!(asked[0]["model"], "qwen3-coder");
    assert_eq!(asked[0]["stream"], true);
    assert!(asked[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Working directory:"));
    let first = asked[0]["messages"][1]["content"].as_str().unwrap();
    assert!(
        first.contains("Build Mode") && first.ends_with("write hello.txt"),
        "{first}"
    );
    let names: Vec<&str> = asked[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    for t in [
        "read_file",
        "write_file",
        "edit_file",
        "run_command",
        "session_metadata",
        "call_sub_agent",
    ] {
        assert!(names.contains(&t), "{t} in {names:?}");
    }
    let second = asked[1]["messages"].as_array().unwrap();
    assert_eq!(second[2]["tool_calls"][0]["function"]["name"], "write_file");
    assert_eq!(second[3]["role"], "tool");
    assert_eq!(second[3]["tool_call_id"], "c1");

    // the next turn carries the history, and runs a command
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(call(
            "c2",
            "run_command",
            json!({"command": "cat hello.txt"}),
        ));
        m.script.push_back(answer(&["It says hi there."]));
    }
    l.runner.run(&l.agent, "what is in it?", vec![]).await;
    let asked = l.model.lock().unwrap().asked.clone();
    let third = asked[2]["messages"].as_array().unwrap();
    assert_eq!(third[1]["content"], "write hello.txt");
    assert!(third[2]["content"]
        .as_str()
        .unwrap()
        .contains("Wrote hello.txt."));
    let tool = &asked[3]["messages"].as_array().unwrap().last().unwrap()["content"];
    assert!(
        tool.as_str().unwrap().contains("exit code 0")
            && tool.as_str().unwrap().contains("hi there"),
        "{tool}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mode_keeps_to_its_tools_and_a_path_outside_is_the_persons() {
    let l = lab(32_768).await;
    l.runner.set_mode(&l.agent, "planner").unwrap();
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(call(
            "c1",
            "write_file",
            json!({"path": "x.txt", "content": "no"}),
        ));
        m.script
            .push_back(call("c2", "read_file", json!({"path": "/etc/hostname"})));
        m.script.push_back(answer(&["Fine."]));
    }
    l.runner.run(&l.agent, "plan it", vec![]).await;
    let asked = l.model.lock().unwrap().asked.clone();
    let names: Vec<&str> = asked[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"read_file")
            && !names.contains(&"write_file")
            && !names.contains(&"edit_file"),
        "the planner does not write: {names:?}"
    );
    assert!(asked[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Plan Mode"));
    let refused = asked[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(refused.contains("is not available to Planner"), "{refused}");
    let outside = asked[2]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(outside.contains("did not allow"), "{outside}");
    assert_eq!(
        l.asked_to_allow.lock().unwrap().clone(),
        ["read_file /etc/hostname"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_conversation_is_summarised_and_the_summary_kept() {
    let l = lab(2048).await;
    let long = "a long message ".repeat(400);
    l.model
        .lock()
        .unwrap()
        .script
        .push_back(answer(&["First answer."]));
    l.runner.run(&l.agent, &long, vec![]).await;
    {
        let mut m = l.model.lock().unwrap();
        m.script
            .push_back(answer(&["The person sent a long message; I answered."]));
        m.script.push_back(answer(&["Second answer."]));
    }
    let out = l.runner.run(&l.agent, "and now?", vec![]).await;
    assert_eq!(out.final_text, "Second answer.");
    let asked = l.model.lock().unwrap().asked.clone();
    assert!(asked[1]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("summarizing conversations"));
    assert!(asked[1].get("tools").is_none());
    let after = asked[2]["messages"].as_array().unwrap();
    assert!(after[1]["content"]
        .as_str()
        .unwrap()
        .contains("The person sent a long message"));
    assert!(
        !asked[2].to_string().contains(&long),
        "the long message is in the summary only"
    );
    assert!(journal(&l)
        .iter()
        .any(|(k, d)| k == "checkpoint" && d["reason"] == "compacted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_models_are_listed_and_a_missing_model_is_said() {
    let l = lab(32_768).await;
    let h = l.runner.harness("local").unwrap();
    let probe = h.probe(l._dir.path()).await;
    assert_eq!(probe.models, ["gpu/qwen3-coder", "gpu/gemma4"]);
    let engine = h.local().unwrap();
    let status = engine.status().await;
    assert_eq!(status["providers"][0]["reachable"], true);
    assert_eq!(status["providers"][0]["key"], false);
    engine.set_key("gpu", "sk-local").unwrap();
    assert!(
        !engine.status().await.to_string().contains("sk-local"),
        "a key is never shown"
    );
    let mut s = engine.settings();
    s.model = String::new();
    engine.set_settings(s).unwrap();
    let out = l.runner.run(&l.agent, "hello", vec![]).await;
    assert_eq!(out.stopped, "error");
    assert!(out.error.contains("no model chosen"), "{}", out.error);
}

fn engine_of(l: &Lab) -> Arc<inari_core::agent::Harness> {
    l.runner.harness("local").unwrap()
}

fn state_of(l: &Lab) -> inari_core::engine::state::ThreadState {
    inari_core::engine::state::ThreadState::load(&l.runner.host.agents.join(&l.agent))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workflow_walks_its_steps_and_waits_for_the_person() {
    let l = lab(32_768).await;
    let wf = json!({"metadata": {"id": "sandbox", "name": "Sandbox"}, "entryStep": "greet",
    "settings": {"maxIterations": 20},
    "steps": [
        {"id": "greet", "name": "Say Hi", "type": "agent", "phase": "build", "agentId": "builder",
         "prompt": "Say hi about {{topic}}.", "transitions": [{"when": {"type": "always"}, "goto": "pick"}]},
        {"id": "pick", "name": "Pick a Mood", "type": "user", "phase": "build", "transitions": [
            {"when": {"type": "step_result", "result": "happy"}, "goto": "happy"},
            {"when": {"type": "always"}, "goto": "check"}]},
        {"id": "happy", "name": "Happy", "type": "agent", "phase": "build",
         "prompt": "Be happy.", "transitions": [{"when": {"type": "always"}, "goto": "check"}]},
        {"id": "check", "name": "Check", "type": "shell", "phase": "verification", "command": "echo checked {{topic}}",
         "transitions": [{"when": {"type": "step_result", "result": "success"}, "goto": "$done"},
                         {"when": {"type": "always"}, "goto": "$blocked"}]}
    ]});
    let dir = l._dir.path().join("engine").join("workflows");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sandbox.workflow.json"), wf.to_string()).unwrap();
    {
        let mut m = l.model.lock().unwrap();
        // the first agent step answers without step_done: it is nudged, then done
        m.script.push_back(answer(&["Hi!"]));
        m.script.push_back(call("s1", "step_done", json!({})));
        m.script.push_back(answer(&["Done."]));
    }
    let launch = inari_core::openfox::Launch {
        id: "sandbox".into(),
        params: [("topic".to_string(), "foxes".to_string())].into(),
        ..Default::default()
    };
    l.runner.launch(&l.agent, "", vec![], launch).await;
    let flow = state_of(&l).flow.unwrap();
    assert_eq!(
        (flow.status.as_str(), flow.step.as_str()),
        ("waiting", "pick")
    );
    assert_eq!(
        flow.choices
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["happy", "continue"]
    );
    let asked = l.model.lock().unwrap().asked.clone();
    let first = asked[0]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        first.contains("Say hi about foxes.") && first.contains("call step_done()"),
        "{first}"
    );
    assert!(asked[0]["tools"].to_string().contains("step_done"));
    let nudge = asked[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        nudge.ends_with("If you have finished the task, call step_done()"),
        "{nudge}"
    );

    // the person picks "happy": that branch, then the shell step, then done
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(call("s2", "step_done", json!({})));
        m.script.push_back(answer(&["Yay."]));
    }
    let resume = inari_core::openfox::Launch {
        resume: true,
        choice: Some("happy".into()),
        label: "happy".into(),
        ..Default::default()
    };
    l.runner.launch(&l.agent, "", vec![], resume).await;
    let flow = state_of(&l).flow.unwrap();
    assert_eq!(flow.status, "completed", "{flow:?}");
    assert_eq!(flow.results["pick"], "happy");
    assert_eq!(flow.results["check"], "success");
    assert!(flow.output["stdout"].contains("checked foxes"));
    let reasons: Vec<String> = journal(&l)
        .iter()
        .filter(|(k, _)| k == "checkpoint")
        .filter_map(|(_, d)| d["reason"].as_str().map(String::from))
        .collect();
    assert!(
        reasons.contains(&"workflow_waiting".to_string())
            && reasons.contains(&"workflow_done".to_string()),
        "{reasons:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sub_agent_works_apart_and_returns_its_value() {
    let l = lab(32_768).await;
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(call(
            "c1",
            "call_sub_agent",
            json!({"subAgentType": "explorer", "prompt": "find the parser"}),
        ));
        // the explorer's own run: its return value
        m.script.push_back(call(
            "r1",
            "return_value",
            json!({"content": "it is in src/parse.rs"}),
        ));
        m.script.push_back(answer(&["Found it."]));
    }
    let out = l.runner.run(&l.agent, "where is the parser?", vec![]).await;
    assert_eq!(out.final_text, "Found it.");
    let asked = l.model.lock().unwrap().asked.clone();
    let sub = asked[1]["messages"].as_array().unwrap();
    assert_eq!(
        sub.len(),
        2,
        "a fresh context: its system prompt and its task"
    );
    assert!(sub[1]["content"]
        .as_str()
        .unwrap()
        .contains("find the parser"));
    assert!(asked[1]["tools"].to_string().contains("return_value"));
    let back = asked[2]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(back, "it is in src/parse.rs");
}

/// An MCP server over HTTP, as the streamable transport answers: one JSON answer a request.
async fn mcp_server(seen: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..i]).to_string();
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        while buf.len() < i + 4 + len {
                            let n = s.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        break (head, buf[i + 4..].to_vec());
                    }
                };
                seen.lock().unwrap().push(head);
                let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let result = match req["method"].as_str().unwrap_or("") {
                    "initialize" => {
                        json!({"protocolVersion": "2025-03-26", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake"}})
                    }
                    "tools/list" => {
                        json!({"tools": [{"name": "echo", "description": "Says it back", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]})
                    }
                    "tools/call" => {
                        json!({"content": [{"type": "text", "text": format!("echo: {}", req["params"]["arguments"]["text"].as_str().unwrap_or(""))}]})
                    }
                    _ => {
                        let _ = s
                            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        return;
                    }
                };
                let b = json!({"jsonrpc": "2.0", "id": req["id"], "result": result}).to_string();
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: sess-1\r\nContent-Length: {}\r\n\r\n{b}", b.len()).as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_servers_tools_are_the_agents_and_its_secret_stays() {
    let l = lab(32_768).await;
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let port = mcp_server(Arc::clone(&seen)).await;
    let h = engine_of(&l);
    let engine = h.local().unwrap();
    engine
        .change("mcp_add", &json!({"name": "fake", "transport": "http", "url": format!("http://127.0.0.1:{port}/mcp"), "headers": "X-Token: s3cret"}))
        .await
        .unwrap();
    let view = engine.library("mcp", "").await.unwrap();
    assert_eq!(view["servers"][0]["status"], "connected", "{view}");
    assert_eq!(view["servers"][0]["headers"], json!(["X-Token"]));
    assert!(!view.to_string().contains("s3cret"));
    {
        let mut m = l.model.lock().unwrap();
        m.script
            .push_back(call("m1", "fake_echo", json!({"text": "hi"})));
        m.script.push_back(answer(&["It said hi."]));
    }
    l.runner.run(&l.agent, "use the echo", vec![]).await;
    let asked = l.model.lock().unwrap().asked.clone();
    assert!(asked[0]["tools"].to_string().contains("fake_echo"));
    let back = asked[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(back, "echo: hi");
    let heads = seen.lock().unwrap().clone();
    assert!(heads.iter().all(|h| h.contains("X-Token: s3cret")));
    assert!(
        heads.last().unwrap().contains("Mcp-Session-Id: sess-1"),
        "the session goes back"
    );
    // turned off, its tools are not offered
    engine
        .change("mcp_toggle", &json!({"name": "fake", "disabled": true}))
        .await
        .unwrap();
    {
        let mut m = l.model.lock().unwrap();
        m.script.push_back(answer(&["ok"]));
    }
    l.runner.run(&l.agent, "again", vec![]).await;
    let asked = l.model.lock().unwrap().asked.clone();
    assert!(!asked[2]["tools"].to_string().contains("fake_echo"));
}
