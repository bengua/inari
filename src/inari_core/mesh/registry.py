"""Node discovery. Nodes announce themselves and heartbeat; the registry keeps
the live ones and merges views from peers (last write wins per node, ordered
by incarnation then time), so every node converges on the same picture."""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import asdict, dataclass, field
from enum import StrEnum
from typing import Any


class Role(StrEnum):
    HEAVY = "heavy"  # workstation or server: sandboxes, inference, builds
    CONTROL = "control"  # a desk that supervises
    MOBILE = "mobile"  # thin client: dashboard, approvals, remote control


@dataclass
class NodeInfo:
    node_id: str
    role: Role
    incarnation: int = 1  # bumped on every restart of the node
    seen_at: float = 0.0
    capabilities: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {**asdict(self), "role": self.role.value}

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> NodeInfo:
        return cls(**{**raw, "role": Role(raw["role"])})


class Registry:
    def __init__(self, *, ttl: float = 15.0, clock: Callable[[], float] = time.monotonic):
        self.ttl, self.clock = ttl, clock
        self._nodes: dict[str, NodeInfo] = {}

    def observe(self, info: NodeInfo) -> bool:
        """Record a sighting. Returns True if it changed what we know."""
        info = NodeInfo(**{**info.__dict__, "seen_at": self.clock()})
        known = self._nodes.get(info.node_id)
        if known and known.incarnation > info.incarnation:
            return False  # a stale announcement from before a restart
        changed = known is None or known.incarnation != info.incarnation
        self._nodes[info.node_id] = info
        return changed

    def alive(self) -> list[NodeInfo]:
        now = self.clock()
        return sorted(
            (n for n in self._nodes.values() if now - n.seen_at <= self.ttl),
            key=lambda n: n.node_id,
        )

    def get(self, node_id: str) -> NodeInfo | None:
        return next((n for n in self.alive() if n.node_id == node_id), None)

    def expired(self) -> list[str]:
        now = self.clock()
        return sorted(n.node_id for n in self._nodes.values() if now - n.seen_at > self.ttl)

    def snapshot(self) -> list[dict[str, Any]]:
        return [n.to_dict() for n in self.alive()]

    def merge(self, snapshot: list[dict[str, Any]]) -> int:
        return sum(self.observe(NodeInfo.from_dict(raw)) for raw in snapshot)
