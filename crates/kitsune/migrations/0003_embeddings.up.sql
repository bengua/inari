-- A note's vector, per embedding model (the model, hence its size, is a setting: never assumed).
CREATE TABLE embeddings (
    note_id     INTEGER NOT NULL REFERENCES notes (id) ON DELETE CASCADE,
    model       TEXT    NOT NULL,
    dim         INTEGER NOT NULL CHECK (dim > 0),
    -- little-endian f32 values, dim of them
    vector      BLOB    NOT NULL CHECK (length(vector) = dim * 4),
    PRIMARY KEY (note_id, model)
);
