"""Search a journal: exact words first, then the hashing index for near matches."""

from __future__ import annotations

from .timeline import Timeline
from .vectors import VectorIndex


def event_text(data: dict) -> str:
    return " ".join(
        str(data[k]) for k in ("text", "name", "output", "arguments") if k in data and data[k]
    )


def search(timeline: Timeline, query: str, k: int = 8) -> list[dict]:
    events = {e.id: e for e in timeline.ordered()}
    needle = query.lower().strip()
    exact = [i for i, e in events.items() if needle and needle in event_text(e.data).lower()]
    index = VectorIndex()
    for i, e in events.items():
        index.add(i, event_text(e.data))
    ranked = [i for i, score in index.search(query, k=k) if score > 0.05 and i not in exact]
    return [
        {"id": i, "kind": events[i].kind.value, "text": event_text(events[i].data)[:160]}
        for i in [*exact, *ranked][:k]
    ]
