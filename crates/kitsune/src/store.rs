//! The store: notes, their audio, their vectors and their triggers, in one SQLite file.

use crate::{migrate, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub(crate) fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A note to keep.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NewNote {
    pub text: String,
    /// voice, text or mcp.
    pub source: String,
    pub needs_review: bool,
    pub lang: String,
    pub tags: Vec<String>,
    pub entities: Value,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub place: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub created_at: f64,
    pub updated_at: f64,
    pub text: String,
    pub source: String,
    pub status: String,
    pub lang: String,
    pub tags: Vec<String>,
    pub entities: Value,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub place: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRef {
    pub id: i64,
    pub kind: String,
    pub note_id: Option<i64>,
    pub path: String,
    pub mime: String,
    pub duration_ms: i64,
    pub bytes: i64,
    pub sha256: String,
    pub voice: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub id: i64,
    pub kind: String,
    pub note_id: Option<i64>,
    pub spec: Value,
    pub next_at: Option<f64>,
    pub active: bool,
    pub fired_at: Option<f64>,
}

const NOTE_COLUMNS: &str =
    "id, created_at, updated_at, text, source, status, lang, tags, entities, lat, lon, place";

fn note_of(r: &Row) -> rusqlite::Result<Note> {
    let tags: String = r.get(7)?;
    let entities: String = r.get(8)?;
    Ok(Note {
        id: r.get(0)?,
        created_at: r.get(1)?,
        updated_at: r.get(2)?,
        text: r.get(3)?,
        source: r.get(4)?,
        status: r.get(5)?,
        lang: r.get(6)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        entities: serde_json::from_str(&entities).unwrap_or(Value::Null),
        lat: r.get(9)?,
        lon: r.get(10)?,
        place: r.get(11)?,
    })
}

/// A query typed by a person, made safe for FTS5: each word a phrase, all of them required.
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) a store and bring its schema to the newest.
    pub fn open(path: &Path) -> Result<Self> {
        Self::ready(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::ready(Connection::open_in_memory()?)
    }

    fn ready(mut conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        migrate::migrate_to(&mut conn, migrate::LATEST)?;
        Ok(Self { conn })
    }

    pub fn capture_note(&self, n: &NewNote) -> Result<i64> {
        if n.text.trim().is_empty() && !n.needs_review {
            return Err(Error::Invalid(
                "an empty note is kept only to be reviewed".into(),
            ));
        }
        let at = now();
        let entities = if n.entities.is_null() {
            "{}".to_string()
        } else {
            serde_json::to_string(&n.entities)?
        };
        self.conn.execute(
            "INSERT INTO notes (created_at, updated_at, text, source, status, lang, tags, entities, lat, lon, place)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                at,
                n.text.trim(),
                if n.source.is_empty() { "text" } else { &n.source },
                if n.needs_review { "needs_review" } else { "ok" },
                n.lang,
                serde_json::to_string(&n.tags)?,
                entities,
                n.lat,
                n.lon,
                n.place,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_note(&self, id: i64) -> Result<Option<Note>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"),
                [id],
                note_of,
            )
            .optional()?)
    }

    /// Change a note's text (a correction after review): it is ok again.
    pub fn correct_note(&self, id: i64, text: &str) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE notes SET text = ?2, status = 'ok', updated_at = ?3 WHERE id = ?1",
            params![id, text.trim(), now()],
        )? == 1)
    }

    pub fn delete_note(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])? == 1)
    }

    /// The notes that match words, best first (BM25); accents do not matter.
    pub fn search_text(&self, query: &str, k: usize) -> Result<Vec<(Note, f64)>> {
        let q = fts_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self.conn.prepare(&format!(
            "SELECT {}, bm25(notes_fts) AS rank FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid
             WHERE notes_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            NOTE_COLUMNS
                .split(", ")
                .map(|c| format!("n.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let rows = st.query_map(params![q, k as i64], |r| {
            Ok((note_of(r)?, r.get::<_, f64>(12)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The notes of a time range, oldest first.
    pub fn notes_between(&self, start: f64, end: f64) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE created_at >= ?1 AND created_at < ?2 ORDER BY created_at"
        ))?;
        let rows = st.query_map(params![start, end], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Keep a note's vector for a model (replacing the one it had).
    pub fn set_embedding(&self, note_id: i64, model: &str, vector: &[f32]) -> Result<()> {
        if model.is_empty() || vector.is_empty() {
            return Err(Error::Invalid(
                "an embedding has its model and its values".into(),
            ));
        }
        let blob: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings (note_id, model, dim, vector) VALUES (?1, ?2, ?3, ?4)",
            params![note_id, model, vector.len() as i64, blob],
        )?;
        Ok(())
    }

    pub fn embedding(&self, note_id: i64, model: &str) -> Result<Option<Vec<f32>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT vector FROM embeddings WHERE note_id = ?1 AND model = ?2",
                params![note_id, model],
                |r| r.get(0),
            )
            .optional()?;
        Ok(blob.map(|b| {
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect()
        }))
    }

    pub fn add_audio(&self, a: &AudioRef) -> Result<i64> {
        if a.path.is_empty() || a.path.starts_with('/') || a.path.contains("..") {
            return Err(Error::Invalid(format!(
                "not a path inside the store: {:?}",
                a.path
            )));
        }
        self.conn.execute(
            "INSERT INTO audio_refs (created_at, kind, note_id, path, mime, duration_ms, bytes, sha256, voice)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![now(), a.kind, a.note_id, a.path, a.mime, a.duration_ms, a.bytes, a.sha256, serde_json::to_string(&a.voice)?],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn audio_of(&self, note_id: i64) -> Result<Vec<AudioRef>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, note_id, path, mime, duration_ms, bytes, sha256, voice FROM audio_refs WHERE note_id = ?1 ORDER BY id",
        )?;
        let rows = st.query_map([note_id], |r| {
            let voice: String = r.get(8)?;
            Ok(AudioRef {
                id: r.get(0)?,
                kind: r.get(1)?,
                note_id: r.get(2)?,
                path: r.get(3)?,
                mime: r.get(4)?,
                duration_ms: r.get(5)?,
                bytes: r.get(6)?,
                sha256: r.get(7)?,
                voice: serde_json::from_str(&voice).unwrap_or(Value::Null),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_trigger(
        &self,
        kind: &str,
        note_id: Option<i64>,
        spec: &Value,
        next_at: Option<f64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO triggers (created_at, kind, note_id, spec, next_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now(), kind, note_id, serde_json::to_string(spec)?, next_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_triggers(&self) -> Result<Vec<Trigger>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, note_id, spec, next_at, active, fired_at FROM triggers ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            let spec: String = r.get(3)?;
            Ok(Trigger {
                id: r.get(0)?,
                kind: r.get(1)?,
                note_id: r.get(2)?,
                spec: serde_json::from_str(&spec).unwrap_or(Value::Null),
                next_at: r.get(4)?,
                active: r.get::<_, i64>(5)? != 0,
                fired_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_trigger(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM triggers WHERE id = ?1", [id])?
            == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn note(text: &str) -> NewNote {
        NewNote {
            text: text.into(),
            source: "voice".into(),
            lang: "fr".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_note_is_kept_found_corrected_and_removed() {
        let s = Store::open_in_memory().unwrap();
        let a = s
            .capture_note(&note("Rappeler le garagiste pour la révision"))
            .unwrap();
        let b = s
            .capture_note(&NewNote {
                tags: vec!["courses".into()],
                place: "Marché de Lyon".into(),
                ..note("Acheter du pain et des œufs")
            })
            .unwrap();
        let got = s.get_note(b).unwrap().unwrap();
        assert_eq!(
            (got.tags.clone(), got.status.as_str()),
            (vec!["courses".to_string()], "ok")
        );
        // accents folded, BM25 order, tags and places indexed too
        assert_eq!(s.search_text("revision", 5).unwrap()[0].0.id, a);
        assert_eq!(s.search_text("courses", 5).unwrap()[0].0.id, b);
        assert_eq!(s.search_text("lyon", 5).unwrap()[0].0.id, b);
        assert!(s
            .search_text("\"; DROP TABLE notes; --", 5)
            .unwrap()
            .is_empty());
        // a correction is indexed in its place
        assert!(s.correct_note(a, "Rappeler la carrosserie").unwrap());
        assert!(s.search_text("garagiste", 5).unwrap().is_empty());
        assert_eq!(s.search_text("carrosserie", 5).unwrap()[0].0.id, a);
        assert!(s.delete_note(a).unwrap());
        assert!(s.search_text("carrosserie", 5).unwrap().is_empty());
        assert!(s.get_note(a).unwrap().is_none());
    }

    #[test]
    fn a_failed_transcription_is_kept_for_review() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.capture_note(&note("  ")).is_err());
        let id = s
            .capture_note(&NewNote {
                needs_review: true,
                ..note("")
            })
            .unwrap();
        assert_eq!(s.get_note(id).unwrap().unwrap().status, "needs_review");
        s.correct_note(id, "Appeler maman").unwrap();
        assert_eq!(s.get_note(id).unwrap().unwrap().status, "ok");
    }

    #[test]
    fn vectors_audio_and_triggers_follow_their_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kitsune.db");
        let id = {
            let s = Store::open(&path).unwrap();
            let id = s.capture_note(&note("Réunion jeudi")).unwrap();
            s.set_embedding(id, "minilm-l12", &[0.25, -1.0, 3.5])
                .unwrap();
            s.add_audio(&AudioRef {
                id: 0,
                kind: "capture".into(),
                note_id: Some(id),
                path: "audio/1.wav.enc".into(),
                mime: "audio/wav".into(),
                duration_ms: 1800,
                bytes: 57_600,
                sha256: String::new(),
                voice: json!({}),
            })
            .unwrap();
            assert!(s
                .add_audio(&AudioRef {
                    path: "../x".into(),
                    ..s.audio_of(id).unwrap()[0].clone()
                })
                .is_err());
            s.create_trigger(
                "time",
                Some(id),
                &json!({"at": 1_800_000_000.0}),
                Some(1_800_000_000.0),
            )
            .unwrap();
            id
        };
        // reopened: the schema is not applied twice, everything is there
        let s = Store::open(&path).unwrap();
        assert_eq!(
            s.embedding(id, "minilm-l12").unwrap().unwrap(),
            vec![0.25, -1.0, 3.5]
        );
        assert_eq!(s.audio_of(id).unwrap()[0].duration_ms, 1800);
        assert_eq!(s.list_triggers().unwrap()[0].spec["at"], 1_800_000_000.0);
        assert!(
            s.create_trigger("whenever", None, &json!({}), None)
                .is_err(),
            "unknown kind"
        );
        // removing the note takes its vector and trigger, keeps its audio reference unlinked
        s.delete_note(id).unwrap();
        assert!(s.embedding(id, "minilm-l12").unwrap().is_none());
        assert!(s.list_triggers().unwrap().is_empty());
    }
}
