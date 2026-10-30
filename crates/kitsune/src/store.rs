//! The store: `brain.db`, one SQLite file. A note carries what was said, what it means, its
//! category and tags, and its own reminder (a time or a place); its words (FTS5), its meaning
//! (vec0, 384 floats) and its place (R*Tree) are indexed beside it, and its audio is referenced.

use crate::search::haversine_m;
use crate::{clock, migrate, vecext, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// The size of a note's meaning vector (the embedding model the person sets must give it).
pub const DIM: usize = 384;

/// The default radius of a place reminder, in metres.
pub const RADIUS_M: i64 = 100;

/// The statuses a note may have.
pub const STATUSES: [&str; 7] = [
    "active",
    "dismissed",
    "failed",
    "pending_transcription",
    "pending_geocode",
    "failed_geocode",
    "ambiguous_geocode",
];

/// A note to keep.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NewNote {
    pub raw_transcript: String,
    /// What it means, said shortly; the transcript itself when empty.
    pub clean_summary: String,
    pub category: String,
    pub tags: Vec<String>,
    /// none, time or geofence.
    pub trigger_type: String,
    /// ISO 8601 UTC, for a time reminder.
    pub due_date: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub radius_meters: i64,
    pub status: String,
    /// The place as said, to geocode it (again) later.
    pub location_raw: String,
    /// voice, text or mcp.
    pub source: String,
    pub lang: String,
}

impl Default for NewNote {
    fn default() -> Self {
        Self {
            raw_transcript: String::new(),
            clean_summary: String::new(),
            category: "memo".into(),
            tags: Vec::new(),
            trigger_type: "none".into(),
            due_date: None,
            latitude: None,
            longitude: None,
            radius_meters: RADIUS_M,
            status: "active".into(),
            location_raw: String::new(),
            source: "text".into(),
            lang: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub raw_transcript: String,
    pub clean_summary: String,
    pub category: String,
    pub tags: Vec<String>,
    pub trigger_type: String,
    pub due_date: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub radius_meters: i64,
    pub status: String,
    pub created_at: String,
    pub location_raw: String,
    pub source: String,
    pub lang: String,
    /// When its time reminder last rang.
    pub notified_at: Option<String>,
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

/// A note's reminder, set or changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "trigger_type", rename_all = "lowercase")]
pub enum Reminder {
    None,
    Time {
        due_date: String,
    },
    Geofence {
        latitude: f64,
        longitude: f64,
        #[serde(default = "radius")]
        radius_meters: i64,
    },
}

fn radius() -> i64 {
    RADIUS_M
}

pub(crate) const NOTE_COLUMNS: &str = "id, raw_transcript, clean_summary, category, tags, \
    trigger_type, due_date, latitude, longitude, radius_meters, status, created_at, location_raw, \
    source, lang, notified_at";

pub(crate) fn note_of(r: &Row) -> rusqlite::Result<Note> {
    let tags: String = r.get(4)?;
    Ok(Note {
        id: r.get(0)?,
        raw_transcript: r.get(1)?,
        clean_summary: r.get(2)?,
        category: r.get(3)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        trigger_type: r.get(5)?,
        due_date: r.get(6)?,
        latitude: r.get(7)?,
        longitude: r.get(8)?,
        radius_meters: r.get(9)?,
        status: r.get(10)?,
        created_at: r.get(11)?,
        location_raw: r.get(12)?,
        source: r.get(13)?,
        lang: r.get(14)?,
        notified_at: r.get(15)?,
    })
}

/// A query typed by a person, made safe for FTS5: each word a phrase, all of them required.
pub(crate) fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The bounding box of a circle on the earth: (min_lat, max_lat, min_lon, max_lon).
pub(crate) fn bbox(lat: f64, lon: f64, radius_m: f64) -> (f64, f64, f64, f64) {
    const M_PER_DEG: f64 = 111_320.0;
    let dlat = radius_m / M_PER_DEG;
    // near the poles a degree of longitude shrinks to nothing: cap the box at the whole circle
    let cos = lat.to_radians().cos().abs().max(1e-6);
    let dlon = (radius_m / (M_PER_DEG * cos)).min(180.0);
    (lat - dlat, lat + dlat, lon - dlon, lon + dlon)
}

fn valid_coords(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
}

/// A JSON array of clean tags (trimmed, deduplicated, no empty one).
fn tags_json(tags: &[String]) -> Result<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.chars().take(64).collect());
        }
    }
    Ok(serde_json::to_string(&out)?)
}

pub struct Store {
    pub(crate) conn: Connection,
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
        // WAL: readers never wait for the writer; busy_timeout: a writer waits its turn up to 5 s
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
        vecext::load(&conn)?;
        migrate::migrate_to(&mut conn, migrate::LATEST)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let s = Self { conn };
        s.reindex_places_if_needed()?;
        Ok(s)
    }

    /// The place index holds every note with coordinates and nothing else (after a migration, or a
    /// write made outside this store).
    fn reindex_places_if_needed(&self) -> Result<()> {
        let with: i64 = self.conn.query_row(
            "SELECT count(*) FROM notes WHERE latitude IS NOT NULL AND longitude IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        let indexed: i64 = self
            .conn
            .query_row("SELECT count(*) FROM notes_geo", [], |r| r.get(0))?;
        if with == indexed {
            return Ok(());
        }
        self.conn.execute("DELETE FROM notes_geo", [])?;
        let mut st = self.conn.prepare(
            "SELECT id, latitude, longitude, radius_meters FROM notes
             WHERE latitude IS NOT NULL AND longitude IS NOT NULL",
        )?;
        let rows: Vec<(i64, f64, f64, i64)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, lat, lon, rad) in rows {
            self.index_place(id, Some(lat), Some(lon), rad)?;
        }
        Ok(())
    }

    /// Put a note's place in the R*Tree (or take it out when it has none).
    fn index_place(
        &self,
        id: i64,
        lat: Option<f64>,
        lon: Option<f64>,
        radius_m: i64,
    ) -> Result<()> {
        self.conn
            .execute("DELETE FROM notes_geo WHERE id = ?1", [id])?;
        if let (Some(lat), Some(lon)) = (lat, lon) {
            let (a, b, c, d) = bbox(lat, lon, radius_m as f64);
            self.conn.execute(
                "INSERT INTO notes_geo (id, min_lat, max_lat, min_lon, max_lon) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, a, b, c, d],
            )?;
        }
        Ok(())
    }

    fn check(n: &NewNote) -> Result<()> {
        if !STATUSES.contains(&n.status.as_str()) {
            return Err(Error::Invalid(format!("unknown status {:?}", n.status)));
        }
        if !["voice", "text", "mcp"].contains(&n.source.as_str()) {
            return Err(Error::Invalid(format!("unknown source {:?}", n.source)));
        }
        if n.raw_transcript.trim().is_empty() && n.status != "pending_transcription" {
            return Err(Error::Invalid(
                "a note has words, unless it waits for its transcription".into(),
            ));
        }
        if n.radius_meters <= 0 {
            return Err(Error::Invalid(
                "a place's radius is a positive number of metres".into(),
            ));
        }
        if let (Some(lat), Some(lon)) = (n.latitude, n.longitude) {
            if !valid_coords(lat, lon) {
                return Err(Error::Invalid(format!(
                    "not a place on earth: {lat}, {lon}"
                )));
            }
        }
        match n.trigger_type.as_str() {
            "none" => {}
            "time" => {
                if n.due_date.as_deref().and_then(clock::parse).is_none() {
                    return Err(Error::Invalid(
                        "a time reminder needs its due date (ISO 8601, UTC)".into(),
                    ));
                }
            }
            "geofence" => {
                let placed = n.latitude.is_some() && n.longitude.is_some();
                if !placed && n.status != "pending_geocode" {
                    return Err(Error::Invalid(
                        "a place reminder needs its coordinates, or waits for them".into(),
                    ));
                }
            }
            other => return Err(Error::Invalid(format!("unknown trigger type {other:?}"))),
        }
        Ok(())
    }

    /// Keep a note; its id.
    pub fn insert_note(&self, n: &NewNote) -> Result<i64> {
        Self::check(n)?;
        let summary = if n.clean_summary.trim().is_empty() {
            n.raw_transcript.trim()
        } else {
            n.clean_summary.trim()
        };
        let category = if n.category.trim().is_empty() {
            "memo"
        } else {
            n.category.trim()
        };
        let due = n.due_date.as_deref().and_then(clock::parse).map(clock::iso);
        self.conn.execute(
            "INSERT INTO notes (raw_transcript, clean_summary, category, tags, trigger_type, due_date,
                                latitude, longitude, radius_meters, status, location_raw, source, lang)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                n.raw_transcript.trim(),
                summary,
                category,
                tags_json(&n.tags)?,
                n.trigger_type,
                due,
                n.latitude,
                n.longitude,
                n.radius_meters,
                n.status,
                n.location_raw.trim(),
                n.source,
                n.lang,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.index_place(id, n.latitude, n.longitude, n.radius_meters)?;
        Ok(id)
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

    pub(crate) fn notes_by_ids(&self, ids: &[i64]) -> Result<Vec<Note>> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(n) = self.get_note(*id)? {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// Change what a note says (a correction after review): it is active again.
    pub fn correct_note(&self, id: i64, text: &str) -> Result<bool> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("a correction has words".into()));
        }
        Ok(self.conn.execute(
            "UPDATE notes SET clean_summary = ?2,
                    raw_transcript = CASE raw_transcript WHEN '' THEN ?2 ELSE raw_transcript END,
                    status = CASE status WHEN 'pending_transcription' THEN 'active' ELSE status END
             WHERE id = ?1",
            params![id, text],
        )? == 1)
    }

    /// Fill in a note the small model has read: its summary, category and tags.
    pub fn set_reading(
        &self,
        id: i64,
        summary: &str,
        category: &str,
        tags: &[String],
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE notes SET clean_summary = ?2, category = ?3, tags = ?4 WHERE id = ?1",
            params![id, summary.trim(), category.trim(), tags_json(tags)?],
        )? == 1)
    }

    /// Set (or take away) a note's reminder.
    pub fn set_reminder(&self, id: i64, r: &Reminder) -> Result<bool> {
        if self.get_note(id)?.is_none() {
            return Ok(false);
        }
        match r {
            Reminder::None => {
                self.conn.execute(
                    "UPDATE notes SET trigger_type = 'none', due_date = NULL, notified_at = NULL WHERE id = ?1",
                    [id],
                )?;
            }
            Reminder::Time { due_date } => {
                let due = clock::parse(due_date).ok_or_else(|| {
                    Error::Invalid(format!("not an ISO 8601 UTC time: {due_date:?}"))
                })?;
                self.conn.execute(
                    "UPDATE notes SET trigger_type = 'time', due_date = ?2, notified_at = NULL,
                            status = CASE status WHEN 'dismissed' THEN 'active' ELSE status END
                     WHERE id = ?1",
                    params![id, clock::iso(due)],
                )?;
            }
            Reminder::Geofence {
                latitude,
                longitude,
                radius_meters,
            } => {
                if !valid_coords(*latitude, *longitude) || *radius_meters <= 0 {
                    return Err(Error::Invalid("not a place on earth, or no radius".into()));
                }
                self.conn.execute(
                    "UPDATE notes SET trigger_type = 'geofence', latitude = ?2, longitude = ?3,
                            radius_meters = ?4,
                            status = CASE WHEN status IN ('dismissed', 'pending_geocode',
                                     'failed_geocode', 'ambiguous_geocode') THEN 'active' ELSE status END
                     WHERE id = ?1",
                    params![id, latitude, longitude, radius_meters],
                )?;
                self.index_place(id, Some(*latitude), Some(*longitude), *radius_meters)?;
            }
        }
        Ok(true)
    }

    /// Give a waiting note the words its audio held (a transcription that came later).
    pub fn set_transcript(&self, id: i64, raw: &str, lang: &str) -> Result<bool> {
        if raw.trim().is_empty() {
            return Err(Error::Invalid("a transcript has words".into()));
        }
        Ok(self.conn.execute(
            "UPDATE notes SET raw_transcript = ?2, lang = ?3,
                    clean_summary = CASE clean_summary WHEN '' THEN ?2 ELSE clean_summary END,
                    status = CASE status WHEN 'pending_transcription' THEN 'active' ELSE status END
             WHERE id = ?1",
            params![id, raw.trim(), lang],
        )? == 1)
    }

    /// The notes whose audio still waits for its words, oldest first.
    pub fn pending_transcriptions(&self) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE status = 'pending_transcription' ORDER BY id"
        ))?;
        let rows = st.query_map([], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Notes with words but no meaning vector yet (the embedding server was away), oldest first.
    pub fn notes_without_vector(&self, limit: usize) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes
             WHERE clean_summary != '' AND id NOT IN (SELECT note_id FROM notes_vec)
             ORDER BY id LIMIT ?1"
        ))?;
        let rows = st.query_map([limit as i64], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_status(&self, id: i64, status: &str) -> Result<bool> {
        if !STATUSES.contains(&status) {
            return Err(Error::Invalid(format!("unknown status {status:?}")));
        }
        Ok(self.conn.execute(
            "UPDATE notes SET status = ?2 WHERE id = ?1",
            params![id, status],
        )? == 1)
    }

    /// A reminder done with: it stops ringing and leaves the active list.
    pub fn dismiss(&self, id: i64) -> Result<bool> {
        self.set_status(id, "dismissed")
    }

    pub fn delete_note(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])? == 1)
    }

    /// The newest notes first: what the memory shows when nobody typed a word.
    pub fn recent_notes(&self, limit: usize) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes ORDER BY created_at DESC, id DESC LIMIT ?1"
        ))?;
        let rows = st.query_map(params![limit as i64], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The notes of a time range (seconds since the epoch, end excluded), oldest first.
    pub fn notes_between(&self, start: i64, end: i64) -> Result<Vec<Note>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE created_at >= ?1 AND created_at < ?2
             ORDER BY created_at, id"
        ))?;
        let rows = st.query_map(params![clock::iso(start), clock::iso(end)], note_of)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count_notes(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?)
    }

    /// The notes that match words, best first, with their BM25 score (lower is better).
    pub fn fts(&self, query: &str, limit: usize) -> Result<Vec<(i64, f64)>> {
        let q = fts_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self.conn.prepare(
            "SELECT rowid, bm25(notes_fts) AS rank FROM notes_fts WHERE notes_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let rows = st.query_map(params![q, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Keep a note's meaning vector (DIM floats), replacing the one it had.
    pub fn set_embedding(&self, note_id: i64, vector: &[f32]) -> Result<()> {
        if vector.len() != DIM {
            return Err(Error::Invalid(format!(
                "a meaning vector has {DIM} values, not {}: set an embedding model that gives {DIM}",
                vector.len()
            )));
        }
        if vector.iter().any(|x| !x.is_finite()) {
            return Err(Error::Invalid("a meaning vector holds numbers only".into()));
        }
        let blob: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.conn
            .execute("DELETE FROM notes_vec WHERE note_id = ?1", [note_id])?;
        self.conn.execute(
            "INSERT INTO notes_vec (note_id, embedding) VALUES (?1, ?2)",
            params![note_id, blob],
        )?;
        Ok(())
    }

    pub fn embedding(&self, note_id: i64) -> Result<Option<Vec<f32>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT embedding FROM notes_vec WHERE note_id = ?1",
                [note_id],
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

    /// The notes nearest in meaning to a vector, nearest first, with their cosine distance.
    pub fn knn(&self, vector: &[f32], k: usize) -> Result<Vec<(i64, f64)>> {
        if vector.len() != DIM {
            return Err(Error::Invalid(format!("a query vector has {DIM} values")));
        }
        if vector.iter().all(|x| *x == 0.0) {
            return Err(Error::Invalid(
                "an empty query vector has no meaning".into(),
            ));
        }
        let blob: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        let mut st = self.conn.prepare(
            "SELECT note_id, distance FROM notes_vec WHERE embedding MATCH ?1 AND k = ?2
             ORDER BY distance",
        )?;
        // a stored vector of zeros has no cosine distance: it is left out
        let rows = st.query_map(params![blob, k.max(1) as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<f64>>(1)?))
        })?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|(id, d)| Some((id, d?)))
            .collect())
    }

    /// The notes whose place covers a point (R*Tree first, then the exact distance), nearest first.
    pub fn notes_covering(&self, lat: f64, lon: f64) -> Result<Vec<(Note, f64)>> {
        let mut st = self.conn.prepare(&format!(
            "SELECT {} FROM notes_geo g JOIN notes n ON n.id = g.id
             WHERE g.min_lat <= ?1 AND g.max_lat >= ?1 AND g.min_lon <= ?2 AND g.max_lon >= ?2",
            NOTE_COLUMNS
                .split(", ")
                .map(|c| format!("n.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let rows: Vec<Note> = st
            .query_map(params![lat, lon], note_of)?
            .collect::<rusqlite::Result<_>>()?;
        let mut out: Vec<(Note, f64)> = rows
            .into_iter()
            .filter_map(|n| {
                let d = haversine_m(n.latitude?, n.longitude?, lat, lon);
                (d <= n.radius_meters as f64).then_some((n, d))
            })
            .collect();
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        Ok(out)
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
            params![
                clock::now() as f64,
                a.kind,
                a.note_id,
                a.path,
                a.mime,
                a.duration_ms,
                a.bytes,
                a.sha256,
                serde_json::to_string(&a.voice)?
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn audio_of(&self, note_id: i64) -> Result<Vec<AudioRef>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, note_id, path, mime, duration_ms, bytes, sha256, voice FROM audio_refs
             WHERE note_id = ?1 ORDER BY id",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str) -> NewNote {
        NewNote {
            raw_transcript: text.into(),
            source: "voice".into(),
            lang: "fr".into(),
            ..Default::default()
        }
    }

    fn unit(seed: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; DIM];
        v[seed % DIM] = 1.0;
        v
    }

    #[test]
    fn a_note_is_kept_found_corrected_and_removed() {
        let s = Store::open_in_memory().unwrap();
        let a = s
            .insert_note(&note("Rappeler le garagiste pour la révision"))
            .unwrap();
        let b = s
            .insert_note(&NewNote {
                tags: vec!["courses".into(), " courses ".into(), "".into()],
                location_raw: "Marché de Lyon".into(),
                ..note("Acheter du pain et des œufs")
            })
            .unwrap();
        let got = s.get_note(b).unwrap().unwrap();
        assert_eq!(got.tags, vec!["courses".to_string()], "tags cleaned");
        assert_eq!(
            (
                got.status.as_str(),
                got.category.as_str(),
                got.trigger_type.as_str()
            ),
            ("active", "memo", "none")
        );
        assert_eq!(
            got.clean_summary, "Acheter du pain et des œufs",
            "the transcript stands in for the summary"
        );
        assert!(
            clock::parse(&got.created_at).is_some(),
            "{}",
            got.created_at
        );
        // accents folded, tags indexed
        assert_eq!(s.fts("revision", 5).unwrap()[0].0, a);
        assert_eq!(s.fts("courses", 5).unwrap()[0].0, b);
        assert!(s.fts("\"; DROP TABLE notes; --", 5).unwrap().is_empty());
        // a correction changes the summary (indexed at once); what was said stays as it was said
        assert!(s.correct_note(a, "Rappeler la carrosserie").unwrap());
        assert_eq!(s.fts("carrosserie", 5).unwrap()[0].0, a);
        let corrected = s.get_note(a).unwrap().unwrap();
        assert_eq!(corrected.clean_summary, "Rappeler la carrosserie");
        assert!(corrected.raw_transcript.contains("garagiste"));
        assert!(s.delete_note(a).unwrap());
        assert!(s.fts("carrosserie", 5).unwrap().is_empty());
        assert!(s.get_note(a).unwrap().is_none());
    }

    #[test]
    fn a_note_waiting_for_its_words_is_kept_then_corrected() {
        let s = Store::open_in_memory().unwrap();
        assert!(
            s.insert_note(&note("  ")).is_err(),
            "no words, not waiting: refused"
        );
        let id = s
            .insert_note(&NewNote {
                status: "pending_transcription".into(),
                ..note("")
            })
            .unwrap();
        assert_eq!(
            s.get_note(id).unwrap().unwrap().status,
            "pending_transcription"
        );
        s.correct_note(id, "Appeler maman").unwrap();
        let n = s.get_note(id).unwrap().unwrap();
        assert_eq!(
            (n.status.as_str(), n.raw_transcript.as_str()),
            ("active", "Appeler maman")
        );
    }

    #[test]
    fn reminders_are_checked_when_kept() {
        let s = Store::open_in_memory().unwrap();
        let bad_time = NewNote {
            trigger_type: "time".into(),
            due_date: Some("demain".into()),
            ..note("x")
        };
        assert!(s.insert_note(&bad_time).is_err());
        let no_place = NewNote {
            trigger_type: "geofence".into(),
            ..note("x")
        };
        assert!(s.insert_note(&no_place).is_err());
        let waiting = NewNote {
            trigger_type: "geofence".into(),
            status: "pending_geocode".into(),
            location_raw: "14 rue de Paris".into(),
            ..note("x")
        };
        assert!(
            s.insert_note(&waiting).is_ok(),
            "a place may wait for its coordinates"
        );
        let off_earth = NewNote {
            latitude: Some(91.0),
            longitude: Some(0.0),
            ..note("x")
        };
        assert!(s.insert_note(&off_earth).is_err());
        let t = s
            .insert_note(&NewNote {
                trigger_type: "time".into(),
                due_date: Some("2026-10-31 09:00:00".into()),
                ..note("x")
            })
            .unwrap();
        assert_eq!(
            s.get_note(t).unwrap().unwrap().due_date.as_deref(),
            Some("2026-10-31T09:00:00Z"),
            "stored as ISO 8601 UTC"
        );
    }

    // the brief's test 2: FTS5 and sqlite-vec stay in step with the notes, removal included
    #[test]
    fn words_and_vectors_follow_their_note_and_are_purged_with_it() {
        let s = Store::open_in_memory().unwrap();
        let id = s.insert_note(&note("Réunion de chantier jeudi")).unwrap();
        s.set_embedding(id, &unit(7)).unwrap();
        assert_eq!(
            s.fts("chantier", 5).unwrap()[0].0,
            id,
            "in notes_fts at once"
        );
        let near = s.knn(&unit(7), 1).unwrap();
        assert_eq!(near[0].0, id, "in notes_vec at once");
        assert!(near[0].1 < 1e-6, "cosine distance 0 to itself");
        assert!(
            s.set_embedding(id, &[1.0, 2.0]).is_err(),
            "only {DIM} values"
        );
        assert!(s.delete_note(id).unwrap());
        assert!(
            s.fts("chantier", 5).unwrap().is_empty(),
            "purged from notes_fts"
        );
        assert!(
            s.knn(&unit(7), 5).unwrap().is_empty(),
            "purged from notes_vec"
        );
        assert!(s.embedding(id).unwrap().is_none());
    }

    // the brief's test 3: the R*Tree finds the note when its radius covers the point, not 500 m off
    #[test]
    fn the_place_index_answers_within_the_radius_only() {
        let s = Store::open_in_memory().unwrap();
        let home = s
            .insert_note(&NewNote {
                trigger_type: "geofence".into(),
                latitude: Some(48.8566),
                longitude: Some(2.3522),
                radius_meters: 100,
                ..note("Code de la porte : 45B2")
            })
            .unwrap();
        let hit = s.notes_covering(48.8566, 2.3522).unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].0.id, home);
        // 60 m north: inside the 100 m circle
        assert_eq!(
            s.notes_covering(48.8566 + 60.0 / 111_320.0, 2.3522)
                .unwrap()
                .len(),
            1
        );
        // more than 500 m away: nothing
        assert!(s
            .notes_covering(48.8566 + 600.0 / 111_320.0, 2.3522)
            .unwrap()
            .is_empty());
        assert!(
            s.notes_covering(48.8566, 2.3522 + 0.01).unwrap().is_empty(),
            "~730 m east"
        );
        // the box corner is outside the circle: the exact distance decides
        let (_, max_lat, _, max_lon) = bbox(48.8566, 2.3522, 100.0);
        assert!(s
            .notes_covering(max_lat - 1e-7, max_lon - 1e-7)
            .unwrap()
            .is_empty());
        // moved, the index follows; removed, it is gone
        s.set_reminder(
            home,
            &Reminder::Geofence {
                latitude: 45.764,
                longitude: 4.8357,
                radius_meters: 150,
            },
        )
        .unwrap();
        assert!(s.notes_covering(48.8566, 2.3522).unwrap().is_empty());
        assert_eq!(s.notes_covering(45.764, 4.8357).unwrap().len(), 1);
        s.delete_note(home).unwrap();
        assert!(s.notes_covering(45.764, 4.8357).unwrap().is_empty());
    }

    #[test]
    fn audio_follows_its_note_and_the_store_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.db");
        let id = {
            let s = Store::open(&path).unwrap();
            let id = s.insert_note(&note("Réunion jeudi")).unwrap();
            s.set_embedding(id, &unit(3)).unwrap();
            s.add_audio(&AudioRef {
                id: 0,
                kind: "capture".into(),
                note_id: Some(id),
                path: "audio/1.wav.enc".into(),
                mime: "audio/wav".into(),
                duration_ms: 1800,
                bytes: 57_600,
                sha256: String::new(),
                voice: serde_json::json!({}),
            })
            .unwrap();
            assert!(s
                .add_audio(&AudioRef {
                    path: "../x".into(),
                    ..s.audio_of(id).unwrap()[0].clone()
                })
                .is_err());
            id
        };
        let s = Store::open(&path).unwrap();
        assert_eq!(s.embedding(id).unwrap().unwrap(), unit(3));
        assert_eq!(s.audio_of(id).unwrap()[0].duration_ms, 1800);
        let journal: String = s
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let busy: i64 = s
            .conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            (journal.as_str(), busy),
            ("wal", 5000),
            "WAL and a 5 s busy timeout"
        );
        s.delete_note(id).unwrap();
        assert!(s.audio_of(id).unwrap().is_empty() || s.audio_of(id).unwrap()[0].note_id.is_none());
    }

    #[test]
    fn earlier_notes_and_their_reminders_are_carried_into_brain_db() {
        // a store at schema 4 with a note, a time trigger, a place trigger and a 384 vector
        let mut conn = Connection::open_in_memory().unwrap();
        vecext::load(&conn).unwrap();
        migrate::migrate_to(&mut conn, 4).unwrap();
        conn.execute_batch(
            "INSERT INTO notes (id, created_at, updated_at, text, source, status, tags, lat, lon, place)
             VALUES (1, 1793437200, 1793437200, 'Payer le loyer', 'voice', 'ok', '[\"maison\"]', NULL, NULL, ''),
                    (2, 1793437200, 1793437200, 'Code Julie 45B2', 'text', 'ok', '[]', NULL, NULL, '14 rue de Paris'),
                    (3, 1793437200, 1793437200, '', 'voice', 'needs_review', '[]', NULL, NULL, '');
             INSERT INTO triggers (created_at, kind, note_id, spec, next_at, active)
             VALUES (0, 'time', 1, '{\"at\": 1793523600}', 1793523600, 1),
                    (0, 'geo', 2, '{\"lat\": 48.8566, \"lon\": 2.3522, \"radius_m\": 80, \"on\": \"enter\"}', NULL, 1);",
        )
        .unwrap();
        let blob: Vec<u8> = unit(1).iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings (note_id, model, dim, vector) VALUES (1, 'minilm', 384, ?1)",
            [blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audio_refs (created_at, kind, note_id, path, mime) VALUES (0, 'capture', 3, 'audio/a.wav.enc', 'audio/wav')",
            [],
        )
        .unwrap();
        let s = Store::ready(conn).unwrap();
        let rent = s.get_note(1).unwrap().unwrap();
        assert_eq!(
            (rent.trigger_type.as_str(), rent.due_date.as_deref()),
            ("time", Some("2026-11-01T09:00:00Z"))
        );
        assert_eq!(rent.created_at, "2026-10-31T09:00:00Z");
        assert_eq!(rent.tags, vec!["maison".to_string()]);
        let code = s.get_note(2).unwrap().unwrap();
        assert_eq!(
            (code.trigger_type.as_str(), code.radius_meters),
            ("geofence", 80)
        );
        assert_eq!(code.location_raw, "14 rue de Paris");
        assert_eq!(
            s.notes_covering(48.8566, 2.3522).unwrap()[0].0.id,
            2,
            "places reindexed"
        );
        assert_eq!(
            s.get_note(3).unwrap().unwrap().status,
            "pending_transcription"
        );
        assert_eq!(
            s.audio_of(3).unwrap().len(),
            1,
            "the audio still points at its note"
        );
        assert_eq!(
            s.knn(&unit(1), 1).unwrap()[0].0,
            1,
            "the 384 vector carried"
        );
        assert_eq!(s.fts("loyer", 3).unwrap()[0].0, 1, "words reindexed");
    }
}
