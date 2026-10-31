-- Lists the assistant keeps beside the notes: things to do, the shopping, the pantry, any other.
CREATE TABLE lists (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL DEFAULT 'custom' CHECK (kind IN ('todo', 'shopping', 'pantry', 'custom')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE list_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    list_id    INTEGER NOT NULL REFERENCES lists(id) ON DELETE CASCADE,
    text       TEXT NOT NULL CHECK (length(trim(text)) > 0),
    qty        REAL,
    unit       TEXT NOT NULL DEFAULT '',
    -- where it is found (a shop's aisle, a shelf), to walk the list in order: sorted as text,
    -- so "1-fruits", "2-frais" walks a shop the way it is laid out
    aisle      TEXT NOT NULL DEFAULT '',
    -- a task's due date, ISO 8601 UTC
    due_date   TEXT,
    -- a pantry item's best-before day, YYYY-MM-DD
    expires_on TEXT,
    done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    done_at    TEXT,
    -- the note it came from, if any
    note_id    INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX list_items_open ON list_items(list_id, done, aisle);
