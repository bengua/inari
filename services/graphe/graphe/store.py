"""The graph's durable trace: sessions, phases, events, envelopes, gate results.

SQLite in WAL mode, one numbered events table read by cursor — the CloudFoxRoom
heritage (owner's code, MIT): writers append, readers poll with
``seq > cursor``, live view and full history are the same query at different
cadences.
"""

import json
import sqlite3
import threading
import time

SCHEMA = """
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS sessions (
    run_id TEXT PRIMARY KEY,
    request TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('running','accepted','failed')),
    accepted INTEGER,
    reason TEXT NOT NULL DEFAULT '',
    created_at REAL NOT NULL,
    finished_at REAL
);

CREATE TABLE IF NOT EXISTS phases (
    phase_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES sessions(run_id),
    seq INTEGER NOT NULL,
    name TEXT NOT NULL,
    lane TEXT NOT NULL CHECK(lane IN ('agent','engineer','code')),
    owner TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK(status IN ('running','success','fail')),
    started_at REAL NOT NULL,
    ended_at REAL
);

CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    phase_id TEXT,
    kind TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS envelopes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    phase_id TEXT NOT NULL REFERENCES phases(phase_id),
    envelope_json TEXT NOT NULL,
    valid INTEGER NOT NULL,
    created_at REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS gate_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    phase_id TEXT NOT NULL REFERENCES phases(phase_id),
    envelope_id INTEGER NOT NULL REFERENCES envelopes(id),
    gate TEXT NOT NULL,
    ok INTEGER NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    created_at REAL NOT NULL
);
"""


class Store:
    """One trace database; every connection opens WAL so reads never block writers."""

    def __init__(self, path):
        self._path = str(path)
        self._lock = threading.Lock()
        self._db = sqlite3.connect(self._path, check_same_thread=False)
        self._db.row_factory = sqlite3.Row
        with self._lock, self._db:
            self._db.executescript(SCHEMA)

    def _now(self):
        return time.time()

    # --- sessions ---------------------------------------------------------
    def ensure_session(self, run_id, request):
        """Pin-or-create: an existing run_id rejoins its session unchanged."""
        created = False
        with self._lock, self._db:
            row = self._db.execute(
                "SELECT run_id FROM sessions WHERE run_id = ?", (run_id,)
            ).fetchone()
            if row is None:
                self._db.execute(
                    "INSERT INTO sessions(run_id, request, status, created_at)"
                    " VALUES (?, ?, 'running', ?)",
                    (run_id, request, self._now()),
                )
                created = True
        if created:
            self.emit(run_id, None, "session_started", {"request": request})
        return run_id

    def finish_session(self, run_id, accepted, reason):
        status = "accepted" if accepted else "failed"
        with self._lock, self._db:
            self._db.execute(
                "UPDATE sessions SET status = ?, accepted = ?, reason = ?, finished_at = ?"
                " WHERE run_id = ?",
                (status, int(accepted), reason, self._now(), run_id),
            )
        self.emit(run_id, None, "session_finished",
                  {"accepted": accepted, "reason": reason})

    # --- phases -----------------------------------------------------------
    def start_phase(self, phase_id, run_id, seq, name, lane, owner, description):
        with self._lock, self._db:
            self._db.execute(
                "INSERT INTO phases(phase_id, run_id, seq, name, lane, owner,"
                " description, status, started_at)"
                " VALUES (?, ?, ?, ?, ?, ?, ?, 'running', ?)",
                (phase_id, run_id, seq, name, lane, owner, description, self._now()),
            )
        self.emit(run_id, phase_id, "phase_started",
                  {"seq": seq, "name": name, "lane": lane, "owner": owner})

    def end_phase(self, phase_id, run_id, ok):
        with self._lock, self._db:
            self._db.execute(
                "UPDATE phases SET status = ?, ended_at = ? WHERE phase_id = ?",
                ("success" if ok else "fail", self._now(), phase_id),
            )
        self.emit(run_id, phase_id, "phase_ended", {"ok": ok})

    # --- envelopes and gates ----------------------------------------------
    def record_envelope(self, run_id, phase_id, envelope, valid):
        with self._lock, self._db:
            cur = self._db.execute(
                "INSERT INTO envelopes(run_id, phase_id, envelope_json, valid, created_at)"
                " VALUES (?, ?, ?, ?, ?)",
                (run_id, phase_id, json.dumps(envelope, sort_keys=True),
                 int(valid), self._now()),
            )
            envelope_id = cur.lastrowid
        self.emit(run_id, phase_id,
                  "envelope_recorded" if valid else "envelope_rejected",
                  {"envelope_id": envelope_id})
        return envelope_id

    def record_gate(self, run_id, phase_id, envelope_id, gate, ok, detail):
        with self._lock, self._db:
            self._db.execute(
                "INSERT INTO gate_results(run_id, phase_id, envelope_id, gate, ok,"
                " detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                (run_id, phase_id, envelope_id, gate, int(ok), detail, self._now()),
            )
        self.emit(run_id, phase_id, "gate_result",
                  {"gate": gate, "ok": ok, "detail": detail})

    # --- events, by cursor --------------------------------------------------
    def emit(self, run_id, phase_id, kind, data):
        with self._lock, self._db:
            cur = self._db.execute(
                "INSERT INTO events(run_id, phase_id, kind, data, created_at)"
                " VALUES (?, ?, ?, ?, ?)",
                (run_id, phase_id, kind, json.dumps(data, sort_keys=True), self._now()),
            )
            return cur.lastrowid

    def events_after(self, run_id, cursor, limit=500):
        """The whole transport: live view and full history, one query."""
        with self._lock:
            rows = self._db.execute(
                "SELECT seq, phase_id, kind, data, created_at FROM events"
                " WHERE run_id = ? AND seq > ? ORDER BY seq LIMIT ?",
                (run_id, cursor, limit),
            ).fetchall()
        return [dict(r) for r in rows]

    # --- reads for the reports ----------------------------------------------
    def phases_of(self, run_id):
        with self._lock:
            rows = self._db.execute(
                "SELECT * FROM phases WHERE run_id = ? ORDER BY seq", (run_id,)
            ).fetchall()
        return [dict(r) for r in rows]

    def gates_of(self, run_id):
        with self._lock:
            rows = self._db.execute(
                "SELECT * FROM gate_results WHERE run_id = ? ORDER BY id", (run_id,)
            ).fetchall()
        return [dict(r) for r in rows]

    def envelopes_of(self, run_id):
        with self._lock:
            rows = self._db.execute(
                "SELECT * FROM envelopes WHERE run_id = ? ORDER BY id", (run_id,)
            ).fetchall()
        return [dict(r) for r in rows]

    def session_of(self, run_id):
        with self._lock:
            row = self._db.execute(
                "SELECT * FROM sessions WHERE run_id = ?", (run_id,)
            ).fetchone()
        return dict(row) if row else None

    def close(self):
        with self._lock:
            self._db.close()
