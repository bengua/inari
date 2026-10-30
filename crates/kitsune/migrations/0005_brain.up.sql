-- brain.db: the shape the R&D brief sets. A note carries its own reminder (a time or a place);
-- its words (FTS5), its meaning (vec0) and its place (R*Tree) are indexed beside it, in the same
-- file. Earlier notes are carried over: their text becomes both the transcript and the summary,
-- and a note's first live reminder (a time first, else a place) becomes its own.

DROP TRIGGER notes_au;
DROP TRIGGER notes_ad;
DROP TRIGGER notes_ai;
DROP TABLE notes_fts;
DROP INDEX notes_created;

CREATE TABLE brain_notes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    -- what was said or written, as it came
    raw_transcript TEXT    NOT NULL,
    -- what it means, said shortly (the small local model writes it)
    clean_summary  TEXT    NOT NULL,
    category       TEXT    NOT NULL DEFAULT 'memo',
    -- a JSON array of strings
    tags           TEXT    NOT NULL DEFAULT '[]',
    trigger_type   TEXT    NOT NULL DEFAULT 'none'
                   CHECK (trigger_type IN ('none', 'time', 'geofence')),
    -- ISO 8601, UTC: 2026-10-31T09:00:00Z
    due_date       TEXT,
    latitude       REAL,
    longitude      REAL,
    radius_meters  INTEGER NOT NULL DEFAULT 100 CHECK (radius_meters > 0),
    status         TEXT    NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'dismissed', 'failed', 'pending_transcription',
                                     'pending_geocode', 'failed_geocode', 'ambiguous_geocode')),
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    -- the place as said ("14 rue de Paris"), kept to geocode it again later
    location_raw   TEXT    NOT NULL DEFAULT '',
    source         TEXT    NOT NULL DEFAULT 'text' CHECK (source IN ('voice', 'text', 'mcp')),
    lang           TEXT    NOT NULL DEFAULT '',
    -- when a time reminder last rang (a reminder rings once, then waits to be dismissed)
    notified_at    TEXT
);

INSERT INTO brain_notes (id, raw_transcript, clean_summary, tags, status, created_at,
                         latitude, longitude, location_raw, source, lang)
SELECT id, text, text, tags,
       CASE status WHEN 'needs_review' THEN 'pending_transcription' ELSE 'active' END,
       strftime('%Y-%m-%dT%H:%M:%SZ', CAST(created_at AS INTEGER), 'unixepoch'),
       lat, lon, place, source, lang
FROM notes;

UPDATE brain_notes
SET trigger_type = 'time',
    due_date = (SELECT strftime('%Y-%m-%dT%H:%M:%SZ', CAST(t.next_at AS INTEGER), 'unixepoch')
                FROM triggers t
                WHERE t.note_id = brain_notes.id AND t.kind = 'time' AND t.active = 1
                  AND t.next_at IS NOT NULL
                ORDER BY t.id LIMIT 1)
WHERE EXISTS (SELECT 1 FROM triggers t
              WHERE t.note_id = brain_notes.id AND t.kind = 'time' AND t.active = 1
                AND t.next_at IS NOT NULL);

UPDATE brain_notes
SET trigger_type = 'geofence',
    latitude = (SELECT json_extract(t.spec, '$.lat') FROM triggers t
                WHERE t.note_id = brain_notes.id AND t.kind = 'geo' AND t.active = 1
                ORDER BY t.id LIMIT 1),
    longitude = (SELECT json_extract(t.spec, '$.lon') FROM triggers t
                 WHERE t.note_id = brain_notes.id AND t.kind = 'geo' AND t.active = 1
                 ORDER BY t.id LIMIT 1),
    radius_meters = MAX(1, COALESCE((SELECT CAST(json_extract(t.spec, '$.radius_m') AS INTEGER)
                                     FROM triggers t
                                     WHERE t.note_id = brain_notes.id AND t.kind = 'geo'
                                       AND t.active = 1
                                     ORDER BY t.id LIMIT 1), 100))
WHERE trigger_type = 'none'
  AND EXISTS (SELECT 1 FROM triggers t
              WHERE t.note_id = brain_notes.id AND t.kind = 'geo' AND t.active = 1
                AND json_extract(t.spec, '$.lat') IS NOT NULL
                AND json_extract(t.spec, '$.lon') IS NOT NULL);

-- the meaning of each note: 384 floats, compared by cosine (vectors of another size are
-- recomputed by the embedding model the person sets)
CREATE VIRTUAL TABLE notes_vec USING vec0(
    note_id   INTEGER PRIMARY KEY,
    embedding FLOAT[384] distance_metric=cosine
);
INSERT INTO notes_vec (note_id, embedding)
SELECT note_id, vector FROM embeddings WHERE dim = 384 GROUP BY note_id;

DROP TABLE embeddings;
DROP INDEX triggers_due;
DROP TABLE triggers;
DROP TABLE notes;
ALTER TABLE brain_notes RENAME TO notes;

CREATE INDEX notes_created ON notes (created_at);
CREATE INDEX notes_due ON notes (trigger_type, status, due_date);

-- the place of each note as a bounding box (filled by the store, which knows the earth's curve)
CREATE VIRTUAL TABLE notes_geo USING rtree(id, min_lat, max_lat, min_lon, max_lon);

-- the words of each note (BM25), kept in step by triggers; accents are folded
CREATE VIRTUAL TABLE notes_fts USING fts5(
    clean_summary, raw_transcript, tags,
    content = 'notes', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
INSERT INTO notes_fts (notes_fts) VALUES ('rebuild');

CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
    INSERT INTO notes_fts (rowid, clean_summary, raw_transcript, tags)
    VALUES (new.id, new.clean_summary, new.raw_transcript, new.tags);
END;
-- a note removed takes its words, its vector and its place with it
CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, clean_summary, raw_transcript, tags)
    VALUES ('delete', old.id, old.clean_summary, old.raw_transcript, old.tags);
    DELETE FROM notes_vec WHERE note_id = old.id;
    DELETE FROM notes_geo WHERE id = old.id;
END;
CREATE TRIGGER notes_au AFTER UPDATE OF clean_summary, raw_transcript, tags ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, clean_summary, raw_transcript, tags)
    VALUES ('delete', old.id, old.clean_summary, old.raw_transcript, old.tags);
    INSERT INTO notes_fts (rowid, clean_summary, raw_transcript, tags)
    VALUES (new.id, new.clean_summary, new.raw_transcript, new.tags);
END;
