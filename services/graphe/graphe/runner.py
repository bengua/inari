"""The run: a sequence of phases on a deterministic rail.

Three lanes (the sssf model): ``agent`` (a caller answers prompts, bounded to a
write scope), ``engineer`` (a human phase — the envelope carries the verdict,
not code), ``code`` (a known command; the script generates the envelope).
Success is earned: every phase defaults to ``fail``; a clean exit flips it, and
an agent phase also needs a valid envelope and green gates.

When an envelope does not parse, violates the schema, or fails a gate, nothing
restarts: the same session is re-prompted with a correction naming exactly what
was wrong — a correction costs one message, a cold start throws away everything
the agent learned.
"""

import hashlib
import json
import os
import secrets
import time
from dataclasses import dataclass, field

from . import envelope as envelope_mod
from .gates import ALL as ALL_GATES


class PhaseError(RuntimeError):
    """A phase that could not earn its success."""


def new_id():
    """Time-ordered enough to sort, unique enough to never collide."""
    return f"{int(time.time() * 1000):x}{secrets.token_hex(5)}"


@dataclass
class AgentCall:
    """One bounded call on the agent lane."""

    prompt: str
    caller: object  # callable(session_id, prompt, correction) -> raw JSON text
    session_id: str
    agent: str = ""
    write_scope: list = field(default_factory=list)
    gates: list = field(default_factory=lambda: list(ALL_GATES))
    max_corrections: int = 3


@dataclass
class _Ctx:
    workdir: str


class Phase:
    """The context manager every phase is, whatever lane it runs on."""

    def __init__(self, run, seq, name, lane, owner, description):
        self.run = run
        self.seq = seq
        self.name = name
        self.lane = lane
        self.owner = owner
        self.phase_id = new_id()
        self._ok = False
        self._store = run._store
        self._store.start_phase(self.phase_id, run.run_id, seq, name, lane, owner,
                                description)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        if exc_type is not None:
            self._store.emit(self.run.run_id, self.phase_id, "phase_error",
                             {"error": str(exc)})
        self._store.end_phase(self.phase_id, self.run.run_id,
                              self._ok and exc_type is None)
        return False

    def succeed(self):
        """Success is earned: the phase says so only when its work held up."""
        self._ok = True

    def log(self, **fields):
        self._store.emit(self.run.run_id, self.phase_id, "log", fields)

    # --- the agent lane ----------------------------------------------------
    def call(self, call: AgentCall):
        """Prompt in, validated envelope out, gates verified — with corrections
        in the same session until it holds, or PhaseError."""
        if self.lane != "agent":
            raise PhaseError(f"phase {self.name!r} is not on the agent lane")
        correction = None
        last_why = "no attempt"
        for _attempt in range(1 + call.max_corrections):
            raw = call.caller(call.session_id, call.prompt, correction)
            env, why = self._try_envelope(raw, call)
            if env is None:
                last_why = why
                correction = f"your envelope was rejected: {why}. Answer with a valid one."
                continue
            failures = self._check_gates(env, call)
            # every attempt lands in the trace — the failure too, or nothing
            # shows why the correction was needed
            envelope_id = self._store.record_envelope(
                self.run.run_id, self.phase_id, env, True)
            for result in env["gate_results"]:
                self._store.record_gate(self.run.run_id, self.phase_id,
                                        envelope_id, result["gate"],
                                        result["ok"], result["detail"])
            if not failures:
                self._store.emit(self.run.run_id, self.phase_id, "agent_answered",
                                 {"session_id": call.session_id})
                return env
            last_why = "; ".join(failures)
            correction = (f"your envelope failed these gates: {last_why}. "
                          "Fix the work, then answer with a valid envelope.")
        raise PhaseError(f"phase {self.name!r}: {last_why}")

    def _try_envelope(self, raw, call):
        """Parse, let the code complete what it owns, validate strictly. A rejected
        envelope is recorded (invalid) so the trace keeps the miss."""
        try:
            env = json.loads(raw)
        except (TypeError, json.JSONDecodeError) as e:
            self._store.record_envelope(self.run.run_id, self.phase_id,
                                        {"raw": str(raw)[:2000]}, False)
            return None, f"not JSON ({e})"
        if not isinstance(env, dict):
            self._store.record_envelope(self.run.run_id, self.phase_id,
                                        {"raw": str(raw)[:2000]}, False)
            return None, "not a JSON object"
        # what the code owns: the agent proposes, the code disposes
        env["envelope"] = "essaim/1"
        env["run_id"] = self.run.run_id
        env["phase_id"] = self.phase_id
        env["lane"] = self.lane
        env["agent"] = call.agent or self.owner
        env["write_scope"] = call.write_scope
        env["trace_cursor"] = self._cursor()
        env.setdefault("status", "done")
        env.setdefault("summary", "")
        env.setdefault("artifacts", [])
        env.setdefault("changed_files", [])
        env.setdefault("notes_for_next_agent", "")
        env.setdefault("commit_message", "")
        env.setdefault("gate_results", [])
        try:
            envelope_mod.validate(env)
        except envelope_mod.EnvelopeError as e:
            self._store.record_envelope(self.run.run_id, self.phase_id, env, False)
            return None, str(e)
        return env, ""

    def _check_gates(self, env, call):
        """Run the gates against the envelope's claims; attach their results."""
        failures = []
        results = []
        for gate in call.gates:
            report = gate(env, _Ctx(self.run.workdir))
            results.append({"gate": report.gate, "ok": report.ok,
                            "detail": report.detail})
            if not report.ok:
                failures.append(f"{report.gate} ({report.detail})")
        env["gate_results"] = results
        return failures

    def _cursor(self):
        events = self._store.events_after(self.run.run_id, 0, limit=1_000_000)
        return events[-1]["seq"] if events else 0

    # --- the code lane -------------------------------------------------------
    def run_code(self, make_envelope):
        """A known command, run by the caller; the script generates the envelope
        (no free-form output). ``make_envelope(phase) -> dict`` fills the fields
        the code owns; the rest is completed here."""
        if self.lane != "code":
            raise PhaseError(f"phase {self.name!r} is not on the code lane")
        env = {
            "envelope": "essaim/1",
            "run_id": self.run.run_id,
            "phase_id": self.phase_id,
            "lane": "code",
            "agent": self.owner,
            "status": "done",
            "summary": "",
            "artifacts": [],
            "changed_files": [],
            "write_scope": [],
            "notes_for_next_agent": "",
            "commit_message": "",
            "gate_results": [],
            "trace_cursor": self._cursor(),
        }
        env.update(make_envelope(self) or {})
        envelope_mod.validate(env)
        self._store.record_envelope(self.run.run_id, self.phase_id, env, True)
        self.succeed()
        return env

    # --- the engineer lane ---------------------------------------------------
    def decide(self, verdict, summary, detail=""):
        """A human phase: the envelope carries the signed verdict, not code.
        ``verdict`` is what the owner answered (True/False); it settles the phase."""
        if self.lane != "engineer":
            raise PhaseError(f"phase {self.name!r} is not on the engineer lane")
        env = {
            "envelope": "essaim/1",
            "run_id": self.run.run_id,
            "phase_id": self.phase_id,
            "lane": "engineer",
            "agent": self.owner,
            "status": "done" if verdict else "blocked",
            "summary": summary,
            "artifacts": [],
            "changed_files": [],
            "write_scope": [],
            "notes_for_next_agent": detail,
            "commit_message": "",
            "gate_results": [],
            "trace_cursor": self._cursor(),
        }
        envelope_mod.validate(env)
        self._store.record_envelope(self.run.run_id, self.phase_id, env, True)
        if verdict:
            self.succeed()
        return env


class Run:
    """One run: its id pins a session, its phases land on the rail in order."""

    def __init__(self, store, run_id, workdir):
        self._store = store
        self.run_id = run_id
        self.workdir = str(workdir)
        self._seq = 0

    @classmethod
    def ensure(cls, store, run_id=None, request="", workdir="."):
        run_id = run_id or new_id()
        store.ensure_session(run_id, request)
        os.makedirs(workdir, exist_ok=True)
        return cls(store, run_id, workdir)

    def phase(self, name, lane, owner, description=""):
        if lane not in ("agent", "engineer", "code"):
            raise PhaseError(f"unknown lane {lane!r}")
        self._seq += 1
        return Phase(self, self._seq, name, lane, owner, description)

    def finish(self, accepted, reason=""):
        """Phases passing is not the run being acceptable: one call settles both."""
        self._store.finish_session(self.run_id, accepted, reason)
        return accepted


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()
