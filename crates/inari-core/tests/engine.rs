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
    assert_eq!(
        names,
        ["read_file", "write_file", "edit_file", "run_command"]
    );
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
    assert_eq!(
        names,
        ["read_file", "run_command"],
        "the planner does not write"
    );
    assert!(asked[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("Plan Mode"));
    let refused = asked[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(refused.contains("not available in Plan mode"), "{refused}");
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
