-- Back to the earlier shape: one note table, triggers apart, vectors per model. The summary
-- becomes the text; a note's own reminder becomes a trigger again.

DROP TRIGGER notes_au;
DROP TRIGGER notes_ad;
DROP TRIGGER notes_ai;
DROP TABLE notes_fts;
DROP TABLE notes_geo;
DROP INDEX notes_due;
DROP INDEX notes_created;

CREATE TABLE embeddings (
    note_id     INTEGER NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    model       TEXT    NOT NULL,
    dim         INTEGER NOT NULL CHECK (dim > 0),
    vector      BLOB    NOT NULL CHECK (length(vector) = dim * 4),
    PRIMARY KEY (note_id, model)
);
INSERT INTO embeddings (note_id, model, dim, vector)
SELECT note_id, 'restored', 384, embedding FROM notes_vec;
DROP TABLE notes_vec;

CREATE TABLE triggers (
    id          INTEGER PRIMARY KEY,
    created_at  REAL    NOT NULL,
    kind        TEXT    NOT NULL CHECK (kind IN ('time', 'geo', 'event')),
    note_id     INTEGER REFERENCES notes (id) ON DELETE CASCADE,
    spec        TEXT    NOT NULL,
    next_at     REAL,
    active      INTEGER NOT NULL DEFAULT 1,
    fired_at    REAL
);
CREATE INDEX triggers_due ON triggers (active, next_at);
INSERT INTO triggers (created_at, kind, note_id, spec, next_at, active)
SELECT CAST(strftime('%s', created_at) AS REAL), 'time', id,
       json_object('at', CAST(strftime('%s', due_date) AS REAL)),
       CAST(strftime('%s', due_date) AS REAL),
       CASE status WHEN 'dismissed' THEN 0 ELSE 1 END
FROM notes WHERE trigger_type = 'time' AND due_date IS NOT NULL;
INSERT INTO triggers (created_at, kind, note_id, spec, next_at, active)
SELECT CAST(strftime('%s', created_at) AS REAL), 'geo', id,
       json_object('lat', latitude, 'lon', longitude, 'radius_m', radius_meters, 'on', 'enter'),
       NULL,
       CASE status WHEN 'dismissed' THEN 0 ELSE 1 END
FROM notes WHERE trigger_type = 'geofence' AND latitude IS NOT NULL AND longitude IS NOT NULL;

CREATE TABLE old_notes (
    id          INTEGER PRIMARY KEY,
    created_at  REAL    NOT NULL,
    updated_at  REAL    NOT NULL,
    text        TEXT    NOT NULL,
    source      TEXT    NOT NULL CHECK (source IN ('voice', 'text', 'mcp')),
    status      TEXT    NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'needs_review')),
    lang        TEXT    NOT NULL DEFAULT '',
    tags        TEXT    NOT NULL DEFAULT '[]',
    entities    TEXT    NOT NULL DEFAULT '{}',
    lat         REAL,
    lon         REAL,
    place       TEXT    NOT NULL DEFAULT ''
);
INSERT INTO old_notes (id, created_at, updated_at, text, source, status, lang, tags, entities,
                       lat, lon, place)
SELECT id, CAST(strftime('%s', created_at) AS REAL), CAST(strftime('%s', created_at) AS REAL),
       clean_summary, source,
       CASE status WHEN 'pending_transcription' THEN 'needs_review' ELSE 'ok' END,
       lang, tags, '{}', latitude, longitude, location_raw
FROM notes;
DROP TABLE notes;
ALTER TABLE old_notes RENAME TO notes;

CREATE INDEX notes_created ON notes (created_at);
CREATE VIRTUAL TABLE notes_fts USING fts5 (
    text, tags, place,
    content = 'notes', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
INSERT INTO notes_fts (notes_fts) VALUES ('rebuild');
CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
    INSERT INTO notes_fts (rowid, text, tags, place) VALUES (new.id, new.text, new.tags, new.place);
END;
CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, text, tags, place) VALUES ('delete', old.id, old.text, old.tags, old.place);
END;
CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN
    INSERT INTO notes_fts (notes_fts, rowid, text, tags, place) VALUES ('delete', old.id, old.text, old.tags, old.place);
    INSERT INTO notes_fts (rowid, text, tags, place) VALUES (new.id, new.text, new.tags, new.place);
END;
