-- What brings a note back: a time, a place entered or left, an event of the system.
CREATE TABLE triggers (
    id          INTEGER PRIMARY KEY,
    created_at  REAL    NOT NULL,
    -- time, geo or event
    kind        TEXT    NOT NULL CHECK (kind IN ('time', 'geo', 'event')),
    note_id     INTEGER REFERENCES notes (id) ON DELETE CASCADE,
    -- its details as JSON: {at, every} / {lat, lon, radius_m, on: enter|exit} / {event}
    spec        TEXT    NOT NULL,
    -- for a time trigger: when it fires next
    next_at     REAL,
    active      INTEGER NOT NULL DEFAULT 1,
    fired_at    REAL
);
CREATE INDEX triggers_due ON triggers (active, next_at);
