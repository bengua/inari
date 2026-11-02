#!/usr/bin/env python3
"""openfox-acp: OpenFox as an ACP agent, so Inari can run its threads with it.

Inari talks to coding agents over the Agent Client Protocol: JSON-RPC, one
message per line, on stdin and stdout. OpenFox has no ACP of its own: it has
an HTTP API and a WebSocket that streams what a session does. This translates
between the two: one OpenFox session per Inari thread, on the machine's own
OpenFox, with OpenFox's own models, tools and confirmations.

    openfox-acp            speak ACP on stdin/stdout (what Inari launches)
    openfox-acp login      read OpenFox's password on stdin, keep its token
    openfox-acp check      say whether OpenFox answers with the kept token

Per machine, in ~/.config/openfox-acp/ (or $OPENFOX_ACP_DIR):
    url     OpenFox's address (default http://127.0.0.1:10369)
    token   what `login` wrote (mode 600). OpenFox derives it from its
            password, so it lasts until that password changes.

Standard library only. stdout carries the protocol and nothing else: every
diagnostic goes to stderr, which Inari keeps in its logs.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import queue
import re
import socket
import ssl
import struct
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

PROTOCOL_VERSION = 1
# OpenFox's two top-level agents. A new OpenFox session starts in "planner",
# which only reads; an Inari thread is there to act, so it starts in
# "builder" ($OPENFOX_ACP_MODE overrides). OpenFox still asks before
# dangerous commands and writes outside the folder: those reach Inari as
# permission requests.
MODES = [
    {"id": "builder", "name": "Build", "description": "OpenFox reads, writes and runs commands."},
    {"id": "planner", "name": "Plan", "description": "OpenFox only reads and plans."},
]
DEFAULT_URL = "http://127.0.0.1:10369"
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def config_dir() -> Path:
    return Path(os.environ.get("OPENFOX_ACP_DIR", Path.home() / ".config" / "openfox-acp"))


def openfox_url() -> str:
    if os.environ.get("OPENFOX_URL"):
        return os.environ["OPENFOX_URL"].rstrip("/")
    try:
        return (config_dir() / "url").read_text(encoding="utf-8").strip().rstrip("/") or DEFAULT_URL
    except FileNotFoundError:
        return DEFAULT_URL


def read_token() -> str:
    try:
        return (config_dir() / "token").read_text(encoding="utf-8").strip()
    except FileNotFoundError:
        return ""


def log(message: str) -> None:
    print(f"openfox-acp: {message}", file=sys.stderr, flush=True)


# ── OpenFox over HTTP ────────────────────────────────────────────────────────

class OpenFoxError(RuntimeError):
    pass


class OpenFox:
    def __init__(self, url: str, token: str) -> None:
        self.url = url.rstrip("/")
        self.token = token

    def call(self, method: str, path: str, body: dict | None = None) -> dict:
        data = None if body is None else json.dumps(body).encode()
        headers = {"content-type": "application/json"}
        if self.token:
            headers["x-session-token"] = self.token
        request = urllib.request.Request(self.url + path, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                raw = response.read()
        except urllib.error.HTTPError as e:
            try:
                reason = json.loads(e.read() or b"{}").get("error") or e.reason
            except ValueError:
                reason = e.reason
            raise OpenFoxError(f"{method} {path}: HTTP {e.code} {reason}") from None
        except OSError as e:
            raise OpenFoxError(f"{method} {path}: OpenFox unreachable at {self.url} ({e})") from None
        return json.loads(raw) if raw else {}

    def project_for(self, cwd: str) -> dict:
        """The OpenFox project whose folder is `cwd`, created when missing."""
        wanted = os.path.realpath(cwd)
        listing = self.call("GET", "/api/projects")
        projects = listing.get("projects", listing) if isinstance(listing, dict) else listing
        for project in projects or []:
            if os.path.realpath(str(project.get("workdir", ""))) == wanted:
                return project
        name = re.sub(r"[^A-Za-z0-9._-]+", "-", os.path.basename(wanted)).strip("-.") or "workspace"
        return self.call("POST", "/api/projects", {"name": name, "workdir": wanted})["project"]

    def new_session(self, project_id: str, title: str) -> dict:
        return self.call("POST", "/api/sessions", {"projectId": project_id, "title": title})["session"]

    def top_level_agents(self) -> list[dict]:
        """The agents a session can run as (OpenFox's own and the ones defined by the user or the
        project), as ACP modes. Falls back to Build and Plan when OpenFox does not list them."""
        try:
            got = self.call("GET", "/api/agents")
        except OpenFoxError:
            return MODES
        names = {"builder": "Build", "planner": "Plan"}
        modes: list[dict] = []
        for key in ("defaults", "userItems", "projectItems"):
            for a in got.get(key) or []:
                if a.get("subagent") or not a.get("id") or any(m["id"] == a["id"] for m in modes):
                    continue
                modes.append({"id": a["id"], "name": names.get(a["id"], a.get("name") or a["id"]),
                              "description": a.get("description") or ""})
        order = {"builder": 0, "planner": 1}
        modes.sort(key=lambda m: order.get(m["id"], 2))
        return modes or MODES

    def providers(self) -> dict:
        return self.call("GET", "/api/providers")

    def send(self, session_id: str, text: str) -> None:
        self.call("POST", f"/api/sessions/{session_id}/message", {"content": text})

    def set_model(self, session_id: str, provider_id: str, model: str) -> None:
        self.call("POST", f"/api/sessions/{session_id}/provider", {"providerId": provider_id, "model": model})

    def set_mode(self, session_id: str, mode: str) -> None:
        self.call("PUT", f"/api/sessions/{session_id}/mode", {"mode": mode})

    def stop(self, session_id: str) -> None:
        self.call("POST", f"/api/sessions/{session_id}/stop", {})

    def confirm(self, session_id: str, call_id: str, approved: bool, always: bool) -> None:
        self.call("POST", f"/api/sessions/{session_id}/confirm-path",
                  {"callId": call_id, "approved": approved, "alwaysAllow": always})

    def answer(self, session_id: str, call_id: str, answer: str | None) -> None:
        body = {"callId": call_id, "skip": True} if answer is None else {"callId": call_id, "answer": answer}
        self.call("POST", f"/api/sessions/{session_id}/answer", body)

    def ws_url(self) -> str:
        parts = urllib.parse.urlsplit(self.url)
        scheme = "wss" if parts.scheme == "https" else "ws"
        query = urllib.parse.urlencode({"token": self.token}) if self.token else ""
        return urllib.parse.urlunsplit((scheme, parts.netloc, "/", query, ""))


def model_catalog(providers: dict, current: str | None = None) -> tuple[list[dict], str | None]:
    """ACP availableModels ("<provider>/<model>") and the current one: the
    session's own model when given (OpenFox's default), else the active
    provider's first. Announcing a model the session does not use would make
    Inari switch the session to it."""
    entries = providers.get("providers", []) if isinstance(providers, dict) else providers
    active = providers.get("activeProviderId") if isinstance(providers, dict) else None
    models: list[dict] = []
    known = current
    current = None
    for provider in entries or []:
        pid, pname = provider.get("id"), provider.get("name") or provider.get("id")
        for model in provider.get("models") or []:
            mid = model.get("id") if isinstance(model, dict) else str(model)
            if not pid or not mid:
                continue
            model_id = f"{pid}/{mid}"
            models.append({"modelId": model_id, "name": f"{pname} · {mid}",
                           "description": f"Provider: {pname}"})
            if current is None and pid == active:
                current = model_id
    if known and any(m["modelId"] == known for m in models):
        current = known
    return models, current or (models[0]["modelId"] if models else None)


# ── a minimal WebSocket client (RFC 6455), enough for OpenFox's event stream ──

class WebSocket:
    def __init__(self, url: str, timeout: float = 30) -> None:
        parts = urllib.parse.urlsplit(url)
        port = parts.port or (443 if parts.scheme == "wss" else 80)
        sock = socket.create_connection((parts.hostname, port), timeout=timeout)
        if parts.scheme == "wss":
            sock = ssl.create_default_context().wrap_socket(sock, server_hostname=parts.hostname)
        self.sock = sock
        self.lock = threading.Lock()
        key = base64.b64encode(os.urandom(16)).decode()
        path = (parts.path or "/") + (f"?{parts.query}" if parts.query else "")
        sock.sendall((f"GET {path} HTTP/1.1\r\nHost: {parts.netloc}\r\nUpgrade: websocket\r\n"
                      f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                      "Sec-WebSocket-Version: 13\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = sock.recv(4096)
            if not chunk:
                raise OpenFoxError("WebSocket handshake: connection closed")
            head += chunk
        head, _, self.buf = head.partition(b"\r\n\r\n")
        lines = head.decode("latin-1").split("\r\n")
        if " 101 " not in lines[0] + " ":
            raise OpenFoxError(f"WebSocket handshake refused: {lines[0]}")
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        headers = {k.strip().lower(): v.strip() for k, _, v in (line.partition(":") for line in lines[1:])}
        if headers.get("sec-websocket-accept") != accept:
            raise OpenFoxError("WebSocket handshake: bad accept key")
        sock.settimeout(None)

    def _exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise EOFError
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _send(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            header = struct.pack("!BB", 0x80 | opcode, 0x80 | n)
        elif n < 65536:
            header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, n)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self.lock:
            self.sock.sendall(header + mask + masked)

    def send_text(self, text: str) -> None:
        self._send(0x1, text.encode())

    def recv(self) -> str | None:
        """The next text message, or None once the server closed."""
        parts: list[bytes] = []
        try:
            while True:
                b1, b2 = self._exact(2)
                fin, opcode = b1 & 0x80, b1 & 0x0F
                n = b2 & 0x7F
                if n == 126:
                    n = struct.unpack("!H", self._exact(2))[0]
                elif n == 127:
                    n = struct.unpack("!Q", self._exact(8))[0]
                mask = self._exact(4) if b2 & 0x80 else None
                payload = self._exact(n)
                if mask:
                    payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
                if opcode == 0x8:
                    return None
                if opcode == 0x9:
                    self._send(0xA, payload)
                    continue
                if opcode in (0x1, 0x2, 0x0):
                    parts.append(payload)
                    if fin:
                        return b"".join(parts).decode("utf-8", "replace")
        except (EOFError, OSError):
            return None

    def close(self) -> None:
        try:
            self._send(0x8, b"")
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


# ── one Inari thread = one OpenFox session ───────────────────────────────────

class Session:
    def __init__(self, openfox: OpenFox, session_id: str) -> None:
        self.id = session_id
        self.events: queue.Queue = queue.Queue()
        self.cancelled = threading.Event()
        self.ws = WebSocket(openfox.ws_url())
        self.closed = threading.Event()
        threading.Thread(target=self._read, daemon=True).start()
        self.ws.send_text(json.dumps({"id": f"load-{session_id}", "type": "session.load",
                                      "payload": {"sessionId": session_id}}))

    def _read(self) -> None:
        while True:
            raw = self.ws.recv()
            if raw is None:
                self.closed.set()
                self.events.put({"type": "_closed"})
                return
            try:
                message = json.loads(raw)
            except ValueError:
                continue
            if message.get("sessionId") not in (None, self.id):
                continue
            self.events.put(message)


TOOL_KINDS = {
    "read": "read", "read_file": "read", "view": "read", "glob": "search", "grep": "search",
    "search": "search", "list": "search", "ls": "search", "write": "edit", "write_file": "edit",
    "edit": "edit", "edit_file": "edit", "patch": "edit", "multi_edit": "edit", "delete": "delete",
    "move": "move", "bash": "execute", "run": "execute", "shell": "execute", "exec": "execute",
    "run_command": "execute", "fetch": "fetch", "web_fetch": "fetch", "web_search": "fetch",
    "think": "think",
}


def tool_kind(tool: str) -> str:
    return TOOL_KINDS.get(tool.lower(), "other")


def tool_title(tool: str, args: dict) -> str:
    for key in ("command", "cmd", "path", "file_path", "filePath", "pattern", "url", "query"):
        value = args.get(key) if isinstance(args, dict) else None
        if isinstance(value, str) and value:
            return f"{tool}: {value[:120]}"
    return tool


def result_text(result) -> str:
    if isinstance(result, str):
        return result
    if isinstance(result, dict):
        for key in ("output", "content", "error", "message"):
            if isinstance(result.get(key), str):
                return result[key]
    return json.dumps(result)[:4000]


def prompt_text(blocks: list) -> str:
    parts = []
    for block in blocks or []:
        kind = block.get("type")
        if kind == "text":
            parts.append(block.get("text", ""))
        elif kind == "resource":
            resource = block.get("resource") or {}
            if "text" in resource:
                parts.append(f"<file uri=\"{resource.get('uri', '')}\">\n{resource['text']}\n</file>")
        elif kind == "resource_link":
            parts.append(f"(file: {block.get('uri', '')})")
    return "\n\n".join(p for p in parts if p)


class Bridge:
    def __init__(self, openfox: OpenFox, out=None) -> None:
        self.openfox = openfox
        self.out = out or sys.stdout
        self.write_lock = threading.Lock()
        self.next_id = 0
        self.pending: dict = {}
        self.sessions: dict[str, Session] = {}

    # wire
    def send(self, message: dict) -> None:
        line = json.dumps(message, ensure_ascii=False)
        with self.write_lock:
            self.out.write(line + "\n")
            self.out.flush()

    def respond(self, msg_id, result) -> None:
        self.send({"jsonrpc": "2.0", "id": msg_id, "result": result})

    def fail(self, msg_id, code: int, message: str) -> None:
        self.send({"jsonrpc": "2.0", "id": msg_id, "error": {"code": code, "message": message}})

    def notify(self, method: str, params: dict) -> None:
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def update(self, session_id: str, update: dict) -> None:
        self.notify("session/update", {"sessionId": session_id, "update": update})

    def request(self, method: str, params: dict) -> dict:
        with self.write_lock:
            self.next_id += 1
            msg_id = f"ofx-{self.next_id}"
        done = threading.Event()
        self.pending[msg_id] = {"done": done}
        self.send({"jsonrpc": "2.0", "id": msg_id, "method": method, "params": params})
        done.wait()
        entry = self.pending.pop(msg_id)
        if "error" in entry:
            raise OpenFoxError(f"{method}: {entry['error'].get('message')}")
        return entry.get("result") or {}

    # dispatch
    def handle(self, message: dict) -> None:
        method, msg_id = message.get("method"), message.get("id")
        if method is None:
            entry = self.pending.get(msg_id)
            if entry is not None:
                if "error" in message:
                    entry["error"] = message["error"]
                else:
                    entry["result"] = message.get("result")
                entry["done"].set()
            return
        params = message.get("params") or {}
        if msg_id is None:
            if method == "session/cancel":
                self.cancel(params.get("sessionId"))
            return
        handlers = {
            "initialize": self.initialize,
            "session/new": self.new_session,
            "session/load": self.load_session,
            "session/set_model": self.set_model,
            "session/set_mode": self.set_mode,
            "session/prompt": self.prompt,
            "authenticate": lambda _p: {},
        }
        handler = handlers.get(method)
        if handler is None:
            self.fail(msg_id, -32601, f"method not found: {method}")
            return

        def run() -> None:
            try:
                self.respond(msg_id, handler(params))
            except OpenFoxError as e:
                log(str(e))
                self.fail(msg_id, -32000, str(e))
            except Exception as e:  # noqa: BLE001 - a bridge must answer, never die silently
                log(f"{method} failed: {e!r}")
                self.fail(msg_id, -32603, f"{method} failed: {e}")

        # A prompt waits for OpenFox and for our own permission requests:
        # it runs beside the reader so the reader keeps taking replies.
        if method == "session/prompt":
            threading.Thread(target=run, daemon=True).start()
        else:
            run()

    def serve(self, stream=None) -> None:
        for line in stream or sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                message = json.loads(line)
            except ValueError:
                log("ignored a line that is not JSON")
                continue
            self.handle(message)

    # ACP methods
    def initialize(self, _params: dict) -> dict:
        return {
            "protocolVersion": PROTOCOL_VERSION,
            "agentCapabilities": {
                "loadSession": True,
                "promptCapabilities": {"image": False, "audio": False, "embeddedContext": True},
            },
            "agentInfo": {"name": "openfox-acp", "title": "OpenFox"},
            "authMethods": [],
        }

    def new_session(self, params: dict) -> dict:
        cwd = params.get("cwd") or os.getcwd()
        project = self.openfox.project_for(cwd)
        session = self.openfox.new_session(project["id"], "Inari")
        self.sessions[session["id"]] = Session(self.openfox, session["id"])
        mode = os.environ.get("OPENFOX_ACP_MODE", "builder")
        if mode != session.get("mode"):
            self.openfox.set_mode(session["id"], mode)
        own = None
        if session.get("providerId") and session.get("providerModel"):
            own = f"{session['providerId']}/{session['providerModel']}"
        return self.session_state(session, own, mode)

    def session_state(self, session: dict, own: str | None, mode: str) -> dict:
        models, current = model_catalog(self.openfox.providers(), own)
        result: dict = {"sessionId": session["id"], "modes": {"availableModes": self.openfox.top_level_agents(), "currentModeId": mode}}
        if models:
            result["models"] = {"availableModels": models, "currentModelId": current}
        return result

    def load_session(self, params: dict) -> dict:
        """Take an OpenFox session back: its own memory is kept by OpenFox, so
        nothing is replayed (Inari keeps the thread's history itself)."""
        sid = str(params.get("sessionId", ""))
        got = self.openfox.call("GET", f"/api/sessions/{sid}")  # 404 -> Inari starts a fresh one
        session = got.get("session", got)
        if sid not in self.sessions:
            self.sessions[sid] = Session(self.openfox, sid)
        own = f"{session['providerId']}/{session['providerModel']}" if session.get("providerId") and session.get("providerModel") else None
        return self.session_state({"id": sid}, own, str(session.get("mode") or "builder"))

    def set_model(self, params: dict) -> dict:
        provider_id, _, model = str(params.get("modelId", "")).partition("/")
        if not provider_id or not model:
            raise OpenFoxError(f"unknown model id {params.get('modelId')!r}")
        self.openfox.set_model(params["sessionId"], provider_id, model)
        return {}

    def set_mode(self, params: dict) -> dict:
        mode = str(params.get("modeId", ""))
        if mode not in {m["id"] for m in self.openfox.top_level_agents()}:
            raise OpenFoxError(f"unknown mode {mode!r}")
        self.openfox.set_mode(params["sessionId"], mode)
        return {}

    def cancel(self, session_id: str | None) -> None:
        session = self.sessions.get(session_id or "")
        if session is None:
            return
        session.cancelled.set()
        try:
            self.openfox.stop(session.id)
        except OpenFoxError as e:
            log(f"stop: {e}")

    def prompt(self, params: dict) -> dict:
        session = self.sessions.get(params.get("sessionId", ""))
        if session is None:
            raise OpenFoxError(f"unknown session {params.get('sessionId')!r}")
        while not session.events.empty():  # what came before this prompt is not ours
            session.events.get_nowait()
        session.cancelled.clear()
        self.openfox.send(session.id, prompt_text(params.get("prompt")))
        return {"stopReason": self.follow(session)}

    def follow(self, session: Session) -> str:
        sid, started, asked, retrying = session.id, False, set(), False
        while True:
            try:
                event = session.events.get(timeout=1.0)
            except queue.Empty:
                continue
            kind, payload = event.get("type"), event.get("payload") or {}
            if kind == "_closed":
                raise OpenFoxError("OpenFox closed the event stream")
            if kind == "chat.delta" and payload.get("content"):
                self.update(sid, {"sessionUpdate": "agent_message_chunk",
                                  "content": {"type": "text", "text": payload["content"]}})
            elif kind == "context.state":
                state = payload.get("contextState") or payload
                used, size = state.get("currentTokens"), state.get("maxTokens")
                if isinstance(used, (int, float)) and isinstance(size, (int, float)) and size > 0:
                    self.update(sid, {"sessionUpdate": "usage_update", "used": int(used), "size": int(size)})
            elif kind == "chat.thinking" and payload.get("content"):
                self.update(sid, {"sessionUpdate": "agent_thought_chunk",
                                  "content": {"type": "text", "text": payload["content"]}})
            elif kind == "chat.tool_call":
                tool, args = payload.get("tool", "tool"), payload.get("args") or {}
                self.update(sid, {"sessionUpdate": "tool_call", "toolCallId": payload.get("callId"),
                                  "title": tool_title(tool, args), "kind": tool_kind(tool),
                                  "status": "in_progress", "rawInput": args})
            elif kind == "chat.tool_result":
                result = payload.get("result")
                failed = isinstance(result, dict) and (result.get("success") is False or bool(result.get("error")))
                self.update(sid, {"sessionUpdate": "tool_call_update", "toolCallId": payload.get("callId"),
                                  "status": "failed" if failed else "completed",
                                  "content": [{"type": "content",
                                               "content": {"type": "text", "text": result_text(result)[:8000]}}]})
            elif kind == "chat.todo":
                entries = [{"content": str(t.get("content") or t.get("text") or t), "priority": "medium",
                            "status": {"done": "completed", "completed": "completed",
                                       "in_progress": "in_progress"}.get(t.get("status"), "pending")}
                           for t in payload.get("todos") or [] if isinstance(t, dict)]
                self.update(sid, {"sessionUpdate": "plan", "entries": entries})
            elif kind in ("chat.path_confirmation", "session.confirmation_pending"):
                call_id = payload.get("callId")
                if call_id and call_id not in asked:
                    asked.add(call_id)
                    self.confirm(session, payload)
            elif kind == "chat.ask_user":
                call_id = payload.get("callId")
                if call_id and call_id not in asked:
                    asked.add(call_id)
                    self.ask(session, payload)
            elif kind == "chat.error":
                text = payload.get("error") or payload.get("message") or "OpenFox reported an error"
                self.update(sid, {"sessionUpdate": "agent_message_chunk",
                                  "content": {"type": "text", "text": f"\n\n[OpenFox] {text}"}})
            elif kind == "chat.llm_retry":
                # OpenFox retries a model that does not answer on its own; say
                # so once, instead of a thread that looks dead.
                if not retrying:
                    retrying = True
                    self.update(sid, {"sessionUpdate": "agent_message_chunk", "content": {
                        "type": "text", "text": f"[OpenFox] the model did not answer "
                        f"({str(payload.get('error', '')).strip()[:200]}); retrying...\n"}})
            elif kind == "chat.llm_retry_failed":
                self.update(sid, {"sessionUpdate": "agent_message_chunk", "content": {
                    "type": "text", "text": f"\n[OpenFox] gave up after {payload.get('attempts', '?')} "
                    f"attempts: {str(payload.get('error', '')).strip()[:300]}. Pick another model.\n"}})
                return "end_turn"
            elif kind == "session.running":
                if payload.get("isRunning"):
                    started = True
                elif started:
                    return "cancelled" if session.cancelled.is_set() else "end_turn"
            elif kind == "chat.done" and session.cancelled.is_set():
                return "cancelled"

    def confirm(self, session: Session, payload: dict) -> None:
        tool = payload.get("tool", "tool")
        paths = ", ".join(payload.get("paths") or [])
        reason = str(payload.get("reason", "")).replace("_", " ")
        outcome = self.request("session/request_permission", {
            "sessionId": session.id,
            "toolCall": {"toolCallId": payload.get("callId"), "title": f"{tool}: {paths or reason}",
                         "kind": tool_kind(tool), "status": "pending"},
            "options": [
                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "always", "name": "Always allow", "kind": "allow_always"},
                {"optionId": "deny", "name": "Deny", "kind": "reject_once"},
            ],
        }).get("outcome") or {}
        chosen = outcome.get("optionId") if outcome.get("outcome") == "selected" else "deny"
        self.openfox.confirm(session.id, payload["callId"], chosen in ("allow", "always"), chosen == "always")

    def ask(self, session: Session, payload: dict) -> None:
        options = payload.get("options") or []
        if payload.get("type") == "confirm":
            options = [{"value": "yes", "label": "Yes"}, {"value": "no", "label": "No"}]
        if not options:
            # ACP has no free-text question: show it and let the next prompt answer.
            self.update(session.id, {"sessionUpdate": "agent_message_chunk",
                                     "content": {"type": "text", "text": f"\n\n{payload.get('question', '')}\n"}})
            self.openfox.answer(session.id, payload["callId"], None)
            return
        outcome = self.request("session/request_permission", {
            "sessionId": session.id,
            "toolCall": {"toolCallId": payload.get("callId"), "title": payload.get("question", "Question"),
                         "kind": "other", "status": "pending"},
            "options": [{"optionId": str(o.get("value")), "name": str(o.get("label") or o.get("value")),
                         "kind": "allow_once"} for o in options],
        }).get("outcome") or {}
        if outcome.get("outcome") == "selected":
            self.openfox.answer(session.id, payload["callId"], outcome.get("optionId"))
        else:
            self.openfox.answer(session.id, payload["callId"], None)


# ── commands ─────────────────────────────────────────────────────────────────

def login(url: str) -> int:
    password = sys.stdin.read().rstrip("\n")
    if not password:
        log("no password on stdin")
        return 2
    try:
        token = OpenFox(url, "").call("POST", "/api/auth/login", {"password": password})["token"]
    except (OpenFoxError, KeyError) as e:
        log(f"login failed: {e}")
        return 1
    directory = config_dir()
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / "token"
    tmp = directory / ".token.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(token + "\n")
    tmp.replace(target)
    print(f"openfox-acp: token kept for {url} in {target}")
    return 0


def check(url: str) -> int:
    try:
        projects = OpenFox(url, read_token()).call("GET", "/api/projects")
    except OpenFoxError as e:
        log(str(e))
        return 1
    count = len(projects.get("projects", projects) if isinstance(projects, dict) else projects)
    print(f"openfox-acp: OpenFox at {url} answers ({count} project(s))")
    return 0


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    url = openfox_url()
    if args[:1] == ["login"]:
        return login(url)
    if args[:1] == ["check"]:
        return check(url)
    if args:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    Bridge(OpenFox(url, read_token())).serve()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
