//! The capture pipeline end to end against a stand-in local AI stack on 127.0.0.1 (a Whisper
//! server, a small language model held to the JSON schema, an embedding server, a Nominatim):
//! the spec's validation matrix and its edge cases.

use kitsune::{
    clock, AsrConfig, EmbedConfig, Error, GeocoderConfig, Kitsune, Mode, SearchOptions, Settings,
    SlmConfig, DIM,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// What the stand-in servers do; switch them off to play an outage.
#[derive(Default)]
struct Stack {
    asr_up: AtomicBool,
    geo_up: AtomicBool,
    embed_up: AtomicBool,
    geo_calls: AtomicUsize,
}

fn read_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
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
                let first = String::from_utf8_lossy(&req[..h])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                return (first, req[h + 4..].to_vec());
            }
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return (String::new(), req),
            Ok(n) => req.extend_from_slice(&buf[..n]),
        }
    }
}

fn reading(
    summary: &str,
    category: &str,
    tags: &[&str],
    trigger: &str,
    when: Value,
    place: &str,
) -> String {
    json!({"clean_summary": summary, "category": category, "tags": tags, "trigger_type": trigger,
           "when": when, "place": place})
    .to_string()
}

fn no_when() -> Value {
    json!({"date": "", "day_offset": 0, "part": "none", "time": ""})
}

/// What the language model answers for a transcript (as a schema-held 3B model would).
fn slm_answer(transcript: &str) -> String {
    let t = transcript.to_lowercase();
    if t.contains("julie") {
        reading(
            "Donner le code du portail à Julie",
            "contact",
            &["julie", "code"],
            "geofence",
            no_when(),
            "12 rue Oberkampf, Paris",
        )
    } else if t.contains("comptable") {
        reading(
            "Appeler le comptable",
            "tache",
            &["comptable"],
            "time",
            json!({"date": "", "day_offset": 1, "part": "morning", "time": ""}),
            "",
        )
    } else if t.contains("café") {
        reading(
            "Racheter du café",
            "tache",
            &["café"],
            "none",
            no_when(),
            "",
        )
    } else if t.contains("rue de la paix") && t.contains("rappelle") {
        reading(
            "Acheter des fleurs rue de la Paix",
            "course",
            &["fleurs"],
            "geofence",
            no_when(),
            "rue de la Paix",
        )
    } else if t.contains("rue de la paix") {
        reading(
            "Le restaurant de la rue de la Paix était excellent",
            "memo",
            &["restaurant"],
            "none",
            no_when(),
            "",
        )
    } else if t.contains("nulle part") {
        reading(
            "Passer chez Nulle Part",
            "tache",
            &[],
            "geofence",
            no_when(),
            "Nulle Part",
        )
    } else if t.contains("boulangerie") {
        reading(
            "Prendre le pain à la boulangerie",
            "course",
            &["pain"],
            "geofence",
            no_when(),
            "Boulangerie Martin, Lyon",
        )
    } else if t.contains("markdown") {
        // a model that is not held to the schema: wrapped in a fence, which is never stripped
        format!(
            "```json\n{}\n```",
            reading("x", "memo", &[], "none", no_when(), "")
        )
    } else {
        reading(transcript, "memo", &[], "none", no_when(), "")
    }
}

/// A 384-wide vector where words of one family share an axis.
fn embedding(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; DIM];
    for w in text.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        let axis = match w {
            "café" | "expresso" | "caféine" => 0,
            "comptable" | "impôts" => 1,
            "" | "passage" | "query" => continue,
            _ => 3 + w.bytes().map(|b| b as usize).sum::<usize>() % (DIM - 3),
        };
        v[axis] += 1.0;
    }
    if v.iter().all(|x| *x == 0.0) {
        v[2] = 1.0;
    }
    v
}

fn serve(stack: Arc<Stack>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let stack = Arc::clone(&stack);
            std::thread::spawn(move || {
                let mut stream = stream;
                let (first, body) = read_request(&mut stream);
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, reply) = answer(&stack, &path, &body);
                let _ = stream.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes(),
                );
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn answer(stack: &Stack, path: &str, body: &[u8]) -> (&'static str, String) {
    let down = ("503 Service Unavailable", "{}".to_string());
    if path.ends_with("/audio/transcriptions") {
        if !stack.asr_up.load(Ordering::SeqCst) {
            return down;
        }
        let raw = String::from_utf8_lossy(body);
        assert!(
            raw.contains("verbose_json"),
            "the server is asked for verbose_json"
        );
        // the "audio" is the words themselves, or NOISE
        let said = raw
            .split("SAID:")
            .nth(1)
            .and_then(|s| s.split("\r\n").next())
            .unwrap_or("");
        let v = if said == "NOISE" {
            json!({"text": "...", "language": "fr", "segments": [{"avg_logprob": -1.4, "no_speech_prob": 0.92}]})
        } else {
            json!({"text": said, "language": "fr", "segments": [{"avg_logprob": -0.2, "no_speech_prob": 0.01}]})
        };
        return ("200 OK", v.to_string());
    }
    if path.ends_with("/chat/completions") {
        let req: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(req["response_format"]["type"], "json_schema");
        assert_eq!(req["response_format"]["json_schema"]["strict"], true);
        let user = req["messages"][1]["content"].as_str().unwrap();
        let transcript = user.split("Transcription : ").nth(1).unwrap();
        let content = slm_answer(transcript);
        return (
            "200 OK",
            json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
                .to_string(),
        );
    }
    if path.ends_with("/embeddings") {
        if !stack.embed_up.load(Ordering::SeqCst) {
            return down;
        }
        let req: Value = serde_json::from_slice(body).unwrap();
        let v = embedding(req["input"].as_str().unwrap());
        return ("200 OK", json!({"data": [{"embedding": v}]}).to_string());
    }
    if path.starts_with("/search?") {
        stack.geo_calls.fetch_add(1, Ordering::SeqCst);
        if !stack.geo_up.load(Ordering::SeqCst) {
            return down;
        }
        let q = path.to_lowercase();
        let hits = if q.contains("oberkampf") {
            json!([{"lat": "48.8650", "lon": "2.3780", "display_name": "12 Rue Oberkampf, Paris"}])
        } else if q.contains("paix") {
            json!([{"lat": "48.8687", "lon": "2.3317", "display_name": "Rue de la Paix, Paris"},
                   {"lat": "47.2138", "lon": "-1.5566", "display_name": "Rue de la Paix, Nantes"}])
        } else if q.contains("boulangerie") {
            json!([{"lat": 45.7640, "lon": 4.8357, "display_name": "Boulangerie Martin, Lyon"}])
        } else {
            json!([])
        };
        return ("200 OK", hits.to_string());
    }
    ("404 Not Found", "{}".into())
}

fn brain(stack: &Arc<Stack>) -> (tempfile::TempDir, Kitsune) {
    for s in [&stack.asr_up, &stack.geo_up, &stack.embed_up] {
        s.store(true, Ordering::SeqCst);
    }
    let base = serve(Arc::clone(stack));
    let dir = tempfile::tempdir().unwrap();
    let mut k = Kitsune::open(dir.path()).unwrap();
    k.set_settings(Settings {
        asr: AsrConfig {
            url: format!("{base}/v1"),
            language: "fr".into(),
            timeout: 10,
            ..Default::default()
        },
        slm: SlmConfig {
            url: format!("{base}/v1"),
            model: "qwen2.5-3b-instruct".into(),
            timeout: 10,
            ..Default::default()
        },
        embed: EmbedConfig {
            url: format!("{base}/v1"),
            model: "multilingual-e5-small".into(),
            passage_prefix: "passage: ".into(),
            query_prefix: "query: ".into(),
            timeout: 10,
        },
        geocoder: GeocoderConfig {
            url: base,
            radius_meters: 150,
            timeout: 10,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    (dir, k)
}

fn wav(said: &str) -> Vec<u8> {
    format!("RIFFSAID:{said}").into_bytes()
}

fn audio_files(dir: &std::path::Path) -> usize {
    walk(&dir.join("audio"))
}

fn walk(p: &std::path::Path) -> usize {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    if e.path().is_dir() {
                        walk(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn the_validation_matrix() {
    let stack = Arc::new(Stack::default());
    let (dir, k) = brain(&stack);

    // 1. Julie's code: a place reminder, placed, in the R*Tree
    let julie = k
        .ingest_voice(
            &wav("Quand j'arrive chez Julie au 12 rue Oberkampf, lui donner le code du portail"),
            "",
            None,
        )
        .unwrap();
    let n = k.store().get_note(julie).unwrap().unwrap();
    assert_eq!(
        (
            n.trigger_type.as_str(),
            n.status.as_str(),
            n.category.as_str()
        ),
        ("geofence", "active", "contact")
    );
    assert_eq!(
        (n.latitude, n.longitude, n.radius_meters),
        (Some(48.8650), Some(2.3780), 150)
    );
    assert_eq!(n.location_raw, "12 rue Oberkampf, Paris");
    assert_eq!(n.lang, "fr");
    assert_eq!(
        k.store().entered(48.8654, 2.3780).unwrap()[0].id,
        julie,
        "inside, ~45 m away"
    );
    assert!(
        k.store().entered(48.8704, 2.3780).unwrap().is_empty(),
        "nothing ~600 m away"
    );

    // 2. the accountant, tomorrow morning: 09:00 UTC the next day
    let compta = k
        .ingest_voice(
            &wav("Rappelle-moi d'appeler le comptable demain matin"),
            "",
            None,
        )
        .unwrap();
    let n = k.store().get_note(compta).unwrap().unwrap();
    let tomorrow = clock::now().div_euclid(86_400) * 86_400 + 86_400 + 9 * 3600;
    assert_eq!(
        (n.trigger_type.as_str(), n.due_date.as_deref()),
        ("time", Some(clock::iso(tomorrow).as_str()))
    );
    assert_eq!(n.clean_summary, "Appeler le comptable");

    // 3. coffee: a task, no reminder
    let cafe = k
        .capture_text("Faut que je pense à racheter du café", vec![])
        .unwrap();
    let n = k.store().get_note(cafe).unwrap().unwrap();
    assert_eq!(
        (n.category.as_str(), n.trigger_type.as_str()),
        ("tache", "none")
    );

    // 4. a memo that names a place is not a place reminder, and the geocoder is not asked
    let calls = stack.geo_calls.load(Ordering::SeqCst);
    let memo = k
        .capture_text("Le resto de la Rue de la Paix était top", vec![])
        .unwrap();
    let n = k.store().get_note(memo).unwrap().unwrap();
    assert_eq!(
        (
            n.category.as_str(),
            n.trigger_type.as_str(),
            n.status.as_str()
        ),
        ("memo", "none", "active")
    );
    assert_eq!(stack.geo_calls.load(Ordering::SeqCst), calls);

    // 5. noise: refused as inaudible, nothing kept, no audio left behind
    let (notes, files) = (k.store().count_notes().unwrap(), audio_files(dir.path()));
    let e = k.ingest_voice(&wav("NOISE"), "", None).unwrap_err();
    assert!(matches!(e, Error::Inaudible(_)), "{e}");
    assert_eq!(
        (k.store().count_notes().unwrap(), audio_files(dir.path())),
        (notes, files)
    );

    // and by meaning: "expresso" finds the coffee, which no word of it says
    let hits = k
        .search(
            "expresso",
            &SearchOptions {
                mode: Mode::Semantic,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(hits[0].note.id, cafe);
    let words = k
        .search(
            "expresso",
            &SearchOptions {
                mode: Mode::Fts,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(words.is_empty());
    let both = k.search("comptable", &SearchOptions::default()).unwrap();
    assert_eq!(both[0].note.id, compta);
    assert!(both[0].text_rank.is_some() && both[0].vector_rank.is_some());
}

#[test]
fn a_place_not_found_ambiguous_or_offline() {
    let stack = Arc::new(Stack::default());
    let (_dir, k) = brain(&stack);

    // no result: no reminder, failed_geocode, tagged
    let id = k
        .capture_text("Rappelle-moi de passer chez Nulle Part", vec![])
        .unwrap();
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.trigger_type.as_str(), n.status.as_str()),
        ("none", "failed_geocode")
    );
    assert!(
        n.tags.iter().any(|t| t == "erreur:geocodage"),
        "{:?}",
        n.tags
    );

    // two rue de la Paix far apart and no position: ambiguous
    let id = k
        .capture_text("Rappelle-moi les fleurs rue de la Paix", vec![])
        .unwrap();
    assert_eq!(
        k.store().get_note(id).unwrap().unwrap().status,
        "ambiguous_geocode"
    );
    // the phone said where the person is (Nantes): the nearest one is taken
    k.seen_at(47.21, -1.55);
    let id = k
        .capture_text("Rappelle-moi les fleurs rue de la Paix", vec![])
        .unwrap();
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!((n.status.as_str(), n.latitude), ("active", Some(47.2138)));

    // the geocoder is away: the place waits, then is found when it is back
    stack.geo_up.store(false, Ordering::SeqCst);
    let id = k
        .capture_text("Rappelle-moi le pain à la boulangerie", vec![])
        .unwrap();
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.trigger_type.as_str(), n.status.as_str()),
        ("none", "pending_geocode")
    );
    assert_eq!(n.location_raw, "Boulangerie Martin, Lyon");
    assert_eq!(k.retry().unwrap().placed, 0, "still away: nothing done");
    stack.geo_up.store(true, Ordering::SeqCst);
    assert_eq!(k.retry().unwrap().placed, 1);
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.trigger_type.as_str(), n.status.as_str(), n.radius_meters),
        ("geofence", "active", 150)
    );
}

#[test]
fn what_waited_for_a_server_is_done_later() {
    let stack = Arc::new(Stack::default());
    let (_dir, k) = brain(&stack);

    // the voice server is away: the audio is kept, the note waits for its words
    stack.asr_up.store(false, Ordering::SeqCst);
    let id = k
        .ingest_voice(
            &wav("Rappelle-moi d'appeler le comptable demain matin"),
            "bureau",
            None,
        )
        .unwrap();
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.status.as_str(), n.location_raw.as_str()),
        ("pending_transcription", "bureau")
    );
    // the embedding server is away too: a typed note is kept, found by its words only
    stack.embed_up.store(false, Ordering::SeqCst);
    let cafe = k.capture_text("Racheter du café", vec![]).unwrap();
    assert!(k.store().embedding(cafe).unwrap().is_none());

    // both come back: the audio is transcribed and read, the note gets its meaning
    stack.asr_up.store(true, Ordering::SeqCst);
    stack.embed_up.store(true, Ordering::SeqCst);
    let done = k.retry().unwrap();
    assert_eq!(done.transcribed, 1);
    assert!(done.embedded >= 1);
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.status.as_str(), n.trigger_type.as_str()),
        ("active", "time")
    );
    assert_eq!(n.clean_summary, "Appeler le comptable");
    assert!(k.store().embedding(cafe).unwrap().is_some());
    assert!(k.store().embedding(id).unwrap().is_some());
}

#[test]
fn an_answer_outside_the_schema_is_not_repaired_the_note_is_kept_as_said() {
    let stack = Arc::new(Stack::default());
    let (_dir, k) = brain(&stack);
    let id = k.capture_text("markdown s'il te plaît", vec![]).unwrap();
    let n = k.store().get_note(id).unwrap().unwrap();
    assert_eq!(
        (n.clean_summary.as_str(), n.category.as_str()),
        ("markdown s'il te plaît", "memo")
    );
}

#[test]
fn a_cloud_address_is_refused_whatever_the_settings_say() {
    let dir = tempfile::tempdir().unwrap();
    let mut k = Kitsune::open(dir.path()).unwrap();
    k.set_settings(Settings {
        slm: SlmConfig {
            url: "http://8.8.8.8/v1".into(),
            timeout: 1,
            ..Default::default()
        },
        asr: AsrConfig {
            url: "http://1.1.1.1/v1".into(),
            timeout: 1,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    // the thought is kept as said, nothing went out
    let id = k.capture_text("Racheter du café", vec![]).unwrap();
    assert_eq!(k.store().get_note(id).unwrap().unwrap().category, "memo");
    // the audio is kept for later, never sent
    let id = k.ingest_voice(b"RIFF", "", None).unwrap();
    assert_eq!(
        k.store().get_note(id).unwrap().unwrap().status,
        "pending_transcription"
    );
}
