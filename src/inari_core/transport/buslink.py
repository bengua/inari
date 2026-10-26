"""Bus-to-bus links over TCP: durable subjects queue while the peer is away and
replay on reconnect; the receiving bus drops duplicates by message id."""

from __future__ import annotations

import asyncio
import contextlib
from collections import deque
from typing import Any

from ..errors import TransportError
from ..mesh.bus import Bus, Message, subject_matches
from .wire import Wire

DURABLE = ("teleport.>", "approval.>", "agent.>")


def _to_dict(m: Message) -> dict[str, Any]:
    return {
        "subject": m.subject, "payload": m.payload, "sender": m.sender,
        "msg_id": m.msg_id, "path": list(m.path),
    }  # fmt: skip


def _from_dict(d: dict[str, Any]) -> Message:
    return Message(d["subject"], d["payload"], d["sender"], d["msg_id"], tuple(d["path"]))


class PeerLink:
    """Same duck type as `mesh.bus.Link`: `peer_name` and `send`."""

    def __init__(self, bus: Bus, durable: tuple[str, ...] = DURABLE, limit: int = 10_000):
        self.bus, self.durable = bus, durable
        self.peer_name = ""
        self.wire: Wire | None = None
        self.outbox: deque[Message] = deque(maxlen=limit)
        self._reader: asyncio.Task | None = None

    @property
    def connected(self) -> bool:
        return self.wire is not None

    async def send(self, message: Message) -> None:
        if self.wire is not None:
            try:
                await self.wire.send(_to_dict(message))
                return
            except TransportError:
                await self._drop()
        if any(subject_matches(p, message.subject) for p in self.durable):
            self.outbox.append(message)

    async def attach(self, wire: Wire) -> None:
        await self._drop()
        self.wire, self.peer_name = wire, wire.peer["bus"]
        while self.outbox and self.wire is not None:
            await self.send(self.outbox.popleft())
        self._reader = asyncio.create_task(self._read(wire))

    async def _read(self, wire: Wire) -> None:
        try:
            while True:
                await self.bus.deliver(_from_dict(await wire.recv()))
        except TransportError:
            pass
        finally:
            if self.wire is wire:
                self.wire = None
            await wire.close()

    async def _drop(self) -> None:
        wire, self.wire = self.wire, None
        if wire is not None:
            await wire.close()
        if self._reader and self._reader is not asyncio.current_task():
            self._reader.cancel()


class BusServer:
    def __init__(
        self,
        bus: Bus,
        key: bytes,
        bind: str = "127.0.0.1",
        port: int = 0,
        expected_peers: tuple[str, ...] = (),
    ):
        """`expected_peers` are bus names that will dial in: links for them exist
        from the start, so durable messages queue even before their first visit."""
        self.bus, self.key, self.bind, self.port = bus, key, bind, port
        self.links: dict[str, PeerLink] = {}
        for name in expected_peers:
            link = self.links[name] = PeerLink(bus)
            link.peer_name = name
            bus.add_link(link)
        self._server: asyncio.AbstractServer | None = None

    async def start(self) -> int:
        self._server = await asyncio.start_server(self._accept, self.bind, self.port)
        self.port = self._server.sockets[0].getsockname()[1]
        return self.port

    async def stop(self) -> None:
        for link in self.links.values():  # first: wait_closed() waits for them
            await link._drop()
        if self._server:
            self._server.close()
            await self._server.wait_closed()

    async def _accept(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        wire = Wire(reader, writer, self.key)
        try:
            await wire.handshake("server", {"bus": self.bus.name})
        except TransportError:
            await wire.close()
            return
        link = self.links.get(wire.peer["bus"])
        if link is None:
            link = self.links[wire.peer["bus"]] = PeerLink(self.bus)
            self.bus.add_link(link)
        await link.attach(wire)


class BusClient:
    """Dials a server and keeps the link up, reconnecting with a capped backoff."""

    def __init__(self, bus: Bus, key: bytes, address: str, port: int, retry: float = 0.2):
        self.bus, self.key, self.address, self.port = bus, key, address, port
        self.retry = retry
        self.link = PeerLink(bus)
        bus.add_link(self.link)
        self._task: asyncio.Task | None = None

    def start(self) -> None:
        self._task = asyncio.create_task(self._run())

    async def stop(self) -> None:
        if self._task:
            self._task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._task
        await self.link._drop()

    async def _run(self) -> None:
        delay = self.retry
        while True:
            try:
                reader, writer = await asyncio.open_connection(self.address, self.port)
                wire = Wire(reader, writer, self.key)
                await wire.handshake("client", {"bus": self.bus.name})
                await self.link.attach(wire)
                delay = self.retry
                while self.link.connected:
                    await asyncio.sleep(0.05)
            except (OSError, TransportError):
                pass
            await asyncio.sleep(delay)
            delay = min(delay * 2, 5.0)
