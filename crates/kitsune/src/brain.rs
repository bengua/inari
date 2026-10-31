//! The whole second brain behind one door: the store (`brain.db`), the sealed audio, and the local
//! servers it uses — voice-to-text, the small language model that reads a thought, the embedding
//! server, the geocoder, the voice. This is what a core holds and what the GUI configures; every
//! path and server address is a setting kept in `config.json`, never hard-coded.
//!
//! A thought goes one way, whatever brings it (the voice, the keyboard, an agent over MCP):
//! heard ([`asr`]) → read ([`understand`]: summary, category, tags, a time or a place) → placed
//! ([`geocode`]) → kept → given its meaning ([`embed`]). Each server is optional: without the
//! model the note is kept as said, without the geocoder a place waits (`pending_geocode`), without
//! embeddings it is found by its words. What waits is tried again by [`Kitsune::retry`].

use crate::asr::{self, AsrConfig};
use crate::embed::{self, EmbedConfig};
use crate::geocode::{self, GeocoderConfig, Outcome};
use crate::mcp::{Config as McpConfig, Memory};
use crate::search::{Mode, SearchOptions};
use crate::store::{AudioRef, NewNote, Note, Reminder};
use crate::tts::{self, TtsConfig};
use crate::understand::{self, SlmConfig};
use crate::{clock, AudioVault, Error, Key, Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The store's file under the brain's folder.
pub const DB_FILE: &str = "brain.db";

/// The tag a note gets when its place cannot be found.
pub const GEOCODE_ERROR_TAG: &str = "erreur:geocodage";

/// Everything the brain is told, from the GUI. Nothing here is assumed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub asr: AsrConfig,
    /// The small language model that reads a thought (empty: notes are kept as said).
    #[serde(default)]
    pub slm: SlmConfig,
    /// The embedding server that gives notes their meaning (empty: search by words only).
    #[serde(default)]
    pub embed: EmbedConfig,
    /// The geocoder that places "chez Julie, 12 rue Oberkampf" (empty: places wait).
    #[serde(default)]
    pub geocoder: GeocoderConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    /// What the MCP face offers the agentic network.
    #[serde(default)]
    pub mcp: McpConfig,
    /// How reminders ring.
    #[serde(default)]
    pub rings: crate::rings::RingConfig,
    /// The live voice (LiveKit and its voice agent): off until a LiveKit server is set.
    #[serde(default)]
    pub voice: VoiceConfig,
}

/// Talking with Kitsune live: the page joins a LiveKit room (the person's own LiveKit server), and
/// the voice agent (`services/kitsune-voice`) answers there, either on Gemini Live (the cloud, the
/// person's choice: their voice goes to Google) or privately (the local speech-to-text, the
/// standard tier, a local voice such as Kokoro). Nothing here is assumed: every field is a setting.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// The LiveKit server as the page reaches it (ws://host:7880 or wss://…); empty: no live voice.
    pub url: String,
    pub api_key: String,
    pub api_secret: String,
    /// gemini (realtime, in the cloud) or private (everything on the person's own servers).
    pub mode: String,
    /// The Gemini Live model, as Google names it (a setting: names change; empty: the plugin's default).
    pub model: String,
    /// The voice the model speaks with.
    pub voice: String,
    /// The language it speaks (fr-FR…).
    pub language: String,
    /// The tools the voice agent may use with the brain (the kitsune tool's actions).
    pub tools: Vec<String>,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            api_key: String::new(),
            api_secret: String::new(),
            mode: "private".into(),
            // the free Live model on the Gemini API (empty would fall back to the plugin's 2.5)
            model: "gemini-3.1-flash-live-preview".into(),
            voice: String::new(),
            language: "fr-FR".into(),
            tools: ["note", "reminders", "lists", "items", "add_items", "check"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl VoiceConfig {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty() && !self.api_key.is_empty() && !self.api_secret.is_empty()
    }
}

/// A thought to keep, and where it comes from.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    pub raw: String,
    /// voice, text or mcp.
    pub source: String,
    pub tags: Vec<String>,
    /// The place typed or said beside the thought ("kitchen"), kept as said.
    pub place: String,
    /// Where the person is, when the phone says (picks the nearest of several places).
    pub near: Option<(f64, f64)>,
    pub lang: String,
}

/// What a round of [`Kitsune::retry`] did.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Retried {
    pub transcribed: usize,
    pub placed: usize,
    pub not_placed: usize,
    pub embedded: usize,
}

pub struct Kitsune {
    dir: PathBuf,
    store: Store,
    vault: AudioVault,
    settings: Settings,
    /// The person's last known position (from the phone), for choosing between places.
    last_seen: Mutex<Option<(f64, f64)>>,
}

impl Kitsune {
    /// Open (or create) a brain under `dir`: the store at `dir/brain.db` (an earlier
    /// `kitsune.db` is taken over), the audio sealed under `dir/audio/` with the key at
    /// `dir/audio.key`, the settings at `dir/config.json`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(e.to_string()))?;
        let db = dir.join(DB_FILE);
        let old = dir.join("kitsune.db");
        if !db.exists() && old.exists() {
            for ext in ["", "-wal", "-shm"] {
                let from = PathBuf::from(format!("{}{ext}", old.display()));
                if from.exists() {
                    std::fs::rename(&from, format!("{}{ext}", db.display()))
                        .map_err(|e| Error::Invalid(e.to_string()))?;
                }
            }
        }
        let store = Store::open(&db)?;
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
            last_seen: Mutex::new(None),
        })
    }

    /// The one brain of a folder, shared by everyone in this process who opens it (the gateway's
    /// pages, the local engine's agents): one store, one set of settings, kept while anyone holds it.
    pub fn shared(dir: &Path) -> Result<std::sync::Arc<Mutex<Kitsune>>> {
        use std::collections::HashMap;
        use std::sync::{Arc, OnceLock, Weak};
        static OPEN: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<Kitsune>>>>> = OnceLock::new();
        std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(e.to_string()))?;
        let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let mut open = OPEN
            .get_or_init(Mutex::default)
            .lock()
            .expect("not poisoned");
        if let Some(k) = open.get(&key).and_then(Weak::upgrade) {
            return Ok(k);
        }
        let k = Arc::new(Mutex::new(Self::open(&key)?));
        open.insert(key, Arc::downgrade(&k));
        Ok(k)
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_settings(&mut self, s: Settings) -> Result<()> {
        std::fs::write(
            self.dir.join("config.json"),
            serde_json::to_vec_pretty(&s).expect("serializes"),
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
        self.settings = s;
        Ok(())
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn vault(&self) -> &AudioVault {
        &self.vault
    }

    /// The phone says where the person is (passively, when it knows): kept to choose between
    /// places with the same name. Never asked for, never polled.
    pub fn seen_at(&self, lat: f64, lon: f64) {
        if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
            *self.last_seen.lock().expect("not poisoned") = Some((lat, lon));
        }
    }

    fn near(&self, near: Option<(f64, f64)>) -> Option<(f64, f64)> {
        if let Some((lat, lon)) = near {
            self.seen_at(lat, lon);
        }
        near.or(*self.last_seen.lock().expect("not poisoned"))
    }

    /// Read a thought into a note: the model's summary, category, tags and reminder, the place
    /// found. Without a model (or when it fails) the note is kept as said.
    fn read_into(&self, n: &mut NewNote, near: Option<(f64, f64)>) {
        if !self.settings.slm.ready() {
            return;
        }
        let Ok(r) = understand::read(&self.settings.slm, &n.raw_transcript, clock::now()) else {
            return;
        };
        n.clean_summary = r.clean_summary;
        n.category = r.category;
        n.tags.extend(r.tags);
        match r.trigger_type.as_str() {
            "time" => {
                if let Ok(due) =
                    understand::resolve_due(&r.when, &self.settings.slm.day_parts, clock::now())
                {
                    n.trigger_type = "time".into();
                    n.due_date = Some(due);
                }
            }
            "geofence" => {
                n.location_raw = r.place;
                self.place_into(n, near);
            }
            _ => {}
        }
    }

    /// Find the place of a geofence note: found, not found, ambiguous, or waiting.
    fn place_into(&self, n: &mut NewNote, near: Option<(f64, f64)>) {
        match geocode::resolve(&self.settings.geocoder, &n.location_raw, near) {
            Outcome::Found(c) => {
                n.trigger_type = "geofence".into();
                n.latitude = Some(c.lat);
                n.longitude = Some(c.lon);
                n.radius_meters = self.settings.geocoder.radius_meters.max(1);
            }
            Outcome::NotFound => {
                n.trigger_type = "none".into();
                n.status = "failed_geocode".into();
                n.tags.push(GEOCODE_ERROR_TAG.into());
            }
            Outcome::Ambiguous(_) => {
                n.trigger_type = "none".into();
                n.status = "ambiguous_geocode".into();
            }
            Outcome::Unreachable(_) => {
                n.trigger_type = "none".into();
                n.status = "pending_geocode".into();
            }
        }
    }

    /// Give a note its meaning vector; quietly nothing when no embedding server answers (the
    /// note is found by its words, and [`Kitsune::retry`] embeds it later).
    fn embed_note(&self, id: i64, summary: &str) -> bool {
        if !self.settings.embed.ready() {
            return false;
        }
        let text = format!("{}{summary}", self.settings.embed.passage_prefix);
        match embed::embed(&self.settings.embed, &text) {
            Ok(v) => self.store.set_embedding(id, &v).is_ok(),
            Err(_) => false,
        }
    }

    /// Keep a thought: read it, place it, keep it, give it its meaning.
    pub fn keep(&self, c: &Capture) -> Result<Note> {
        let near = self.near(c.near);
        let mut n = NewNote {
            raw_transcript: c.raw.trim().into(),
            source: c.source.clone(),
            tags: c.tags.clone(),
            location_raw: c.place.trim().into(),
            lang: c.lang.clone(),
            ..Default::default()
        };
        self.read_into(&mut n, near);
        let id = self.store.insert_note(&n)?;
        let note = self.store.get_note(id)?.expect("just kept");
        self.embed_note(id, &note.clean_summary);
        Ok(note)
    }

    /// Keep a note from words, read the way every note is read.
    pub fn note_from_words(&self, raw: &str, source: &str, tags: Vec<String>) -> Result<Note> {
        self.keep(&Capture {
            raw: raw.into(),
            source: source.into(),
            tags,
            ..Default::default()
        })
    }

    /// Keep a written note.
    pub fn capture_text(&self, text: &str, tags: Vec<String>) -> Result<i64> {
        Ok(self.note_from_words(text, "text", tags)?.id)
    }

    /// A voice capture, start to end: transcribe it, seal the audio, keep the note linked to it.
    /// A capture heard too poorly is refused ([`Error::Inaudible`]) and nothing is kept. When the
    /// server cannot be reached, the audio is kept and the note waits for its transcription
    /// (`pending_transcription`), so nothing the person said is ever lost.
    pub fn ingest_voice(&self, wav: &[u8], place: &str, at: Option<(f64, f64)>) -> Result<i64> {
        let heard = asr::transcribe(&self.settings.asr, wav);
        if let Err(Error::Inaudible(why)) = heard {
            return Err(Error::Inaudible(why));
        }
        let rel = self.vault.put(wav, "wav")?;
        let note_id = match heard {
            Ok(t) => {
                let lang = if t.language.is_empty() {
                    self.settings.asr.language.clone()
                } else {
                    t.language
                };
                self.keep(&Capture {
                    raw: t.text,
                    source: "voice".into(),
                    place: place.into(),
                    near: at,
                    lang,
                    ..Default::default()
                })?
                .id
            }
            Err(_) => {
                self.near(at);
                self.store.insert_note(&NewNote {
                    source: "voice".into(),
                    status: "pending_transcription".into(),
                    location_raw: place.into(),
                    ..Default::default()
                })?
            }
        };
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

    /// The owner writes what was said: a note that waited for its words is read like a new one.
    pub fn correct_note(&self, id: i64, text: &str) -> Result<bool> {
        let Some(before) = self.store.get_note(id)? else {
            return Ok(false);
        };
        if before.status == "pending_transcription" {
            self.store.set_transcript(id, text, &before.lang)?;
            self.reread(id)?;
        } else {
            self.store.correct_note(id, text)?;
        }
        let after = self.store.get_note(id)?.expect("still there");
        self.embed_note(id, &after.clean_summary);
        Ok(true)
    }

    /// Read a kept note again (its words came later) and apply what the model found.
    fn reread(&self, id: i64) -> Result<()> {
        let Some(note) = self.store.get_note(id)? else {
            return Ok(());
        };
        let mut n = NewNote {
            raw_transcript: note.raw_transcript.clone(),
            clean_summary: note.clean_summary.clone(),
            tags: note.tags.clone(),
            location_raw: note.location_raw.clone(),
            ..Default::default()
        };
        self.read_into(&mut n, self.near(None));
        self.store
            .set_reading(id, &n.clean_summary, &n.category, &n.tags)?;
        match (n.trigger_type.as_str(), n.latitude, n.longitude, n.due_date) {
            ("time", _, _, Some(due_date)) => {
                self.store.set_reminder(id, &Reminder::Time { due_date })?;
            }
            ("geofence", Some(latitude), Some(longitude), _) => {
                self.store.set_reminder(
                    id,
                    &Reminder::Geofence {
                        latitude,
                        longitude,
                        radius_meters: n.radius_meters,
                    },
                )?;
            }
            _ => {}
        }
        if n.status != "active" {
            self.store.set_status(id, &n.status)?;
        }
        Ok(())
    }

    /// Try again what waited for a server: audio without words, places not yet found, notes
    /// without a meaning vector. Stops at the first server still out of reach; safe to call often.
    pub fn retry(&self) -> Result<Retried> {
        let mut done = Retried::default();
        if self.settings.asr.ready() {
            for n in self.store.pending_transcriptions()? {
                let Some(a) = self
                    .store
                    .audio_of(n.id)?
                    .into_iter()
                    .find(|a| a.kind == "capture")
                else {
                    continue;
                };
                let wav = self.vault.get(&a.path)?;
                match asr::transcribe(&self.settings.asr, &wav) {
                    Ok(t) => {
                        self.store.set_transcript(n.id, &t.text, &t.language)?;
                        self.reread(n.id)?;
                        let after = self.store.get_note(n.id)?.expect("still there");
                        self.embed_note(n.id, &after.clean_summary);
                        done.transcribed += 1;
                    }
                    // heard too poorly: it waits for the owner to write it
                    Err(Error::Inaudible(_)) => continue,
                    Err(_) => break,
                }
            }
        }
        if self.settings.geocoder.ready() {
            let near = self.near(None);
            for n in self.store.pending_geocode()? {
                match geocode::resolve(&self.settings.geocoder, &n.location_raw, near) {
                    Outcome::Found(c) => {
                        self.store.set_reminder(
                            n.id,
                            &Reminder::Geofence {
                                latitude: c.lat,
                                longitude: c.lon,
                                radius_meters: self.settings.geocoder.radius_meters.max(1),
                            },
                        )?;
                        done.placed += 1;
                    }
                    Outcome::NotFound => {
                        let mut tags = n.tags.clone();
                        tags.push(GEOCODE_ERROR_TAG.into());
                        self.store
                            .set_reading(n.id, &n.clean_summary, &n.category, &tags)?;
                        self.store.set_status(n.id, "failed_geocode")?;
                        done.not_placed += 1;
                    }
                    Outcome::Ambiguous(_) => {
                        self.store.set_status(n.id, "ambiguous_geocode")?;
                        done.not_placed += 1;
                    }
                    Outcome::Unreachable(_) => break,
                }
            }
        }
        if self.settings.embed.ready() {
            for n in self.store.notes_without_vector(100)? {
                if !self.embed_note(n.id, &n.clean_summary) {
                    break;
                }
                done.embedded += 1;
            }
        }
        Ok(done)
    }

    /// Say a text aloud: synthesise it on the voice server, seal the audio, keep it (linked to a
    /// note when one is given). The audio bytes to play now, its media type, and the stored path.
    pub fn speak(&self, text: &str, note_id: Option<i64>) -> Result<(Vec<u8>, String, String)> {
        if let Some(id) = note_id {
            if self.store.get_note(id)?.is_none() {
                return Err(Error::Invalid(format!("no note {id} to speak")));
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

    /// Search the notes as the search box and the MCP tool ask: by words and, when an embedding
    /// server answers, by meaning.
    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<crate::Hit>> {
        let vector =
            if opts.mode != Mode::Fts && self.settings.embed.ready() && !query.trim().is_empty() {
                let text = format!("{}{}", self.settings.embed.query_prefix, query.trim());
                embed::embed(&self.settings.embed, &text).ok()
            } else {
                None
            };
        self.store.search(query, vector.as_deref(), opts)
    }

    /// Answer one MCP request against the brain (the agentic network's door).
    pub fn mcp(&self, request: &Value) -> Option<Value> {
        crate::mcp::handle(self, &self.settings.mcp, request)
    }
}

/// The four MCP tools, done by the whole brain.
impl Memory for Kitsune {
    fn search_notes(&self, query: &str, mode: Mode, limit: usize) -> Result<Vec<Note>> {
        let opts = SearchOptions {
            mode,
            limit,
            ..Default::default()
        };
        Ok(self
            .search(query, &opts)?
            .into_iter()
            .map(|h| h.note)
            .collect())
    }
    fn get_active_reminders(&self, trigger_type: Option<&str>) -> Result<Vec<Note>> {
        self.store.active_reminders(trigger_type)
    }
    fn create_note(&self, raw_transcript: &str) -> Result<Note> {
        self.note_from_words(raw_transcript, "mcp", Vec::new())
    }
    fn dismiss_reminder(&self, note_id: i64) -> Result<bool> {
        self.store.dismiss(note_id)
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
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let id = k
            .ingest_voice(b"RIFF fake wav", "Cuisine", Some((48.85, 2.35)))
            .unwrap();
        assert_eq!(
            *k.last_seen.lock().unwrap(),
            Some((48.85, 2.35)),
            "the position is kept, not stored as a reminder"
        );
        let note = k.store().get_note(id).unwrap().unwrap();
        assert_eq!(
            (
                note.raw_transcript.as_str(),
                note.status.as_str(),
                note.lang.as_str()
            ),
            ("Acheter des piles", "active", "fr")
        );
        assert_eq!(
            (note.location_raw.as_str(), note.source.as_str()),
            ("Cuisine", "voice")
        );
        assert!(dir.path().join(DB_FILE).is_file(), "the store is brain.db");
        // found by word, accents folded
        assert_eq!(k.store().fts("piles", 5).unwrap()[0].0, id);
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
        assert_eq!(note.status, "pending_transcription");
        // the audio is still there, so nothing said is lost
        assert_eq!(k.store().audio_of(id).unwrap().len(), 1);
        // a correction later makes it active
        k.store().correct_note(id, "Rappeler le médecin").unwrap();
        assert_eq!(k.store().get_note(id).unwrap().unwrap().status, "active");
    }

    #[test]
    fn a_folder_has_one_brain_in_a_process() {
        let dir = tempfile::tempdir().unwrap();
        let a = Kitsune::shared(dir.path()).unwrap();
        let b = Kitsune::shared(dir.path()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
        let id = a.lock().unwrap().capture_text("partagé", vec![]).unwrap();
        assert!(b.lock().unwrap().store().get_note(id).unwrap().is_some());
    }

    #[test]
    fn the_mcp_door_is_the_whole_brain() {
        let dir = tempfile::tempdir().unwrap();
        let k = Kitsune::open(dir.path()).unwrap();
        k.capture_text("note écrite", vec!["perso".into()]).unwrap();
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "écrite"}}});
        let reply = k.mcp(&req).unwrap();
        assert_eq!(
            reply["result"]["structuredContent"]["notes"][0]["clean_summary"],
            "note écrite"
        );
        let made = k.mcp(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "create_note", "arguments": {"raw_transcript": "dicté par un agent"}}})).unwrap();
        assert_eq!(made["result"]["structuredContent"]["note"]["source"], "mcp");
    }

    #[test]
    fn an_earlier_kitsune_db_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        {
            let s = Store::open(&dir.path().join("kitsune.db")).unwrap();
            s.insert_note(&NewNote {
                raw_transcript: "ancienne note".into(),
                ..Default::default()
            })
            .unwrap();
        }
        let k = Kitsune::open(dir.path()).unwrap();
        assert!(!dir.path().join("kitsune.db").exists());
        assert_eq!(k.store().fts("ancienne", 3).unwrap().len(), 1);
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
    fn a_note_is_spoken_and_sealed() {
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

        // speech stays the owner's (the page's voice): the MCP face offers only the four tools
        let list = k
            .mcp(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn the_live_voice_defaults_to_the_free_gemini_live_model() {
        let v = VoiceConfig::default();
        assert_eq!(v.model, "gemini-3.1-flash-live-preview");
        assert_eq!(v.mode, "private", "the cloud stays an opt-in");
    }

    #[test]
    fn without_a_voice_server_speaking_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let k = Kitsune::open(dir.path()).unwrap();
        assert!(k.speak("x", None).is_err());
    }
}
