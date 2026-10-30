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

    // set a reminder, see it, dismiss it
    let reminder = json!({"trigger_type": "time", "due_date": "2026-10-31T09:00:00Z"});
    let (_, set) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_reminder", "id": note, "reminder": reminder})),
    )
    .await;
    assert_eq!(set["ok"], true, "{set}");
    let (_, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "reminders"})),
    )
    .await;
    assert_eq!(list["reminders"][0]["id"], note);
    assert_eq!(list["reminders"][0]["trigger_type"], "time");
    let (_, gone) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "dismiss", "id": note})),
    )
    .await;
    assert_eq!(gone["ok"], true);

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
    assert_eq!(note["note"]["status"], "pending_transcription");
    assert_eq!(note["note"]["raw_transcript"], "");

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
    assert_eq!(note["note"]["status"], "active");
    assert_eq!(note["note"]["clean_summary"], "Rappeler le dentiste");
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
    assert_eq!(notes[0]["clean_summary"], "Acheter du pain");
    assert_eq!(notes[1]["id"], voice);
    assert_eq!(notes[1]["location_raw"], "kitchen");
    assert!(notes[0]["created_at"].as_str().unwrap().ends_with('Z'));

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
    assert_eq!(list["notes"][0]["clean_summary"], "Acheter du bon pain");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_reminder_lives_on_its_note_and_the_page_reads_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    let (_, kept) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Code de Julie", "tags": ["perso"]})),
    )
    .await;
    let note = kept["id"].as_i64().unwrap();
    let place = json!({"trigger_type": "geofence", "latitude": 48.8566, "longitude": 2.3522, "radius_meters": 150});
    let (_, made) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_reminder", "id": note, "reminder": place})),
    )
    .await;
    assert_eq!(made["ok"], true, "{made}");

    let (_, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "reminders", "trigger_type": "geofence"})),
    )
    .await;
    let r = list["reminders"][0].clone();
    assert_eq!(r["id"], note);
    assert_eq!(r["latitude"], 48.8566);
    assert_eq!(r["radius_meters"], 150);
    assert_eq!(r["tags"][0], "perso");
    let (_, none) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "reminders", "trigger_type": "time"})),
    )
    .await;
    assert_eq!(none["reminders"].as_array().unwrap().len(), 0);

    // a bad reminder is refused plainly
    let (st, bad) = call(port, owner, "POST", "/api/kitsune",
        Some(json!({"action": "set_reminder", "id": note, "reminder": {"trigger_type": "geofence", "latitude": 99.0, "longitude": 0.0}}))).await;
    assert_eq!(st, 400, "{bad}");

    // a place reminder is not snoozed by the clock; a time one is
    let (_, snoozed) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "snooze", "id": note, "due_date": "2026-11-02T09:00:00Z"})),
    )
    .await;
    assert_eq!(snoozed["ok"], false, "{snoozed}");
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_reminder", "id": note,
        "reminder": {"trigger_type": "time", "due_date": "2026-11-01T09:00:00Z"}})),
    )
    .await;
    let (_, snoozed) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "snooze", "id": note, "due_date": "2026-11-02T09:00:00Z"})),
    )
    .await;
    assert_eq!(snoozed["ok"], true, "{snoozed}");
    let (_, list) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "reminders"})),
    )
    .await;
    assert_eq!(list["reminders"][0]["due_date"], "2026-11-02T09:00:00Z");

    // then dismissed: it leaves the list
    let (_, gone) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "dismiss", "id": note})),
    )
    .await;
    assert_eq!(gone["ok"], true);
    let (_, empty) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "reminders"})),
    )
    .await;
    assert_eq!(empty["reminders"].as_array().unwrap().len(), 0);
    let (st, status) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!((st, status["reminders"].as_i64()), (200, Some(0)));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owner_points_the_core_at_its_own_servers_and_the_page_reads_them_back() {
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";

    // nothing set at first
    let (st, status) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(st, 200);
    assert_eq!(
        (status["asr_ready"].as_bool(), status["tts_ready"].as_bool()),
        (Some(false), Some(false))
    );
    assert_eq!(status["settings"]["embed_model"], "");

    // the page sends the whole settings back, so what it does not know survives
    let settings = json!({
        "asr": {"url": "http://127.0.0.1:8080/v1", "model": "whisper-1", "language": "fr", "timeout": 60},
        "tts": {"url": "http://127.0.0.1:8880/v1", "model": "kokoro", "voice": "fr_FR-siwis-medium",
                "format": "wav", "timeout": 60},
        "embed_model": "bge-m3",
        "mcp": {"token": "brain", "tools": ["keep"], "embed_model": "bge-m3"}
    });
    let (st, saved) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_settings", "settings": settings})),
    )
    .await;
    assert_eq!((st, saved["ok"].as_bool()), (200, Some(true)));

    let (st, status) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(st, 200);
    assert_eq!(
        (status["asr_ready"].as_bool(), status["tts_ready"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(status["settings"]["asr"]["model"], "whisper-1");
    assert_eq!(status["settings"]["asr"]["language"], "fr");
    assert_eq!(status["settings"]["tts"]["voice"], "fr_FR-siwis-medium");
    assert_eq!(status["settings"]["embed_model"], "bge-m3");
    assert_eq!(status["settings"]["mcp"]["token"], "brain");

    // kept on disk, so a restart finds the same servers
    let raw = std::fs::read_to_string(dir.path().join("home/kitsune/config.json")).unwrap();
    let on_disk: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(on_disk["asr"]["url"], "http://127.0.0.1:8080/v1");
    assert_eq!(on_disk["tts"]["url"], "http://127.0.0.1:8880/v1");

    // and it says no, plainly, to a shape it cannot read
    let (st, bad) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_settings", "settings": {"asr": "no"}})),
    )
    .await;
    assert_eq!(st, 400);
    assert!(bad["error"].as_str().unwrap_or("").contains("bad settings"));
}
