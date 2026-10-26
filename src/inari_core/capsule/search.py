"""Search a journal: exact words first, then the hashing index for near matches."""

from __future__ import annotations

from .timeline import Timeline
from .vectors import VectorIndex


def event_text(data: dict) -> str:
    return " ".join(
        str(data[k]) for k in ("text", "name", "output", "arguments") if k in data and data[k]
    )


def tool_title(data: dict) -> str:
    """A tool call as a person reads it: `write_file: notes.txt`, `bash: ls -la`,
    `$ make` for a command the person typed. Never a raw argument dump."""
    name = str(data.get("name") or "tool")
    args = data.get("arguments") or {}
    if not isinstance(args, dict):
        return name
    if args.get("by") == "user":
        return f"$ {args.get('cmd', '')}"
    inner = args.get("input") if isinstance(args.get("input"), dict) else args
    detail = ""
    for key in ("cmd", "command", "path", "file_path", "url", "query", "pattern"):
        if inner.get(key):
            detail = str(inner[key])
            break
    if ":" in name or not detail:
        return name
    return f"{name}: {detail}"


def event_summary(kind: str, data: dict) -> str:
    """One line for an event, the same everywhere: interface, history, search."""
    if kind == "tool_call":
        return tool_title(data)
    if kind == "provider_switch":
        return str(data.get("to") or "")
    if kind == "checkpoint":
        reason, detail = str(data.get("reason") or ""), str(data.get("detail") or "")
        return f"{reason}: {detail}" if detail else reason
    return event_text(data)


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
