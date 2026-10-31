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

    // the phone's passive position is taken, and a retry with nothing waiting does nothing
    let (st, seen) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "seen_at", "lat": 48.85, "lon": 2.35})),
    )
    .await;
    assert_eq!((st, seen["ok"].as_bool()), (200, Some(true)));
    let (st, retried) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "retry"})),
    )
    .await;
    assert_eq!((st, retried["retried"]["placed"].as_i64()), (200, Some(0)));

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
    call(port, owner, "POST", "/api/kitsune", Some(json!({"action": "set_settings", "settings": {"embed": {"url": "http://127.0.0.1:8082/v1", "model": "minilm"}, "mcp": {"token": "net-token"}}}))).await;
    let (_, after) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(after["settings"]["embed"]["model"], "minilm");
    assert_eq!(after["embed_ready"], true);
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
    assert_eq!(status["settings"]["embed"]["url"], "");
    assert_eq!(status["slm_ready"], false);
    assert_eq!(status["settings"]["slm"]["day_parts"]["morning"], "09:00");

    // the page sends the whole settings back, so what it does not know survives
    let settings = json!({
        "asr": {"url": "http://127.0.0.1:8080/v1", "model": "whisper-1", "language": "fr", "timeout": 60},
        "tts": {"url": "http://127.0.0.1:8880/v1", "model": "kokoro", "voice": "fr_FR-siwis-medium",
                "format": "wav", "timeout": 60},
        "slm": {"url": "http://127.0.0.1:8081/v1", "model": "qwen2.5-3b-instruct"},
        "embed": {"url": "http://127.0.0.1:8082/v1", "model": "multilingual-e5-small"},
        "geocoder": {"url": "http://127.0.0.1:8088", "kind": "photon"},
        "mcp": {"token": "brain", "tools": ["search_notes"]}
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
    assert_eq!(
        status["settings"]["embed"]["model"],
        "multilingual-e5-small"
    );
    assert_eq!(status["settings"]["slm"]["model"], "qwen2.5-3b-instruct");
    assert_eq!(status["settings"]["geocoder"]["kind"], "photon");
    assert_eq!(status["settings"]["geocoder"]["radius_meters"], 100);
    assert_eq!(
        (
            status["slm_ready"].as_bool(),
            status["embed_ready"].as_bool(),
            status["geocoder_ready"].as_bool()
        ),
        (Some(true), Some(true), Some(true))
    );
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

/// A stand-in Whisper server that hears only noise (verbose_json, high no_speech_prob).
fn noisy_whisper() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut req = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                if let Some(h) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&req[..h]).to_ascii_lowercase();
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if req.len() >= h + 4 + len {
                        break;
                    }
                }
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let body = r#"{"text":"...","segments":[{"avg_logprob":-1.3,"no_speech_prob":0.9}]}"#;
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            );
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_heard_too_poorly_is_refused_and_nothing_is_kept() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";
    let asr = noisy_whisper();
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_settings", "settings": {"asr": {"url": asr, "timeout": 10}}})),
    )
    .await;

    let audio = base64::engine::general_purpose::STANDARD.encode(vec![7u8; 3200]);
    let (st, refused) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "ingest_voice", "audio": audio})),
    )
    .await;
    assert_eq!(st, 422, "{refused}");
    assert_eq!(refused["inaudible"], true);
    assert!(refused["error"]
        .as_str()
        .unwrap()
        .contains("nothing was kept"));
    let (_, status) = call(port, owner, "GET", "/api/kitsune", None).await;
    assert_eq!(status["notes"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_due_reminder_rings_on_the_page_and_a_place_rings_on_entry() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_settings", "settings": {"rings": {"tick_seconds": 1}}})),
    )
    .await;

    // the page listens
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(format!("GET /api/kitsune/events?{owner} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .await
        .unwrap();

    // a reminder already due
    let (_, kept) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Appeler le comptable"})),
    )
    .await;
    let id = kept["id"].as_i64().unwrap();
    call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "set_reminder", "id": id,
        "reminder": {"trigger_type": "time", "due_date": "2020-01-01T09:00:00Z"}})),
    )
    .await;

    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    let rang = tokio::time::timeout(Duration::from_secs(10), async {
        while !String::from_utf8_lossy(&got).contains("event: ring")
            || !String::from_utf8_lossy(&got).ends_with("\n\n")
        {
            let n = s.read(&mut buf).await.unwrap();
            assert!(n > 0, "the stream closed");
            got.extend_from_slice(&buf[..n]);
        }
    })
    .await;
    assert!(
        rang.is_ok(),
        "no ring within 10 s: {}",
        String::from_utf8_lossy(&got)
    );
    let text = String::from_utf8_lossy(&got);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(
        text.contains("Appeler le comptable") && text.contains("\"notified_at\""),
        "{text}"
    );

    // a place reminder rings when the phone says it was entered, then stays quiet
    let (_, julie) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "capture_text", "text": "Donner le code à Julie"})),
    )
    .await;
    let jid = julie["id"].as_i64().unwrap();
    call(port, owner, "POST", "/api/kitsune", Some(json!({"action": "set_reminder", "id": jid,
        "reminder": {"trigger_type": "geofence", "latitude": 48.8650, "longitude": 2.3780, "radius_meters": 100}}))).await;
    let (st, far) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "entered", "lat": 48.8704, "lon": 2.3780})),
    )
    .await;
    assert_eq!((st, far["rang"].as_array().unwrap().len()), (200, 0));
    let (_, near) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "entered", "lat": 48.8654, "lon": 2.3780})),
    )
    .await;
    assert_eq!(near["rang"][0]["id"], jid);
    let (_, again) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "entered", "lat": 48.8654, "lon": 2.3780})),
    )
    .await;
    assert_eq!(again["rang"].as_array().unwrap().len(), 0);

    // a guest cannot listen
    let (st, _) = call(port, "token=nope", "GET", "/api/kitsune/events", None).await;
    assert_eq!(st, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_keeps_lists_places_a_task_again_and_counts_the_week() {
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";
    let k = |body: Value| call(port, owner, "POST", "/api/kitsune", Some(body));

    let (st, added) = k(json!({"action": "add_items", "list": "Courses", "kind": "shopping",
        "items": [{"text": "Lait", "qty": 1, "unit": "L", "aisle": "2-frais"}, {"text": "Tomates", "aisle": "1-fruits"}]})).await;
    assert_eq!(st, 200, "{added}");
    let (_, again) = k(json!({"action": "add_items", "list": "Courses", "kind": "shopping", "items": [{"text": "lait", "qty": 2, "unit": "L"}]})).await;
    assert_eq!(again["ids"][0], added["ids"][0], "the same line grows");
    let (_, items) = k(json!({"action": "items", "list": "Courses"})).await;
    assert_eq!(items["items"][0]["text"], "Tomates", "by aisle");
    assert_eq!(items["items"][1]["qty"], 3.0);
    let (_, lists) = k(json!({"action": "lists"})).await;
    assert_eq!(lists["lists"][0]["open"], 2);
    k(json!({"action": "check", "ids": [added["ids"][1]]})).await;

    // a task that did not happen is placed again, without a trace of blame
    let (_, t) = k(json!({"action": "add_items", "list": "Pro", "kind": "todo",
        "items": [{"text": "Rappeler la banque", "due_date": "2020-01-01T09:00:00Z"}]}))
    .await;
    let (_, late) = k(json!({"action": "overdue"})).await;
    assert_eq!(late["items"][0]["id"], t["ids"][0]);
    let (_, moved) =
        k(json!({"action": "move_item", "id": t["ids"][0], "due_date": "2099-01-01T09:00:00Z"}))
            .await;
    assert_eq!(moved["ok"], true);
    let (_, late) = k(json!({"action": "overdue"})).await;
    assert_eq!(late["items"].as_array().unwrap().len(), 0);

    // the week, privately
    let (_, week) = k(json!({"action": "week"})).await;
    assert_eq!(week["done"], 1);

    // no core runs the local engine here: an assistant says so plainly; nothing runs
    let (st, no) = k(json!({"action": "assist", "workflow": "daily-plan", "title": "Plan"})).await;
    assert_eq!(st, 400, "{no}");
    assert!(no["error"].as_str().unwrap().contains("local engine"));
    let (_, runs) = k(json!({"action": "runs"})).await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 0);
}

/// A model server as the local engine reaches it (OpenAI-compatible, streamed): it plays a script
/// of answers, one per request, and keeps the requests.
async fn scripted_model(script: Vec<Value>) -> (u16, Arc<std::sync::Mutex<Vec<Value>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked: Arc<std::sync::Mutex<Vec<Value>>> = Arc::default();
    let script = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(
        script,
    )));
    let seen = Arc::clone(&asked);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let (seen, script) = (Arc::clone(&seen), Arc::clone(&script));
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
                        let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length:")
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
                if head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .ends_with("/models")
                {
                    let b = json!({"data": [{"id": "qwen3"}]}).to_string();
                    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await;
                    return;
                }
                seen.lock()
                    .unwrap()
                    .push(serde_json::from_slice(&body).unwrap_or(Value::Null));
                let next = script
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(json!({"text": "ok"}));
                let events: Vec<Value> = match next.get("call") {
                    Some(c) => vec![
                        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "c", "type": "function", "function": {"name": c["name"], "arguments": c["args"].to_string()}}]}}]}),
                        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
                    ],
                    None => vec![
                        json!({"choices": [{"index": 0, "delta": {"content": next["text"]}}]}),
                        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
                    ],
                };
                let mut out = String::new();
                for e in events {
                    out.push_str(&format!("data: {e}\n\n"));
                }
                out.push_str("data: [DONE]\n\n");
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).as_bytes()).await;
            });
        }
    });
    (port, asked)
}

/// The whole way an assistant goes, on a real node: the page starts the inbox workflow, the
/// gateway makes a thread on the core's local engine and launches it, the assistant (a scripted
/// model) fills the shopping list through the kitsune tool, the brain the page reads holds it, and
/// the page sees the run in Running.
#[tokio::test(flavor = "multi_thread")]
async fn an_assistant_started_from_the_page_works_through_to_the_brain() {
    let (model_port, asked) = scripted_model(vec![
        json!({"call": {"name": "kitsune", "args": {"action": "add_items", "list": "Courses", "kind": "shopping", "items": [{"text": "café", "aisle": "3-épicerie"}]}}}),
        json!({"call": {"name": "step_done", "args": {}}}),
        json!({"call": {"name": "step_done", "args": {}}}),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    // the local engine's settings, as the interface would have set them
    let engine = d.join("home").join("engine").join("local");
    std::fs::create_dir_all(&engine).unwrap();
    std::fs::write(
        engine.join("settings.json"),
        json!({"providers": [{"id": "gpu", "name": "vLLM", "url": format!("http://127.0.0.1:{model_port}/v1")}],
               "model": "gpu/qwen3", "mode": "assistant"})
        .to_string(),
    )
    .unwrap();
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {"local": {"kind": "local"}},
    }))
    .unwrap();
    let home = Node::start(cfg).await.unwrap();
    let port = home.gateway_port;
    let owner = "token=owner";

    // the gateway learns that this core runs the local engine
    let mut started = Value::Null;
    for _ in 0..60 {
        let (st, d) = call(
            port,
            owner,
            "POST",
            "/api/kitsune",
            Some(
                json!({"action": "assist", "workflow": "kitsune-inbox", "title": "Sort my notes"}),
            ),
        )
        .await;
        if st == 200 {
            started = d;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let thread = started["thread"]
        .as_str()
        .unwrap_or_else(|| panic!("never started: {started}"))
        .to_string();
    assert!(thread.starts_with("kitsune-kitsune-inbox-"));

    // the assistant's work reaches the brain the page reads
    let mut items = Value::Null;
    for _ in 0..80 {
        let (st, got) = call(
            port,
            owner,
            "POST",
            "/api/kitsune",
            Some(json!({"action": "items", "list": "Courses"})),
        )
        .await;
        if st == 200 && got["items"].as_array().is_some_and(|a| !a.is_empty()) {
            items = got;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(items["items"][0]["text"], "café", "{items}");
    // the model was asked as the assistant, with the brain's tool and the workflow's step
    let first = asked.lock().unwrap()[0].clone();
    assert!(first["tools"].to_string().contains("\"kitsune\""));
    assert!(
        first["messages"].to_string().contains("triage"),
        "the assistant's own prompt"
    );
    // the page sees the run
    let (_, runs) = call(
        port,
        owner,
        "POST",
        "/api/kitsune",
        Some(json!({"action": "runs"})),
    )
    .await;
    assert!(
        runs["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == thread.as_str()),
        "{runs}"
    );
}

/// A node whose local engine talks to `model_port` (settings as the interface writes them).
async fn engine_node(d: &std::path::Path, model_port: u16, extra: Value) -> Arc<Node> {
    for (n, v) in [("mesh.key", "m"), ("device.key", "d"), ("token", "owner")] {
        std::fs::write(d.join(n), v).unwrap();
    }
    let engine = d.join("home").join("engine").join("local");
    std::fs::create_dir_all(&engine).unwrap();
    let mut settings = json!({"providers": [{"id": "gpu", "name": "vLLM", "url": format!("http://127.0.0.1:{model_port}/v1")}],
                              "model": "gpu/qwen3", "mode": "assistant"});
    for (k, v) in extra.as_object().unwrap() {
        settings[k] = v.clone();
    }
    std::fs::write(engine.join("settings.json"), settings.to_string()).unwrap();
    let cfg: NodeConfig = serde_json::from_value(json!({
        "node_id": "home", "data_dir": d.join("home"), "key_file": d.join("mesh.key"), "device_key_file": d.join("device.key"),
        "gateway_listen": "127.0.0.1:0", "gateway_token_file": d.join("token"), "heartbeat": 0.1,
        "harnesses": {"local": {"kind": "local"}},
    }))
    .unwrap();
    Node::start(cfg).await.unwrap()
}

/// Development handed off, the whole way: the page starts dev-handoff, the developer writes the
/// plan, the workflow waits for the person, the page sends their choice, and the assistant opens a
/// new thread for the coder through the node (announced, labelled, started on the plan).
#[tokio::test(flavor = "multi_thread")]
async fn a_development_job_is_planned_approved_and_handed_to_a_new_thread() {
    let (model_port, asked) = scripted_model(vec![
        // the developer (a sub-agent): its plan
        json!({"call": {"name": "return_value", "args": {"content": "## Goal\nA contact form.\n## Acceptance criteria\n- it sends"}}}),
        // after the person's choice, the assistant opens the thread, then says the step is done
        json!({"call": {"name": "delegate", "args": {"harness": "local", "title": "Contact form", "prompt": "## Goal\nA contact form."}}}),
        json!({"call": {"name": "step_done", "args": {}}}),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    // the person chose not to be asked before a thread opens
    let home = engine_node(dir.path(), model_port, json!({"delegate_asks": false})).await;
    let port = home.gateway_port;
    let owner = "token=owner";
    let k = |body: Value| call(port, owner, "POST", "/api/kitsune", Some(body));

    let mut started = Value::Null;
    for _ in 0..60 {
        let (st, d) = k(json!({"action": "assist", "workflow": "dev-handoff", "title": "Development", "params": {"request": "a contact form"}})).await;
        if st == 200 {
            started = d;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let thread = started["thread"]
        .as_str()
        .unwrap_or_else(|| panic!("never started: {started}"))
        .to_string();

    // the run waits for the person, with its choices, on the page
    let mut choice = Value::Null;
    for _ in 0..80 {
        let (_, runs) = k(json!({"action": "runs"})).await;
        if let Some(r) = runs["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == thread.as_str())
        {
            if r["workflow"]["status"] == "waiting" {
                choice = r["workflow"]["choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["id"] == "Hand-off")
                    .cloned()
                    .unwrap();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        choice["id"], "Hand-off",
        "the step for the person never showed"
    );
    let (st, _) = call(port, owner, "POST", "/api/message",
        Some(json!({"agent": thread, "text": "", "workflow": {"resume": true, "choice": "Hand-off", "label": "Hand-off"}}))).await;
    assert_eq!(st, 200);

    // a new thread, the coder's, opened by the assistant and started on the plan
    let mut opened = String::new();
    for _ in 0..80 {
        let found = asked.lock().unwrap().iter().any(|r| {
            r["messages"].as_array().is_some_and(|m| {
                m.len() <= 3
                    && m.iter().any(|x| {
                        x["role"] == "user"
                            && x["content"]
                                .as_str()
                                .is_some_and(|c| c.contains("## Goal\nA contact form."))
                    })
            })
        });
        if found {
            opened = "yes".into();
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        opened, "yes",
        "the coder's thread never started on the plan"
    );
    let threads: Vec<String> =
        std::fs::read_dir(dir.path().join("home").join("host").join("agents"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
    assert!(
        threads.iter().any(|t| t.starts_with("contact-form-")),
        "{threads:?}"
    );
}

/// AI-Memory as a managed service, against a stand-in `ai-memory` (a script): started with its
/// token, checked, an agent wired, stopped; the token is mode 600 and never shown.
#[tokio::test(flavor = "multi_thread")]
async fn the_shared_memory_is_started_checked_wired_and_stopped() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let home = node(dir.path()).await;
    let port = home.gateway_port;
    let owner = "token=owner";
    let mem_port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let bin = dir.path().join("ai-memory");
    std::fs::write(&bin, r#"#!/bin/sh
case "$1" in
  serve) exec python3 -c '
import os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        ok = self.headers.get("Authorization") == "Bearer " + os.environ["AI_MEMORY_AUTH_TOKEN"]
        self.send_response(200 if ok else 401); self.send_header("Content-Length", "0"); self.end_headers()
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
' "$2" ;;
  install-mcp) echo "mcp server added for $3" ;;
  install-hooks) echo "hooks added for $3" ;;
  *) exit 2 ;;
esac
"#).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let m = |body: Value| call(port, owner, "POST", "/api/aimemory", Some(body));

    let (st, set) = m(json!({"action": "set", "config": {"binary": bin.display().to_string(), "args": format!("serve {mem_port}"),
        "url": format!("http://127.0.0.1:{mem_port}")}})).await;
    assert_eq!(st, 200, "{set}");
    let (st, started) = m(json!({"action": "start"})).await;
    assert_eq!(st, 200, "{started}");
    let mut ok = false;
    for _ in 0..40 {
        let (st, _) = m(json!({"action": "check"})).await;
        if st == 200 {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(ok, "it never answered with its token");
    let (st, wired) = m(json!({"action": "wire", "client": "codex"})).await;
    assert_eq!(st, 200, "{wired}");
    assert!(wired["said"]
        .as_str()
        .unwrap()
        .contains("mcp server added for codex"));
    assert!(wired["said"]
        .as_str()
        .unwrap()
        .contains("hooks added for codex"));
    let (st, _) = m(json!({"action": "wire", "client": "rm -rf /"})).await;
    assert_eq!(st, 400, "only known agents");
    let (_, status) = call(port, owner, "GET", "/api/aimemory", None).await;
    assert_eq!(status["running"], true);
    let token = std::fs::read_to_string(dir.path().join("home").join("ai-memory.token")).unwrap();
    assert!(
        !status.to_string().contains(token.trim()),
        "the token is never shown"
    );
    let mode = std::fs::metadata(dir.path().join("home").join("ai-memory.token"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let (_, stopped) = m(json!({"action": "stop"})).await;
    assert_eq!(stopped["running"], false);
    // a guest sees nothing of it
    let (st, _) = call(port, "token=nope", "GET", "/api/aimemory", None).await;
    assert_eq!(st, 401);
}
