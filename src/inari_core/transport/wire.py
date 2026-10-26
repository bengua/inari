"""Authenticated framing over a stream.

Handshake: each side sends a random nonce and proves knowledge of the shared key
by returning HMAC(key, own-nonce || peer-nonce || role). Every later frame
carries an HMAC under a session key, so frames cannot be altered, injected or
replayed on another connection. There is NO encryption: run it over a private
network (WireGuard, Tailscale) or a TLS tunnel."""

from __future__ import annotations

import asyncio
import hashlib
import hmac
import json
import os
import struct
from typing import Any

from ..errors import TransportError

MAX_FRAME = 32 * 1024 * 1024


def _mac(key: bytes, *parts: bytes) -> bytes:
    return hmac.new(key, b"|".join(parts), hashlib.sha256).digest()


class Wire:
    def __init__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter, key: bytes):
        self.reader, self.writer, self._key = reader, writer, key
        self._session = b""
        self._seq_out = self._seq_in = 0
        self.peer: dict[str, Any] = {}

    async def handshake(self, role: str, hello: dict[str, Any], timeout: float = 5.0) -> None:
        """`role` is "client" or "server"; both sides run this concurrently."""
        mine = os.urandom(16)
        try:
            self.writer.write(mine)
            await self.writer.drain()
            theirs = await asyncio.wait_for(self.reader.readexactly(16), timeout)
            nonces = (mine, theirs) if role == "client" else (theirs, mine)
            self._session = _mac(self._key, b"session", *nonces)
            await self.send({"hello": hello})
            reply = await asyncio.wait_for(self.recv(), timeout)
        except (OSError, asyncio.IncompleteReadError, TimeoutError) as exc:
            raise TransportError(f"handshake failed: {exc}") from exc
        self.peer = reply["hello"]

    async def send(self, obj: dict[str, Any]) -> None:
        body = json.dumps(obj, separators=(",", ":")).encode()
        if len(body) > MAX_FRAME:
            raise TransportError("frame too large")
        tag = _mac(self._session, struct.pack(">Q", self._seq_out), body)
        self._seq_out += 1
        try:
            self.writer.write(struct.pack(">I", len(body)) + tag + body)
            await self.writer.drain()
        except OSError as exc:
            raise TransportError(f"send failed: {exc}") from exc

    async def recv(self) -> dict[str, Any]:
        try:
            (size,) = struct.unpack(">I", await self.reader.readexactly(4))
            if size > MAX_FRAME:
                raise TransportError("frame too large")
            tag = await self.reader.readexactly(32)
            body = await self.reader.readexactly(size)
        except (OSError, asyncio.IncompleteReadError) as exc:
            raise TransportError(f"connection lost: {exc}") from exc
        expected = _mac(self._session, struct.pack(">Q", self._seq_in), body)
        if not hmac.compare_digest(tag, expected):
            raise TransportError("bad frame signature (wrong key or tampering)")
        self._seq_in += 1
        return json.loads(body)

    async def close(self) -> None:
        self.writer.close()
        try:
            await self.writer.wait_closed()
        except OSError:
            pass
