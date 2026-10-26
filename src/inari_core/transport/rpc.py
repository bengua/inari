"""Run teleportation across processes: an `Endpoint` served and called over TCP."""

from __future__ import annotations

import asyncio
import base64
from typing import Any

from ..capsule.models import Manifest
from ..errors import InariError, TransportError
from ..teleport.host import CapsuleHost
from ..teleport.protocol import FrozenAgent
from .wire import Wire

CHUNK = 512 * 1024


class RemoteError(InariError):
    """The peer ran the call and it failed; `kind` is the remote exception type."""

    def __init__(self, kind: str, message: str) -> None:
        super().__init__(f"{kind}: {message}")
        self.kind = kind


class EndpointServer:
    def __init__(self, host: CapsuleHost, key: bytes, bind: str = "127.0.0.1", port: int = 0):
        self.host, self.key, self.bind, self.port = host, key, bind, port
        self._server: asyncio.AbstractServer | None = None
        self._partial: dict[tuple[str, str], bytearray] = {}

    async def start(self) -> int:
        self._server = await asyncio.start_server(self._serve, self.bind, self.port)
        self.port = self._server.sockets[0].getsockname()[1]
        return self.port

    async def stop(self) -> None:
        if self._server:
            self._server.close()
            await self._server.wait_closed()

    async def _serve(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        wire = Wire(reader, writer, self.key)
        try:
            await wire.handshake("server", {"node": self.host.node_id})
            call = await wire.recv()
            try:
                result = await self._dispatch(call["method"], call.get("args", {}))
                await wire.send({"ok": True, "result": result})
            except Exception as exc:  # noqa: BLE001 - reported to the caller
                await wire.send({"ok": False, "kind": type(exc).__name__, "message": str(exc)})
        except TransportError:
            pass  # a bad peer is dropped; nothing to clean up (crash-only)
        finally:
            await wire.close()

    async def _dispatch(self, method: str, a: dict[str, Any]) -> Any:
        h = self.host
        if method == "freeze":
            frozen = await h.freeze(a["agent"])
            return {
                "manifest": frozen.manifest.model_dump(mode="json"),
                "epoch": frozen.epoch,
            }
        if method == "read_chunk":
            data = await h.read(a["agent"], a["rel"])
            piece = data[a["offset"] : a["offset"] + CHUNK]
            return {"data": base64.b64encode(piece).decode(), "total": len(data)}
        if method == "unfreeze":
            return await h.unfreeze(a["agent"], a["epoch"])
        if method == "tombstone":
            return await h.tombstone(a["agent"], a["epoch"])
        if method == "offer":
            have = await h.offer(a["sid"], Manifest.model_validate(a["manifest"]), a["epoch"])
            return sorted(have)
        if method == "receive_chunk":
            key = (a["sid"], a["rel"])
            buf = self._partial.setdefault(key, bytearray())
            if a["offset"] != len(buf):
                raise InariError("out-of-order chunk")
            buf += base64.b64decode(a["data"])
            if a["last"]:
                del self._partial[key]
                await h.receive(a["sid"], a["rel"], bytes(buf))
            return None
        if method in ("verify", "prepare", "abort"):
            return await getattr(h, method)(a["sid"])
        if method == "activate":
            return await h.activate(a["sid"], a["epoch"])
        if method == "holder":
            held = await h.holder(a["agent"])
            return list(held) if held else None
        raise InariError(f"unknown method {method!r}")


class RemoteEndpoint:
    """An `Endpoint` whose work happens on another node. One connection per call:
    nothing to resume after a failure, so a retry is just another call."""

    def __init__(self, node_id: str, address: str, port: int, key: bytes, timeout: float = 30):
        self.node_id, self.address, self.port = node_id, address, port
        self._key, self._timeout = key, timeout

    async def _call(self, method: str, **args: Any) -> Any:
        try:
            reader, writer = await asyncio.wait_for(
                asyncio.open_connection(self.address, self.port), self._timeout
            )
        except (OSError, TimeoutError) as exc:
            raise TransportError(f"cannot reach {self.node_id}: {exc}") from exc
        wire = Wire(reader, writer, self._key)
        try:
            await wire.handshake("client", {"node": "caller"})
            await wire.send({"method": method, "args": args})
            reply = await asyncio.wait_for(wire.recv(), self._timeout)
        except TimeoutError as exc:
            raise TransportError(f"{method}: no reply") from exc
        finally:
            await wire.close()
        if not reply["ok"]:
            raise RemoteError(reply["kind"], reply["message"])
        return reply["result"]

    async def freeze(self, agent_id: str) -> FrozenAgent:
        r = await self._call("freeze", agent=agent_id)
        return FrozenAgent(Manifest.model_validate(r["manifest"]), r["epoch"])

    async def read(self, agent_id: str, rel: str) -> bytes:
        out, offset = bytearray(), 0
        while True:
            r = await self._call("read_chunk", agent=agent_id, rel=rel, offset=offset)
            out += base64.b64decode(r["data"])
            offset = len(out)
            if offset >= r["total"]:
                return bytes(out)

    async def unfreeze(self, agent_id: str, epoch: int) -> None:
        await self._call("unfreeze", agent=agent_id, epoch=epoch)

    async def tombstone(self, agent_id: str, epoch: int) -> None:
        await self._call("tombstone", agent=agent_id, epoch=epoch)

    async def offer(self, session_id: str, manifest: Manifest, epoch: int) -> set[str]:
        have = await self._call(
            "offer",
            sid=session_id,
            manifest=manifest.model_dump(mode="json"),
            epoch=epoch,
        )
        return set(have)

    async def receive(self, session_id: str, rel: str, data: bytes) -> None:
        pieces = [data[i : i + CHUNK] for i in range(0, len(data), CHUNK)] or [b""]
        offset = 0
        for n, piece in enumerate(pieces):
            await self._call(
                "receive_chunk", sid=session_id, rel=rel, offset=offset,
                data=base64.b64encode(piece).decode(), last=n == len(pieces) - 1,
            )  # fmt: skip
            offset += len(piece)

    async def verify(self, session_id: str) -> None:
        await self._call("verify", sid=session_id)

    async def prepare(self, session_id: str) -> None:
        await self._call("prepare", sid=session_id)

    async def activate(self, session_id: str, epoch: int) -> None:
        await self._call("activate", sid=session_id, epoch=epoch)

    async def abort(self, session_id: str) -> None:
        await self._call("abort", sid=session_id)

    async def holder(self, agent_id: str) -> tuple[str, int] | None:
        r = await self._call("holder", agent=agent_id)
        return (r[0], r[1]) if r else None
