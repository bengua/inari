"""HTTP/SSE gateway between a browser (desktop graph or phone) and the bus.

It aggregates the mesh into one snapshot, pushes it to clients as server-sent
events, and turns two client actions into bus messages: a teleport request and an
approval verdict. The gateway is the device-side component that holds the approval
key; browsers authenticate to it with a bearer token. No dependency beyond the
standard library."""

from __future__ import annotations

import asyncio
import hashlib
import hmac
import importlib.resources
import json
import mimetypes
import os
import time
import urllib.request
import uuid
from collections.abc import Callable
from dataclasses import asdict
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, urlsplit

from .approvals import ApprovalRequest, ApprovalResponse, sign
from .errors import TransportError
from .mesh.bus import Bus, Message, request
from .mesh.telemetry import Telemetry, mascot_state
from .usage import poll

MAX_BODY = 64 * 1024
# What a page may load and where it may talk (the same policy as the Rust gateway's).
CSP = (
    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
    "img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; "
    "frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'"
)
COOKIE = "inari_session"
SESSION_TTL = 30 * 24 * 3600


def owner_hash(token: str) -> str:
    """What an owner session is known by (as `session::owner_hash` in Rust)."""
    return hashlib.sha256(f"inari-owner|{token}".encode()).hexdigest()[:32]


# The files of the page under /assets/, and their types (the same list as the Rust gateway's).
ASSETS = {
    "app.css": "text/css; charset=utf-8",
    "app.js": "text/javascript; charset=utf-8",
    "i18n.js": "text/javascript; charset=utf-8",
    "fonts/ibm_plex_sans.woff2": "font/woff2",
    "fonts/ibm_plex_mono_regular.woff2": "font/woff2",
    "fonts/ibm_plex_mono_medium.woff2": "font/woff2",
    "fonts/bricolage_grotesque.woff2": "font/woff2",
}
TimelineReader = Callable[[str], list[dict[str, Any]]]


class Gateway:
    def __init__(
        self,
        bus: Bus,
        *,
        token: str,
        device_key: bytes,
        timeline_reader: TimelineReader | None = None,
        ui: dict[str, Any] | None = None,
        mascot_dir: Path | None = None,
        usage_sources: list | None = None,
        usage_interval: float = 60.0,
    ) -> None:
        self.bus, self.token, self._key = bus, token, device_key
        # Sessions are signed with a key of this process: the reference keeps no state on disk,
        # so its sessions end with it (the Rust gateway keeps its key in its data directory).
        self._session_key = os.urandom(32)
        self.timeline_reader = timeline_reader
        self.log: dict[str, list[dict[str, Any]]] = {}
        self.ui = {"footer": "", **(ui or {})}
        self.mascot_dir = Path(mascot_dir) if mascot_dir else None
        self.usage: list[dict[str, Any]] = []
        self.usage_sources = usage_sources or []
        self.usage_interval = usage_interval
        self._usage_task: asyncio.Task | None = None
        self._logo_cache: dict[str, tuple[str, bytes]] = {}
        self.notices: list[dict[str, Any]] = []
        self.moves: dict[str, dict[str, Any]] = {}
        self._holder: dict[str, str] = {}
        self.telemetry = Telemetry()
        self.nodes: list[dict[str, Any]] = []
        self.results: list[dict[str, Any]] = []
        self.approvals: dict[str, ApprovalRequest] = {}
        self._clients: set[asyncio.Queue[str]] = set()
        self._server: asyncio.AbstractServer | None = None
        self.port = 0
        bus.subscribe("mesh.nodes", self._on_nodes)
        bus.subscribe("agent.*.state", self._on_agent)
        bus.subscribe("node.*.load", self._on_load)
        bus.subscribe("node.lost", self._on_lost)
        bus.subscribe("teleport.result", self._on_result)
        bus.subscribe("approval.request", self._on_approval)
        bus.subscribe("teleport.state", self._on_move)
        bus.subscribe("agent.*.event", self._on_event)
        bus.subscribe("agent.*.history", self._on_history)
        bus.subscribe("agent.*.done", self._on_notice)
        bus.subscribe("agent.created", self._on_notice)
        bus.subscribe("agent.merged", self._on_notice)

    # -- state ---------------------------------------------------------------
    def snapshot(self) -> dict[str, Any]:
        snap = self.telemetry.snapshot()
        mascot = asdict(mascot_state(snap))
        mascot["phase"] = mascot["phase"].value
        return {
            "nodes": self.nodes,
            "agents": {
                a: {**{k: x for k, x in v.items() if k != "phase"}, "phase": v["phase"].value}
                for a, v in snap["agents"].items()
            },
            "moves": self._live_moves(),
            "load": snap["nodes"],
            "mascot": mascot,
            "results": self.results[-5:],
            "notices": self.notices[-5:],
            "usage": self.usage,
            "ui": {**self.ui, "mascot": self.mascot_dir is not None},
            "approvals": [r.model_dump(mode="json") for r in self.approvals.values()],
        }

    def _push(self) -> None:
        text = json.dumps(self.snapshot())
        for queue in self._clients:
            if queue.full():
                queue.get_nowait()  # a slow client loses old frames, not the stream
            queue.put_nowait(text)

    def _on_move(self, m: Message) -> None:
        p = m.payload
        self.moves[p["agent"]] = {**p, "at": time.time()}
        self._push()

    def _live_moves(self) -> dict[str, dict[str, Any]]:
        """Moves in flight, plus finished ones for a few seconds so a client can show the end."""
        now = time.time()
        self.moves = {
            a: v for a, v in self.moves.items()
            if v["state"] not in ("committed", "rolled_back") or now - v["at"] < 5
        }  # fmt: skip
        return {a: {k: x for k, x in v.items() if k != "at"} for a, v in self.moves.items()}

    def _on_nodes(self, m: Message) -> None:
        self.nodes = m.payload["nodes"]
        self._push()

    async def _on_agent(self, m: Message) -> None:
        p = m.payload
        extra = {k: v for k, v in p.items() if k not in ("agent", "node", "phase")}
        self.telemetry.update_agent(p["agent"], p["node"], p["phase"], **extra)
        self._push()
        seen_at = self._holder.get(p["agent"])
        self._holder[p["agent"]] = p["node"]
        if seen_at != p["node"]:  # first sighting, or it moved: ask the holder for the journal
            await self.bus.publish("agent.history.request", {"agent": p["agent"]})

    def _on_event(self, m: Message) -> None:
        p = m.payload
        events = self.log.setdefault(p["agent"], [])
        if all(e["id"] != p["id"] for e in events[-50:]):
            events.append({"id": p["id"], "kind": p["kind"], "text": p["text"]})
        self._push()

    def _on_history(self, m: Message) -> None:
        agent, history = m.payload["agent"], list(m.payload["events"])
        known = {e["id"] for e in history}
        live = [e for e in self.log.get(agent, []) if e["id"] not in known]
        self.log[agent] = history + live  # a late reply must not erase newer events
        self._push()

    def _on_notice(self, m: Message) -> None:
        self.notices.append({"subject": m.subject, **m.payload})
        self.notices = self.notices[-10:]
        self._push()

    def _on_load(self, m: Message) -> None:
        p = m.payload
        self.telemetry.update_node(p["node"], gpu=p.get("gpu", 0.0), packets=p.get("packets", 0.0))
        self._push()

    def _on_lost(self, m: Message) -> None:
        self.telemetry.drop_node(m.payload["node"])
        self.nodes = [n for n in self.nodes if n["node_id"] != m.payload["node"]]
        self._push()

    def _on_result(self, m: Message) -> None:
        self.results.append(m.payload)
        self._push()

    def _on_approval(self, m: Message) -> None:
        request = ApprovalRequest.model_validate(m.payload)
        self.approvals[request.request_id] = request
        self._push()

    def search(self, query: str, agent: str = "") -> list[dict[str, Any]]:
        from .capsule.vectors import VectorIndex

        needle, hits = query.lower().strip(), []
        pool = {a: v for a, v in self.log.items() if not agent or a == agent}
        index = VectorIndex()
        for a, events in pool.items():
            for e in events:
                index.add(f"{a}|{e['id']}", e["text"])
                if needle and needle in e["text"].lower():
                    hits.append({"agent": a, **e})
        seen = {(h["agent"], h["id"]) for h in hits}
        for key, score in index.search(query, k=6):
            a, _, eid = key.partition("|")
            if score > 0.05 and (a, eid) not in seen:
                e = next(x for x in pool[a] if x["id"] == eid)
                hits.append({"agent": a, **e})
        return hits[:10]

    async def _ask(self, subject: str, payload: dict[str, Any], timeout: float = 5.0):
        """Ask the node that holds an agent and wait for its one reply."""
        reply = f"reply.{uuid.uuid4().hex}"
        try:
            return (
                await request(self.bus, subject, {**payload, "reply": reply}, reply, timeout)
            ).payload
        except TransportError:
            return None

    # -- actions ---------------------------------------------------------------
    async def request_teleport(self, agent: str, source: str, target: str) -> str:
        request = uuid.uuid4().hex
        await self.bus.publish(
            "teleport.request",
            {"request": request, "agent": agent, "source": source, "target": target},
        )
        return request

    async def answer_approval(self, request_id: str, approved: bool) -> bool:
        request = self.approvals.pop(request_id, None)
        if request is None:
            return False
        response = ApprovalResponse(
            request_id=request_id,
            approved=approved,
            signature=sign(request, approved, self._key),
        )
        await self.bus.publish("approval.response", response.model_dump(mode="json"))
        self._push()
        return True

    # -- HTTP ------------------------------------------------------------------
    async def start(self, host: str = "127.0.0.1", port: int = 0) -> int:
        self._server = await asyncio.start_server(self._serve, host, port)
        self.port = self._server.sockets[0].getsockname()[1]
        if self.usage_sources:
            self._usage_task = asyncio.create_task(
                poll(self.usage_sources, self.usage_interval, self._set_usage)
            )
        return self.port

    async def _set_usage(self, items: list[dict[str, Any]]) -> None:
        self.usage = items
        self._push()

    async def stop(self) -> None:
        if self._usage_task:
            self._usage_task.cancel()
        if self._server:
            self._server.close()
            self._clients.clear()

    def _authorized(self, headers: dict[str, str], query: dict[str, list[str]]) -> bool:
        given = headers.get("authorization", "").removeprefix("Bearer ").strip()
        given = given or (query.get("token") or [""])[0]
        if given:
            return hmac.compare_digest(given.encode(), self.token.encode())
        return self._session_ok(headers.get("cookie", ""))

    def _session_mac(self, kind: str, digest: str, exp: int) -> str:
        msg = b"|".join([b"inari-session", kind.encode(), digest.encode(), str(exp).encode()])
        return hmac.new(self._session_key, msg, hashlib.sha256).hexdigest()

    def session_cookie(self) -> str:
        """A signed owner session: `v1.o.<hash of the token>.<expiry>.<mac>`."""
        exp = int(time.time()) + SESSION_TTL
        digest = owner_hash(self.token)
        return f"v1.o.{digest}.{exp}.{self._session_mac('o', digest, exp)}"

    def _session_ok(self, cookies: str) -> bool:
        value = next(
            (
                v
                for k, _, v in (c.strip().partition("=") for c in cookies.split(";"))
                if k == COOKIE
            ),
            "",
        )
        parts = value.split(".")
        if len(parts) != 5 or parts[0] != "v1" or parts[1] != "o" or not parts[3].isdigit():
            return False
        _, kind, digest, exp, given = parts
        want = self._session_mac(kind, digest, int(exp))
        return (
            bool(self.token)
            and hmac.compare_digest(want, given)
            and int(exp) > time.time()
            and digest == owner_hash(self.token)
        )

    async def _sign_in(self, headers: dict[str, str], body: bytes, writer) -> None:
        """POST /api/session {"secret": token}: a session cookie for the owner."""
        try:
            secret = str(json.loads(body or b"{}").get("secret", "")).strip()
        except (ValueError, AttributeError):
            secret = ""
        if not (
            self.token and secret and hmac.compare_digest(secret.encode(), self.token.encode())
        ):
            return await self._json(writer, 401, {"error": "unknown token or pass"})
        secure = "; Secure" if headers.get("x-forwarded-proto", "").lower() == "https" else ""
        cookie = (
            f"Set-Cookie: {COOKIE}={self.session_cookie()}; Path=/; HttpOnly; SameSite=Strict; "
            f"Max-Age={SESSION_TTL}{secure}\r\n"
        )
        await self._send(writer, 200, "application/json", b'{"role": "owner"}', cookie)

    async def _serve(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 10)
            lines = head.decode("latin-1").split("\r\n")
            method, target, _ = lines[0].split(" ", 2)
            headers = {
                k.lower(): v.strip() for k, v in (ln.split(":", 1) for ln in lines[1:] if ":" in ln)
            }
            size = int(headers.get("content-length", "0"))
            if size > MAX_BODY:
                return await self._send(writer, 413, "text/plain", b"too large")
            body = await reader.readexactly(size) if size else b""
            url = urlsplit(target)
            await self._route(method, url.path, parse_qs(url.query), headers, body, writer)
        except (
            asyncio.IncompleteReadError,
            asyncio.LimitOverrunError,
            ValueError,
            TimeoutError,
        ):
            pass
        except ConnectionError:
            pass
        finally:
            writer.close()

    async def _send(self, writer, status: int, ctype: str, body: bytes, extra: str = "") -> None:
        reason = {200: "OK", 400: "Bad Request", 401: "Unauthorized", 404: "Not Found",
                  413: "Payload Too Large"}.get(status, "")  # fmt: skip
        writer.write(
            f"HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n"
            f"Content-Security-Policy: {CSP}\r\nReferrer-Policy: no-referrer\r\n"
            f"{extra}Connection: close\r\n\r\n".encode()
            + body
        )
        await writer.drain()

    async def _json(self, writer, status: int, obj: Any) -> None:
        await self._send(writer, status, "application/json", json.dumps(obj).encode())

    async def _route(self, method, path, query, headers, body, writer) -> None:
        if method == "GET" and path == "/":
            page = importlib.resources.files("inari_core").joinpath("web/index.html").read_bytes()
            return await self._send(writer, 200, "text/html; charset=utf-8", page)
        if method == "GET" and path.startswith("/assets/"):
            return await self._asset(writer, path[len("/assets/") :])
        if method == "GET" and path.startswith("/fox/") and self.mascot_dir is not None:
            return await self._static(writer, self.mascot_dir, path[len("/fox/") :] or "index.html")
        if method == "POST" and path == "/api/session":
            return await self._sign_in(headers, body, writer)
        if method == "POST" and path == "/api/logout":
            end = f"Set-Cookie: {COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0\r\n"
            return await self._send(writer, 200, "application/json", b"{}", end)
        if not self._authorized(headers, query):
            return await self._json(writer, 401, {"error": "unauthorized"})
        if method == "GET" and path == "/api/whoami":
            return await self._json(writer, 200, {"role": "owner"})
        if method == "GET" and path == "/events":
            return await self._events(writer)
        if method == "GET" and path.startswith("/api/agent/") and path.endswith("/timeline"):
            agent = path.split("/")[3]
            events = (
                self.timeline_reader(agent) if self.timeline_reader else self.log.get(agent, [])
            )
            return await self._json(writer, 200, {"events": events})
        if method == "GET" and path.startswith("/api/agent/") and path.endswith("/files"):
            reply = await self._ask("agent.files.request", {"agent": path.split("/")[3]})
            return await self._json(writer, 200, reply or {"files": []})
        if method == "GET" and path.startswith("/api/agent/") and path.endswith("/file"):
            want = {"agent": path.split("/")[3], "path": (query.get("path") or [""])[0]}
            return await self._json(
                writer, 200, await self._ask("agent.file.request", want) or {"text": ""}
            )
        if method == "GET" and path.startswith("/api/logo/"):
            return await self._logo(writer, path.rsplit("/", 1)[1])
        if method == "GET" and path == "/api/search":
            return await self._json(
                writer,
                200,
                {"hits": self.search((query.get("q") or [""])[0], (query.get("agent") or [""])[0])},
            )
        if method == "POST":
            try:
                data = json.loads(body or b"{}")
                if path == "/api/teleport":
                    req = await self.request_teleport(
                        str(data["agent"]), str(data["source"]), str(data["target"])
                    )
                    return await self._json(writer, 200, {"request": req})
                if path == "/api/exec":
                    reply = await self._ask(
                        "agent.exec.request",
                        {"agent": str(data["agent"]), "cmd": str(data["cmd"])},
                        timeout=75.0,
                    )
                    return await self._json(
                        writer, 200, reply or {"ok": False, "output": "no answer"}
                    )
                if path == "/api/upload":
                    reply = await self._ask(
                        "agent.upload.request",
                        {k: str(data[k]) for k in ("agent", "path", "b64")},
                        timeout=30.0,
                    )
                    return await self._json(writer, 200, reply or {"ok": False})
                if path == "/api/harness":
                    await self.bus.publish(
                        f"agent.{data['agent']}.harness", {"harness": str(data["harness"])}
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/model":
                    await self.bus.publish(
                        f"agent.{data['agent']}.model", {"model": str(data["model"])}
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/agent":
                    await self.bus.publish(
                        "agent.create",
                        {k: data[k] for k in ("thread_id", "node", "harness") if k in data}
                        | {"title": str(data.get("title", ""))},
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/message":
                    await self.bus.publish(
                        f"agent.{data['agent']}.run", {"text": str(data["text"])}
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/fork":
                    await self.bus.publish(
                        f"agent.{data['agent']}.fork",
                        {"new_thread": str(data["new_thread"]), "upto": data.get("upto")},
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/merge":
                    await self.bus.publish(
                        "agent.merge",
                        {"target": str(data["target"]), "source": str(data["source"])},
                    )
                    return await self._json(writer, 200, {"ok": True})
                if path == "/api/approval":
                    ok = await self.answer_approval(str(data["request_id"]), bool(data["approved"]))
                    return await self._json(writer, 200 if ok else 404, {"ok": ok})
            except (KeyError, ValueError):
                return await self._json(writer, 400, {"error": "bad request"})
        await self._json(writer, 404, {"error": "not found"})

    async def _asset(self, writer, rel: str) -> None:
        """A file of the page (style, script, fonts): public like the page, never outside web/."""
        if rel not in ASSETS:
            return await self._json(writer, 404, {"error": "not found"})
        body = importlib.resources.files("inari_core").joinpath("web", rel).read_bytes()
        writer.write(
            (
                f"HTTP/1.1 200 OK\r\nContent-Type: {ASSETS[rel]}\r\n"
                f"Content-Length: {len(body)}\r\n"
                "Cache-Control: public, max-age=86400\r\nX-Content-Type-Options: nosniff\r\n"
                "Connection: close\r\n\r\n"
            ).encode()
            + body
        )
        await writer.drain()

    async def _static(self, writer, root: Path, rel: str) -> None:
        """Serve a file under `root`, never outside it. Streamed: sprite sets are large."""
        target = (root / rel).resolve()
        if root.resolve() not in target.parents or not target.is_file():
            return await self._json(writer, 404, {"error": "not found"})
        ctype = mimetypes.guess_type(target.name)[0] or "application/octet-stream"
        writer.write(
            (
                f"HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\n"
                f"Content-Length: {target.stat().st_size}\r\n"
                "Cache-Control: max-age=3600\r\nX-Content-Type-Options: nosniff\r\n"
                "Connection: close\r\n\r\n"
            ).encode()
        )
        with target.open("rb") as fh:
            while chunk := fh.read(65536):
                writer.write(chunk)
                await writer.drain()

    async def _logo(self, writer, family: str) -> None:
        """Provider logos live on the usage source's server: fetched once, served from here."""
        if family not in self._logo_cache:
            for source in self.usage_sources:
                url = getattr(source, "logos", {}).get(family)
                if url:
                    try:
                        raw = await asyncio.to_thread(
                            lambda u=url: urllib.request.urlopen(u, timeout=10).read()
                        )
                        self._logo_cache[family] = (
                            "image/svg+xml" if raw[:5] != b"\x89PNG\r" else "image/png",
                            raw,
                        )
                    except (OSError, ValueError):
                        pass
        if family not in self._logo_cache:
            return await self._json(writer, 404, {"error": "no logo"})
        ctype, raw = self._logo_cache[family]
        await self._send(writer, 200, ctype, raw)

    async def _events(self, writer) -> None:
        queue: asyncio.Queue[str] = asyncio.Queue(maxsize=50)
        self._clients.add(queue)
        writer.write(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n"
            b"Connection: keep-alive\r\n\r\n"
        )
        queue.put_nowait(json.dumps(self.snapshot()))
        try:
            while True:
                try:
                    text = await asyncio.wait_for(queue.get(), 15)
                    writer.write(f"data: {text}\n\n".encode())
                except TimeoutError:
                    writer.write(b": keep-alive\n\n")
                await writer.drain()
        except (ConnectionError, asyncio.CancelledError):
            pass
        finally:
            self._clients.discard(queue)
