//! The whole second brain behind one door: the store, the sealed audio, and the servers it uses
//! (voice-to-text now, voice synthesis later). This is what a core holds and what the GUI
//! configures; everything a path or a server address is a setting kept in `config.json`, never
//! hard-coded. Opening it readies the store (its schema) and the audio key.

use crate::asr::{self, AsrConfig};
use crate::mcp::Config as McpConfig;
use crate::store::{AudioRef, NewNote};
use crate::tts::{self, TtsConfig};
use crate::{AudioVault, Key, Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Everything the brain is told, from the GUI. Nothing here is assumed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub asr: AsrConfig,
    /// The embedding model that gives notes their meaning vector (empty: search by words only).
    #[serde(default)]
    pub embed_model: String,
    #[serde(default)]
    pub tts: TtsConfig,
    /// What the MCP face offers the agentic network.
    #[serde(default)]
    pub mcp: McpConfig,
}

pub struct Kitsune {
    dir: PathBuf,
    store: Store,
    vault: AudioVault,
    settings: Settings,
}

impl Kitsune {
    /// Open (or create) a brain under `dir`: the store at `dir/kitsune.db`, the audio sealed under
    /// `dir/audio/` with the key at `dir/audio.key`, the settings at `dir/config.json`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| crate::Error::Invalid(e.to_string()))?;
        let store = Store::open(&dir.join("kitsune.db"))?;
        let key = Key::load_or_create(&dir.join("audio.key"))?;
        let vault = AudioVault::new(dir, key);
        let settings = std::fs::read(dir.join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Self {
            dir: dir.to_path_buf(),
            store,
            vault,
            settings,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_settings(&mut self, s: Settings) -> Result<()> {
        std::fs::write(
            self.dir.join("config.json"),
            serde_json::to_vec_pretty(&s).expect("serializes"),
        )
        .map_err(|e| crate::Error::Invalid(e.to_string()))?;
        self.settings = s;
        Ok(())
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn vault(&self) -> &AudioVault {
        &self.vault
    }

    /// Keep a written note.
    pub fn capture_text(&self, text: &str, tags: Vec<String>) -> Result<i64> {
        self.store.capture_note(&NewNote {
            text: text.into(),
            source: "text".into(),
            tags,
            ..Default::default()
        })
    }

    /// A voice capture, start to end: seal the audio, transcribe it, keep the note linked to it.
    /// When the server cannot transcribe, the audio is kept and the note waits for a review, so
    /// nothing the person said is ever lost.
    pub fn ingest_voice(&self, wav: &[u8], place: &str, at: Option<(f64, f64)>) -> Result<i64> {
        let rel = self.vault.put(wav, "wav")?;
        let transcription = asr::transcribe(&self.settings.asr, wav);
        let (lat, lon) = match at {
            Some((la, lo)) => (Some(la), Some(lo)),
            None => (None, None),
        };
        let note = match &transcription {
            Ok(text) => NewNote {
                text: text.clone(),
                source: "voice".into(),
                lang: self.settings.asr.language.clone(),
                place: place.into(),
                lat,
                lon,
                ..Default::default()
            },
            Err(_) => NewNote {
                text: String::new(),
                source: "voice".into(),
                needs_review: true,
                place: place.into(),
                lat,
                lon,
                ..Default::default()
            },
        };
        let note_id = self.store.capture_note(&note)?;
        self.store.add_audio(&AudioRef {
            id: 0,
            kind: "capture".into(),
            note_id: Some(note_id),
            path: rel,
            mime: "audio/wav".into(),
            duration_ms: 0,
            bytes: wav.len() as i64,
            sha256: String::new(),
            voice: json!({}),
        })?;
        Ok(note_id)
    }

    /// Say a text aloud: synthesise it on the voice server, seal the audio, keep it (linked to a
    /// note when one is given). The audio bytes to play now, its media type, and the stored path.
    pub fn speak(&self, text: &str, note_id: Option<i64>) -> Result<(Vec<u8>, String, String)> {
        if let Some(id) = note_id {
            if self.store.get_note(id)?.is_none() {
                return Err(crate::Error::Invalid(format!("no note {id} to speak")));
            }
        }
        let (audio, format) = tts::synthesize(&self.settings.tts, text)?;
        let rel = self.vault.put(&audio, &format)?;
        let mime = tts::mime_of(&format).to_string();
        self.store.add_audio(&AudioRef {
            id: 0,
            kind: "tts".into(),
            note_id,
            path: rel.clone(),
            mime: mime.clone(),
            duration_ms: 0,
            bytes: audio.len() as i64,
            sha256: String::new(),
            voice: json!({"model": self.settings.tts.model, "voice": self.settings.tts.voice, "format": format}),
        })?;
        Ok((audio, mime, rel))
    }

    /// Answer one MCP request against the brain (the agentic network's door).
    pub fn mcp(&self, request: &Value) -> Option<Value> {
        // synthesize_speech needs the vault and the voice server, so the facade answers it here;
        // the rest is the store-only handler
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let name = request
            .pointer("/params/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if method == "tools/call"
            && name == "synthesize_speech"
            && self.settings.mcp.offers("synthesize_speech")
        {
            let id = request.get("id").cloned()?;
            let args = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            if !self.settings.mcp.token.is_empty()
                && args.get("_auth").and_then(Value::as_str)
                    != Some(self.settings.mcp.token.as_str())
            {
                return Some(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32001, "message": "this tool needs the right _auth token"}}),
                );
            }
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            let note_id = args.get("note_id").and_then(Value::as_i64);
            let result = match self.speak(text, note_id) {
                Ok((_audio, mime, rel)) => {
                    json!({"content": [{"type": "text", "text": format!("Spoke it ({mime}), kept at {rel}.")}]})
                }
                Err(e) => {
                    json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})
                }
            };
            return Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }
        let mut cfg = self.settings.mcp.clone();
        if cfg.embed_model.is_empty() {
            cfg.embed_model = self.settings.embed_model.clone();
        }
        let mut reply = crate::mcp::handle(&self.store, &cfg, request)?;
        // add synthesize_speech to the list the facade can serve (the store handler does not know it)
        if method == "tools/list" && self.settings.tts.ready() && cfg.offers("synthesize_speech") {
            if let Some(tools) = reply
                .pointer_mut("/result/tools")
                .and_then(Value::as_array_mut)
            {
                tools.push(json!({"name": "synthesize_speech", "description": "Say a text aloud in the configured voice; keeps the audio, linked to a note when note_id is given.",
                    "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}, "note_id": {"type": "integer"}}, "required": ["text"]}}));
            }
        }
        Some(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A stand-in transcription server: answers {"text": …} unless told to fail.
    fn fake_asr(reply: &'static str, status: &'static str) -> (String, Arc<Mutex<Vec<u8>>>) {
        let seen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let s2 = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                // read the whole request (headers + Content-Length body) before replying, as a
                // real server does, so the client never hits a reset mid-write
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let head_end = req.windows(4).position(|w| w == b"\r\n\r\n");
                    if let Some(h) = head_end {
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
                s2.lock().unwrap().extend_from_slice(&req);
                let body = reply;
                let _ = stream.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
                );
            }
        });
        (format!("http://127.0.0.1:{port}/v1"), seen)
    }

    #[test]
    fn a_voice_capture_is_transcribed_sealed_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen) = fake_asr(r#"{"text": "Acheter des piles"}"#, "200 OK");
        let mut k = Kitsune::open(dir.path()).unwrap();
        k.set_settings(Settings {
            asr: AsrConfig {
                url,
                model: "whisper-1".into(),
                language: "fr".into(),
                timeout: 10,
            },
            ..Default::default()
        })
        .unwrap();
        let id = k
            .ingest_voice(b"RIFF fake wav", "Cuisine", Some((48.85, 2.35)))
            .unwrap();
        let note = k.store().get_note(id).unwrap().unwrap();
        assert_eq!(
            (note.text.as_str(), note.status.as_str(), note.lang.as_str()),
            ("Acheter des piles", "ok", "fr")
        );
        assert_eq!(note.place, "Cuisine");
        // found by word, accents folded
        assert_eq!(k.store().search_text("piles", 5).unwrap()[0].0.id, id);
        // the audio is sealed on disk and reads back
        let audio = k.store().audio_of(id).unwrap();
        assert_eq!(audio.len(), 1);
        let on_disk = std::fs::read(dir.path().join(&audio[0].path)).unwrap();
        assert_ne!(on_disk, b"RIFF fake wav");
        assert_eq!(k.vault().get(&audio[0].path).unwrap(), b"RIFF fake wav");
        // the server received a multipart with the model and language
        let sent = String::from_utf8_lossy(&seen.lock().unwrap()).to_string();
        assert!(
            sent.contains("multipart/form-data")
                && sent.contains("name=\"model\"")
                && sent.contains("whisper-1")
        );
        assert!(sent.contains("name=\"language\"") && sent.contains("fr"));

        // settings persist across a reopen
        let k2 = Kitsune::open(dir.path()).unwrap();
        assert_eq!(k2.settings().asr.model, "whisper-1");
    }

    #[test]
    fn a_failed_transcription_keeps_the_audio_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _) = fake_asr("boom", "500 Internal Server Error");
        let mut k = Kitsune::open(dir.path()).unwrap();
        k.set_settings(Settings {
            asr: AsrConfig {
                url,
                timeout: 10,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let id = k.ingest_voice(b"RIFF fake wav", "", None).unwrap();
        let note = k.store().get_note(id).unwrap().unwrap();
        assert_eq!(note.status, "needs_review");
        // the audio is still there, so nothing said is lost
        assert_eq!(k.store().audio_of(id).unwrap().len(), 1);
        // a correction later makes it ok
        k.store().correct_note(id, "Rappeler le médecin").unwrap();
        assert_eq!(k.store().get_note(id).unwrap().unwrap().status, "ok");
    }

    #[test]
    fn the_mcp_door_uses_the_configured_embedding_model() {
        let dir = tempfile::tempdir().unwrap();
        let mut k = Kitsune::open(dir.path()).unwrap();
        k.set_settings(Settings {
            embed_model: "minilm".into(),
            ..Default::default()
        })
        .unwrap();
        k.capture_text("note écrite", vec!["perso".into()]).unwrap();
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "écrite"}}});
        let reply = k.mcp(&req).unwrap();
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("note écrite"));
    }

    /// A stand-in voice server: answers raw audio bytes with a content type.
    fn fake_tts(
        audio: &'static [u8],
        content_type: &'static str,
        status: &'static str,
    ) -> (String, Arc<Mutex<Vec<u8>>>) {
        let seen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let s2 = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
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
                s2.lock().unwrap().extend_from_slice(&req);
                let mut head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", audio.len()).into_bytes();
                head.extend_from_slice(audio);
                let _ = stream.write_all(&head);
            }
        });
        (format!("http://127.0.0.1:{port}/v1"), seen)
    }

    #[test]
    fn a_note_is_spoken_sealed_and_offered_over_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen) = fake_tts(b"ID3 fake mp3 audio", "audio/mpeg", "200 OK");
        let mut k = Kitsune::open(dir.path()).unwrap();
        k.set_settings(Settings {
            tts: TtsConfig {
                url,
                model: "kokoro".into(),
                voice: "ff_siwis".into(),
                format: "mp3".into(),
                timeout: 10,
            },
            mcp: McpConfig::default(),
            ..Default::default()
        })
        .unwrap();
        let note = k.capture_text("Bonne nuit", vec![]).unwrap();
        let (audio, mime, rel) = k.speak("Bonne nuit", Some(note)).unwrap();
        assert_eq!(
            (audio.as_slice(), mime.as_str()),
            (b"ID3 fake mp3 audio".as_slice(), "audio/mpeg")
        );
        assert!(rel.ends_with(".mp3.enc"));
        // sealed on disk, reads back, linked to the note as a tts audio with the voice kept
        assert_ne!(std::fs::read(dir.path().join(&rel)).unwrap(), audio);
        assert_eq!(k.vault().get(&rel).unwrap(), audio);
        let refs = k.store().audio_of(note).unwrap();
        assert_eq!(refs[0].kind, "tts");
        assert_eq!(refs[0].voice["voice"], "ff_siwis");
        // the request carried the model, voice and format
        let sent = String::from_utf8_lossy(&seen.lock().unwrap()).to_string();
        assert!(
            sent.contains("\"model\":\"kokoro\"")
                && sent.contains("\"voice\":\"ff_siwis\"")
                && sent.contains("\"response_format\":\"mp3\"")
        );
        // speaking an unknown note is refused
        assert!(k.speak("x", Some(999)).is_err());

        // over MCP: the tool is listed (tts ready) and calling it speaks
        let list = k
            .mcp(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"synthesize_speech"));
        let call = k.mcp(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "synthesize_speech", "arguments": {"text": "Salut"}}})).unwrap();
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Spoke it"));
    }

    #[test]
    fn without_a_voice_server_speaking_says_so_and_mcp_hides_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        let k = Kitsune::open(dir.path()).unwrap();
        assert!(k.speak("x", None).is_err());
        let list = k
            .mcp(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(
            !names.contains(&"synthesize_speech"),
            "no voice server: the tool is not offered"
        );
    }
}
