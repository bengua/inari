-- Notes: what was said or written, as text; where it came from; whether it needs a look.
CREATE TABLE notes (
    id          INTEGER PRIMARY KEY,
    created_at  REAL    NOT NULL,
    updated_at  REAL    NOT NULL,
    text        TEXT    NOT NULL,
    -- voice, text or mcp
    source      TEXT    NOT NULL CHECK (source IN ('voice', 'text', 'mcp')),
    -- ok, or needs_review when the transcription failed or is doubtful
    status      TEXT    NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'needs_review')),
    lang        TEXT    NOT NULL DEFAULT '',
    -- JSON arrays and objects: tags, entities (dates, places, people)
    tags        TEXT    NOT NULL DEFAULT '[]',
    entities    TEXT    NOT NULL DEFAULT '{}',
    lat         REAL,
    lon         REAL,
    place       TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX notes_created ON notes (created_at);

-- Full-text index of the notes (BM25), kept in step by triggers. Accents are folded.
CREATE VIRTUAL TABLE notes_fts USING fts5 (
    text, tags, place,
    content = 'notes', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
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
