"""One-gesture approval of sensitive commands from a phone.

A request is signed by the device with a key it shares with the node. A response
is accepted once, before it expires, and only if its signature matches; silence
means no."""

from __future__ import annotations

import asyncio
import fnmatch
import hmac
import time
import uuid
from collections.abc import Callable
from hashlib import sha256

from pydantic import BaseModel

from .canon import canonical


class ApprovalRequest(BaseModel):
    request_id: str
    agent_id: str
    node_id: str
    command: str
    nonce: str
    expires_at: float


class ApprovalResponse(BaseModel):
    request_id: str
    approved: bool
    signature: str


def needs_approval(command: str, patterns: list[str]) -> bool:
    return any(fnmatch.fnmatch(command, p) for p in patterns)


def sign(request: ApprovalRequest, approved: bool, key: bytes) -> str:
    body = canonical({"id": request.request_id, "n": request.nonce, "ok": approved})
    return hmac.new(key, body, sha256).hexdigest()


class ApprovalGate:
    def __init__(self, key: bytes, clock: Callable[[], float] = time.time) -> None:
        self._key, self._clock = key, clock
        self._pending: dict[str, tuple[ApprovalRequest, asyncio.Future[bool]]] = {}

    def open(self, agent_id: str, node_id: str, command: str, ttl: float = 60.0):
        request = ApprovalRequest(
            request_id=uuid.uuid4().hex,
            agent_id=agent_id,
            node_id=node_id,
            command=command,
            nonce=uuid.uuid4().hex,
            expires_at=self._clock() + ttl,
        )
        future: asyncio.Future[bool] = asyncio.get_running_loop().create_future()
        self._pending[request.request_id] = (request, future)
        return request, future

    def resolve(self, response: ApprovalResponse) -> bool:
        """Accept a response. Returns False for anything that is not valid, fresh
        and unused; a rejected response changes nothing."""
        entry = self._pending.get(response.request_id)
        if entry is None:
            return False
        request, future = entry
        if self._clock() > request.expires_at:
            return False
        expected = sign(request, response.approved, self._key)
        if not hmac.compare_digest(expected, response.signature):
            return False
        del self._pending[response.request_id]
        if not future.done():
            future.set_result(response.approved)
        return True

    async def decide(self, future: asyncio.Future[bool], timeout: float) -> bool:
        try:
            return await asyncio.wait_for(future, timeout)
        except TimeoutError:
            return False  # default deny
