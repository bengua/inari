"""What a node must offer to take part in a teleportation."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Protocol

from ..capsule.models import Manifest

BLOB_PREFIX = "artifacts/blobs/"


@dataclass(frozen=True)
class FrozenAgent:
    manifest: Manifest
    epoch: int


@dataclass
class Outcome:
    session_id: str
    state: str
    epoch: int = 0
    bytes_sent: int = 0
    files_sent: int = 0
    blobs_skipped: int = 0
    error: str = ""
    needs_attention: bool = False
    history: list[str] = field(default_factory=list)


class Endpoint(Protocol):
    node_id: str

    # Source side.
    async def freeze(self, agent_id: str) -> FrozenAgent: ...
    async def read(self, agent_id: str, rel: str) -> bytes: ...
    async def unfreeze(self, agent_id: str, epoch: int) -> None: ...
    async def tombstone(self, agent_id: str, epoch: int) -> None: ...

    # Target side.
    async def offer(self, session_id: str, manifest: Manifest, epoch: int) -> set[str]: ...
    async def receive(self, session_id: str, rel: str, data: bytes) -> None: ...
    async def verify(self, session_id: str) -> None: ...
    async def prepare(self, session_id: str) -> None: ...
    async def activate(self, session_id: str, epoch: int) -> None: ...
    async def abort(self, session_id: str) -> None: ...

    # Both.
    async def holder(self, agent_id: str) -> tuple[str, int] | None: ...
