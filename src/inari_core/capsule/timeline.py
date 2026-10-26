"""The append-only journal: a grow-only set of events, merged by union."""

from __future__ import annotations

import json
from collections.abc import Iterable
from typing import Any

from ..canon import digest_of
from ..errors import ConflictError
from .models import EventKind, TimelineEvent


class Timeline:
    def __init__(self, events: Iterable[TimelineEvent] = ()) -> None:
        self._by_id: dict[str, TimelineEvent] = {}
        for event in events:
            self.add(event)

    def __len__(self) -> int:
        return len(self._by_id)

    def add(self, event: TimelineEvent) -> bool:
        """Insert an event. Returns False if it was already known."""
        known = self._by_id.get(event.id)
        if known is None:
            self._by_id[event.id] = event
            return True
        if known.hash != event.hash:
            raise ConflictError(f"two different events share the id {event.id}")
        return False

    def ordered(self) -> list[TimelineEvent]:
        return sorted(
            self._by_id.values(), key=lambda e: (e.lamport, e.node_id, e.node_seq)
        )

    def clock(self) -> int:
        return max((e.lamport for e in self._by_id.values()), default=0)

    def _last(self, node_id: str) -> TimelineEvent | None:
        mine = [e for e in self._by_id.values() if e.node_id == node_id]
        return max(mine, key=lambda e: e.node_seq, default=None)

    def append(
        self, node_id: str, kind: EventKind, data: dict[str, Any], ts: float
    ) -> TimelineEvent:
        last = self._last(node_id)
        event = TimelineEvent.make(
            node_id=node_id,
            node_seq=(last.node_seq + 1) if last else 1,
            lamport=self.clock() + 1,
            ts=ts,
            kind=kind,
            data=data,
            prev=last.hash if last else "",
        )
        self.add(event)
        return event

    def merge(self, other: Timeline) -> int:
        """Union with another replica. Commutative, associative, idempotent."""
        return sum(self.add(e) for e in other._by_id.values())

    def verify(self) -> list[str]:
        """Problems found, empty when every per-node chain is intact."""
        problems: list[str] = []
        by_node: dict[str, list[TimelineEvent]] = {}
        for event in self._by_id.values():
            by_node.setdefault(event.node_id, []).append(event)
        for node, events in sorted(by_node.items()):
            events.sort(key=lambda e: e.node_seq)
            prev = ""
            for expected, event in enumerate(events, start=1):
                if event.node_seq != expected:
                    problems.append(f"{node}: gap before seq {event.node_seq}")
                    break
                if event.hash != event.compute_hash():
                    problems.append(f"{event.id}: hash does not match content")
                if event.prev != prev:
                    problems.append(f"{event.id}: broken chain")
                prev = event.hash
        return problems

    def head(self) -> str:
        """Order-independent fingerprint of the whole journal."""
        return digest_of(sorted((e.id, e.hash) for e in self._by_id.values()))

    def dumps(self) -> str:
        return "".join(
            json.dumps(e.model_dump(mode="json"), sort_keys=True) + "\n"
            for e in self.ordered()
        )

    @classmethod
    def loads(cls, text: str) -> Timeline:
        return cls(
            TimelineEvent.model_validate_json(line)
            for line in text.splitlines()
            if line.strip()
        )
