"""HTTP/SSE gateway between a browser (desktop graph or phone) and the bus.

It aggregates the mesh into one snapshot, pushes it to clients as server-sent
events, and turns two client actions into bus messages: a teleport request and an
approval verdict. The gateway is the device-side component that holds the approval
key; browsers authenticate to it with a bearer token. No dependency beyond the
standard library."""

from __future__ import annotations

import asyncio
import hmac
import importlib.resources
import json
import uuid
from collections.abc import Callable
from dataclasses import asdict
from typing import Any
from urllib.parse import parse_qs, urlsplit

from .approvals import ApprovalRequest, ApprovalResponse, sign
from .mesh.bus import Bus, Message
from .mesh.telemetry import Telemetry, mascot_state

MAX_BODY = 64 * 1024
TimelineReader = Callable[[str], list[dict[str, Any]]]


class Gateway:
    def __init__(
        self,
        bus: Bus,
        *,
        token: str,
        device_key: bytes,
        timeline_reader: TimelineReader | None = None,
    ) -> None:
        self.bus, self.token, self._key = bus, token, device_key
        self.timeline_reader = timeline_reader
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

    # -- state ---------------------------------------------------------------
    def snapshot(self) -> dict[str, Any]:
        snap = self.telemetry.snapshot()
        mascot = asdict(mascot_state(snap))
        mascot["phase"] = mascot["phase"].value
        return {
            "nodes": self.nodes,
            "agents": {
                a: {"node": v["node"], "phase": v["phase"].value} for a, v in snap["agents"].items()
            },
            "load": snap["nodes"],
            "mascot": mascot,
            "results": self.results[-5:],
            "approvals": [r.model_dump(mode="json") for r in self.approvals.values()],
        }

    def _push(self) -> None:
        text = json.dumps(self.snapshot())
        for queue in self._clients:
            if queue.full():
                queue.get_nowait()  # a slow client loses old frames, not the stream
            queue.put_nowait(text)

    def _on_nodes(self, m: Message) -> None:
        self.nodes = m.payload["nodes"]
        self._push()

    def _on_agent(self, m: Message) -> None:
        p = m.payload
        self.telemetry.update_agent(p["agent"], p["node"], p["phase"])
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
        return self.port

    async def stop(self) -> None:
        if self._server:
            self._server.close()
            self._clients.clear()

    def _authorized(self, headers: dict[str, str], query: dict[str, list[str]]) -> bool:
        given = headers.get("authorization", "").removeprefix("Bearer ").strip()
        given = given or (query.get("token") or [""])[0]
        return hmac.compare_digest(given.encode(), self.token.encode())

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

    async def _send(self, writer, status: int, ctype: str, body: bytes) -> None:
        reason = {200: "OK", 400: "Bad Request", 401: "Unauthorized", 404: "Not Found",
                  413: "Payload Too Large"}.get(status, "")  # fmt: skip
        writer.write(
            f"HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n"
            "Content-Security-Policy: default-src 'self' 'unsafe-inline'; connect-src 'self'\r\n"
            "Connection: close\r\n\r\n".encode()
            + body
        )
        await writer.drain()

    async def _json(self, writer, status: int, obj: Any) -> None:
        await self._send(writer, status, "application/json", json.dumps(obj).encode())

    async def _route(self, method, path, query, headers, body, writer) -> None:
        if method == "GET" and path == "/":
            page = importlib.resources.files("inari_core").joinpath("web/index.html").read_bytes()
            return await self._send(writer, 200, "text/html; charset=utf-8", page)
        if not self._authorized(headers, query):
            return await self._json(writer, 401, {"error": "unauthorized"})
        if method == "GET" and path == "/events":
            return await self._events(writer)
        if method == "GET" and path.startswith("/api/agent/") and path.endswith("/timeline"):
            agent = path.split("/")[3]
            events = self.timeline_reader(agent) if self.timeline_reader else []
            return await self._json(writer, 200, {"events": events})
        if method == "POST":
            try:
                data = json.loads(body or b"{}")
                if path == "/api/teleport":
                    req = await self.request_teleport(
                        str(data["agent"]), str(data["source"]), str(data["target"])
                    )
                    return await self._json(writer, 200, {"request": req})
                if path == "/api/approval":
                    ok = await self.answer_approval(str(data["request_id"]), bool(data["approved"]))
                    return await self._json(writer, 200 if ok else 404, {"ok": ok})
            except (KeyError, ValueError):
                return await self._json(writer, 400, {"error": "bad request"})
        await self._json(writer, 404, {"error": "not found"})

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
