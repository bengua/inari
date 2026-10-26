"""Semantic compression for a move to a narrower context window."""

from __future__ import annotations

import json
import math
from typing import Protocol

from ..capsule.models import EventKind, TimelineEvent


def estimate_tokens(text: str) -> int:
    """Cheap, provider-independent estimate (about four characters a token)."""
    return math.ceil(len(text) / 4)


def event_tokens(event: TimelineEvent) -> int:
    return estimate_tokens(json.dumps(event.data, sort_keys=True)) + 4


class Summarizer(Protocol):
    def summarize(self, dropped: list[TimelineEvent], working_memory: str) -> str: ...


class ExtractiveSummarizer:
    """Deterministic and offline: what was decided and which tools ran. A model
    backed summarizer can replace it behind the same protocol."""

    def summarize(self, dropped: list[TimelineEvent], working_memory: str) -> str:
        decisions = [str(e.data.get("text", "")) for e in dropped if e.kind is EventKind.DECISION]
        tools = sorted({str(e.data.get("name")) for e in dropped if e.kind is EventKind.TOOL_CALL})
        lines = [f"Handover digest: {len(dropped)} earlier events were condensed."]
        if tools:
            lines.append("Tools used: " + ", ".join(tools) + ".")
        lines += [f"Decision: {d}" for d in decisions]
        if working_memory.strip():
            lines.append("Working memory:\n" + working_memory.strip())
        return "\n".join(lines)


def compact(
    events: list[TimelineEvent],
    *,
    budget_tokens: int,
    working_memory: str = "",
    summarizer: Summarizer | None = None,
    reserve: float = 0.3,
) -> list[TimelineEvent]:
    """Keep the newest events that fit, never splitting a tool call from its
    result, and prepend a digest of what was dropped. `reserve` leaves room for
    the digest and the model's answer."""
    summarizer = summarizer or ExtractiveSummarizer()
    usable = int(budget_tokens * (1 - reserve))
    if sum(event_tokens(e) for e in events) <= usable:
        return list(events)
    kept: list[TimelineEvent] = []
    used = 0
    for event in reversed(events):
        cost = event_tokens(event)
        if used + cost > usable:
            break
        kept.append(event)
        used += cost
    kept.reverse()
    # Start the kept tail on a turn boundary, so no result is left without its call.
    while kept and kept[0].kind not in (EventKind.USER_MESSAGE, EventKind.SYSTEM):
        kept.pop(0)
    dropped = events[: len(events) - len(kept)]
    digest = summarizer.summarize(dropped, working_memory)
    first = events[0]
    note = TimelineEvent.make(
        node_id="handover",
        node_seq=1,
        lamport=first.lamport,
        ts=first.ts,
        kind=EventKind.SYSTEM,
        data={"text": digest},
    )
    return [note, *kept]
