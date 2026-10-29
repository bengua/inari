-- Audio kept for a note: the capture it was transcribed from, or speech synthesised from it.
CREATE TABLE audio_refs (
    id          INTEGER PRIMARY KEY,
    created_at  REAL    NOT NULL,
    -- capture or tts
    kind        TEXT    NOT NULL CHECK (kind IN ('capture', 'tts')),
    note_id     INTEGER REFERENCES notes (id) ON DELETE SET NULL,
    -- relative to the store's folder; the file itself is encrypted at rest
    path        TEXT    NOT NULL UNIQUE,
    mime        TEXT    NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    bytes       INTEGER NOT NULL DEFAULT 0,
    sha256      TEXT    NOT NULL DEFAULT '',
    -- tts: the engine, the voice and its settings, as JSON
    voice       TEXT    NOT NULL DEFAULT '{}'
);
CREATE INDEX audio_note ON audio_refs (note_id);
