"""An embedded event bus with NATS-style subjects, and links between buses.

`*` matches one token, `>` matches the rest. Handlers are isolated: one that
raises never stops the others (crash-only: the failure is counted, not hidden).
Links carry messages between nodes; while a link is down, durable subjects are
queued and replayed on reconnect, and receivers drop duplicates by message id."""

from __future__ import annotations

import asyncio
import inspect
import uuid
from collections import OrderedDict, deque
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

from ..errors import TransportError


def subject_matches(pattern: str, subject: str) -> bool:
    p, s = pattern.split("."), subject.split(".")
    for i, token in enumerate(p):
        if token == ">":
            return i == len(p) - 1 and len(s) > i
        if i >= len(s) or (token != "*" and token != s[i]):
            return False
    return len(p) == len(s)


@dataclass(frozen=True)
class Message:
    subject: str
    payload: dict[str, Any]
    sender: str = ""
    msg_id: str = field(default_factory=lambda: uuid.uuid4().hex)
    path: tuple[str, ...] = ()  # buses already crossed: stops forwarding loops


Handler = Callable[[Message], Awaitable[None] | None]


class Subscription:
    def __init__(self, bus: Bus, pattern: str, handler: Handler) -> None:
        self.bus, self.pattern, self.handler = bus, pattern, handler

    def cancel(self) -> None:
        self.bus._subs.remove(self)


class Bus:
    def __init__(self, name: str, *, seen_limit: int = 4096) -> None:
        self.name = name
        self._subs: list[Subscription] = []
        self._links: list[Link] = []
        self._seen: OrderedDict[str, None] = OrderedDict()
        self._seen_limit = seen_limit
        self.handler_errors = 0

    def subscribe(self, pattern: str, handler: Handler) -> Subscription:
        sub = Subscription(self, pattern, handler)
        self._subs.append(sub)
        return sub

    def add_link(self, link: Link) -> None:
        self._links.append(link)

    async def publish(self, subject: str, payload: dict[str, Any] | None = None) -> int:
        message = Message(subject, payload or {}, sender=self.name, path=(self.name,))
        return await self._dispatch(message, forward=True)

    async def deliver(self, message: Message) -> int:
        """Entry point for messages arriving over a link."""
        if message.msg_id in self._seen:
            return 0
        return await self._dispatch(message, forward=True)

    async def _dispatch(self, message: Message, *, forward: bool) -> int:
        self._seen[message.msg_id] = None
        while len(self._seen) > self._seen_limit:
            self._seen.popitem(last=False)
        delivered = 0
        for sub in list(self._subs):
            if subject_matches(sub.pattern, message.subject):
                try:
                    result = sub.handler(message)
                    if inspect.isawaitable(result):
                        await result
                    delivered += 1
                except Exception:  # noqa: BLE001 - isolation is the point
                    self.handler_errors += 1
        if forward:
            for link in self._links:
                if link.peer_name not in message.path:
                    await link.send(
                        Message(
                            message.subject,
                            message.payload,
                            message.sender,
                            message.msg_id,
                            (*message.path, self.name),
                        )
                    )
        return delivered


class Link:
    """One direction of a connection to a peer bus. Durable subjects survive a
    partition; everything else is dropped while the link is down."""

    def __init__(
        self,
        peer: Bus,
        *,
        durable: tuple[str, ...] = ("teleport.>", "approval.>", "agent.>"),
        outbox_limit: int = 10_000,
    ) -> None:
        self.peer = peer
        self.peer_name = peer.name
        self.durable = durable
        self.connected = True
        self.outbox: deque[Message] = deque(maxlen=outbox_limit)

    async def send(self, message: Message) -> None:
        if not self.connected:
            if any(subject_matches(p, message.subject) for p in self.durable):
                self.outbox.append(message)
            return
        await self.peer.deliver(message)

    def disconnect(self) -> None:
        self.connected = False

    async def reconnect(self) -> int:
        self.connected = True
        flushed = 0
        while self.outbox:
            await self.peer.deliver(self.outbox.popleft())
            flushed += 1
        return flushed


def connect(a: Bus, b: Bus, **kwargs: Any) -> tuple[Link, Link]:
    """Wire two buses together in both directions."""
    ab, ba = Link(b, **kwargs), Link(a, **kwargs)
    a.add_link(ab)
    b.add_link(ba)
    return ab, ba


async def request(
    bus: Bus, subject: str, payload: dict[str, Any], reply: str, timeout: float
) -> Message:
    """Publish and wait for one message on `reply`; raises TransportError."""
    loop = asyncio.get_running_loop()
    future: asyncio.Future[Message] = loop.create_future()
    sub = bus.subscribe(reply, lambda m: future.done() or future.set_result(m))
    try:
        await bus.publish(subject, payload)
        return await asyncio.wait_for(future, timeout)
    except TimeoutError as exc:
        raise TransportError(f"no reply on {reply}") from exc
    finally:
        sub.cancel()
