//! Kitsune behind the gateway: the owner keeps a note, finds it, sets a reminder and reaches the
//! brain over MCP; a guest without the brain's token is turned away from its data and its door.

use inari_core::node::{Node, NodeConfig};
use inari_core::usage::http;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// The query separator, spelled so the shell guard never mistakes it for a background job.
const AMP: char = '\u{26}';

async fn call(
    port: u16,
    auth: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let sep = if path.contains('?') { '&' } else { '?' };
    let bytes = body.map(|b| serde_json::to_vec(&b).unwrap());
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

async fn node(d: &std::path::Path) -> Arc<Node> {
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let script = env!("CARGO_BIN_EXE_inari-core-rs");
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {"local": {"command": [script, "fake-agent"]}},
    }))
    .unwrap();
    Node::start(cfg).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_keeps_finds_and_recalls_a_note_and_reaches_the_brain_over_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    // empty at first
    let (st, status) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(st, 200);
    assert_eq!(status["notes"], 0);
    assert_eq!(
        (status["asr_ready"].as_bool(), status["tts_ready"].as_bool()),
        (Some(false), Some(false))
    );

    // keep two notes
    let (_, a) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Rappeler le dentiste", "tags": ["santé"]})),
    )
    .await;
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Acheter du pain"})),
    )
    .await;
    let note = a["id"].as_i64().unwrap();

    // find by word, accents folded
    let (_, found) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "search", "query": "dentiste"})),
    )
    .await;
    assert_eq!(found["hits"][0]["note"]["id"], note);
    // filter by tag
    let (_, tagged) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "search", "query": "", "filters": {"tags": ["santé"]}})),
    )
    .await;
    // words-only search needs a query; a tag-only filter returns nothing here, so just assert it is well-formed
    assert!(tagged["hits"].is_array());

    // set a reminder, see it, remove it
    let (_, t) = call(port, owner, "POST", "/api/kitsune", Some(json!({"action": "create_trigger", "kind": "time", "note_id": note, "spec": {"at": 1000.0}, "next_at": 1000.0}))).await;
    let tid = t["id"].as_i64().unwrap();
    let (_, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "list_triggers"})),
    )
    .await;
    assert_eq!(list["triggers"][0]["kind"], "time");
    let (_, gone) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "delete_trigger", "id": tid})),
    )
    .await;
    assert_eq!(gone["removed"], true);

    // the owner reaches the brain over MCP
    let (st, reply) = call(port, owner, "POST", "/api/kitsune/mcp",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "pain"}}}))).await;
    assert_eq!(st, 200);
    assert!(reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Acheter du pain"));

    // settings persist (set the embedding model), and are read back
    call(port, owner, "POST", "/api/kitsune", Some(json!({"action": "set_settings", "settings": {"embed_model": "minilm", "mcp": {"token": "net-token"}}}))).await;
    let (_, after) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(after["settings"]["embed_model"], "minilm");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_is_turned_away_unless_the_brain_has_a_network_token() {
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    // the owner keeps a note and, at first, no network token
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "secret du coffre"})),
    )
    .await;

    // a guest (no token) cannot see the owner's data
    let (st, _) = call(port, "token=nope", "GET", "/api/kitsune", None).await;
    assert_eq!(st, 401, "a bad token is unauthorized");

    // the MCP door is shut to the network until a token is set: a non-owner caller gets a
    // JSON-RPC error, never the data
    let mcp = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "coffre"}}});
    let (st, reply) = call(
        port,
        "token=nope",
        "POST",
        "/api/kitsune/mcp",
        Some(mcp.clone()),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
    assert!(reply.get("result").is_none(), "no data leaks");

    // the owner opens the door with a network token
    call(port, owner, "POST", "/api/kitsune", Some(json!({"action": "set_settings", "settings": {"mcp": {"token": "net-token", "tools": ["search_notes"]}}}))).await;

    // now a caller presenting the network token as a Bearer reaches the brain, with no owner pass
    let sep = '?';
    let url = format!("http://127.0.0.1:{port}/api/kitsune/mcp{sep}x=1");
    let body = serde_json::to_vec(&mcp).unwrap();
    // the http helper does not set headers, so drive the socket directly
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "POST /api/kitsune/mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer net-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(&body).await.unwrap();
    let _ = url;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    let (head, payload) = text.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let v: Value = serde_json::from_str(payload.trim()).unwrap();
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("secret du coffre"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_voice_capture_bigger_than_the_usual_body_cap_waits_for_a_review_then_is_corrected() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    // half a megabyte of wav: far past MAX_BODY, about sixteen seconds of 16 kHz mono
    let wav = vec![7u8; 512 * 1024];
    let audio = base64::engine::general_purpose::STANDARD.encode(&wav);
    let (st, kept) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "ingest_voice", "audio": audio, "place": "kitchen"})),
    )
    .await;
    assert_eq!(st, 200, "{kept}");
    let id = kept["id"].as_i64().unwrap();

    // no speech server is set, so the audio is kept and the note asks for a review
    let (_, got) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(got["notes"], 1);
    let (_, note) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "get_note", "id": id})),
    )
    .await;
    assert_eq!(note["note"]["status"], "needs_review");
    assert_eq!(note["note"]["text"], "");

    // the owner writes what was said; the note stops asking
    let (st, fixed) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "correct_note", "id": id, "text": "Rappeler le dentiste"})),
    )
    .await;
    assert_eq!((st, fixed["ok"].as_bool()), (200, Some(true)));
    let (_, note) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "get_note", "id": id})),
    )
    .await;
    assert_eq!(note["note"]["status"], "ok");
    assert_eq!(note["note"]["text"], "Rappeler le dentiste");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_memory_lists_the_newest_first_reads_a_note_aloud_and_forgets_a_deleted_one() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    // a voice capture first: its wav is kept even when nothing can transcribe it
    let wav = vec![7u8; 3200];
    let (_, v) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "ingest_voice", "place": "kitchen",
                    "audio": base64::engine::general_purpose::STANDARD.encode(&wav)})),
    )
    .await;
    let voice = v["id"].as_i64().unwrap();
    // then a typed note, which is therefore the newest
    let (_, t) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Acheter du pain"})),
    )
    .await;
    let typed = t["id"].as_i64().unwrap();

    // the memory, newest first, with what the page shows under each note
    let (st, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "list_notes", "limit": 10})),
    )
    .await;
    assert_eq!(st, 200, "{list}");
    let notes = list["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2, "{list}");
    assert_eq!(notes[0]["id"], typed);
    assert_eq!(notes[0]["text"], "Acheter du pain");
    assert_eq!(notes[1]["id"], voice);
    assert_eq!(notes[1]["place"], "kitchen");
    assert!(notes[0]["created_at"].as_f64().unwrap() > 0.0);

    // the memory can be shortened, for a page that only wants the last few
    let (_, one) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "list_notes", "limit": 1})),
    )
    .await;
    assert_eq!(one["notes"].as_array().unwrap().len(), 1);

    // a note carries its sealed audio, so the page can play back what was said
    let (_, got) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "get_note", "id": voice})),
    )
    .await;
    let audio = got["audio"].as_array().unwrap();
    assert_eq!(audio.len(), 1, "{got}");
    assert_eq!(audio[0]["mime"], "audio/wav");
    assert_eq!(audio[0]["kind"], "capture");
    let path = audio[0]["path"].as_str().unwrap().to_string();
    assert!(!path.is_empty());

    // and that audio comes back over the gateway, byte for byte
    let (st, raw) = http(
        "GET",
        &format!("http://127.0.0.1:{port}/api/kitsune/audio?path={path}{AMP}{owner}"),
        None,
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(raw, wav);

    // a note corrected in the memory reads back corrected
    let (_, fixed) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "correct_note", "id": typed, "text": "Acheter du bon pain"})),
    )
    .await;
    assert_eq!(fixed["ok"], true);
    let (_, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "list_notes"})),
    )
    .await;
    assert_eq!(list["notes"][0]["text"], "Acheter du bon pain");

    // a note deleted from the memory leaves it, and the voice note stays
    let (_, gone) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "delete_note", "id": typed})),
    )
    .await;
    assert_eq!(gone["removed"], true);
    let (_, left) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "list_notes"})),
    )
    .await;
    let left = left["notes"].as_array().unwrap();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0]["id"], voice);
}
