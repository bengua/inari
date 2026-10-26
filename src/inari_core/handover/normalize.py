"""Provider handover: turn the neutral journal into what a given provider expects.

The journal stores what happened, not how one vendor spells it. Rendering is a
pure function of (events, target), so a thread can move between providers at
any point and move back again without loss."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Literal

from ..capsule.models import EventKind, TimelineEvent

INTERRUPTED = "Interrupted: this call never returned a result (node handover)."


@dataclass
class Block:
    type: Literal["text", "tool_use", "tool_result"]
    text: str = ""
    call_id: str = ""
    name: str = ""
    arguments: dict[str, Any] = field(default_factory=dict)
    is_error: bool = False


@dataclass
class Message:
    role: Literal["user", "assistant"]
    blocks: list[Block]


@dataclass
class Rendered:
    system: str
    messages: list[Message]


def to_messages(events: list[TimelineEvent], *, same_provider: bool = False) -> Rendered:
    """Provider-neutral conversation. Reasoning is private to the model that
    produced it, so it is dropped when the provider changes."""
    system: list[str] = []
    messages: list[Message] = []
    pending: dict[str, Block] = {}

    def close_pending() -> None:
        # A call with no result (the node died mid-call) gets an explicit error
        # result: every provider rejects a dangling tool call.
        if pending:
            messages.append(
                Message(
                    "user",
                    [
                        Block("tool_result", text=INTERRUPTED, call_id=cid, is_error=True)
                        for cid in pending
                    ],
                )
            )
            pending.clear()

    for event in events:
        kind, data = event.kind, event.data
        if kind is EventKind.SYSTEM:
            system.append(str(data.get("text", "")))
        elif kind is EventKind.USER_MESSAGE:
            close_pending()
            messages.append(Message("user", [Block("text", text=str(data["text"]))]))
        elif kind in (EventKind.ASSISTANT_MESSAGE, EventKind.DECISION):
            close_pending()
            text = str(data.get("text", ""))
            if kind is EventKind.DECISION:
                text = f"[decision] {text}"
            messages.append(Message("assistant", [Block("text", text=text)]))
        elif kind is EventKind.THOUGHT and same_provider:
            messages.append(Message("assistant", [Block("text", text=str(data.get("text", "")))]))
        elif kind is EventKind.TOOL_CALL:
            block = Block(
                "tool_use",
                call_id=str(data["call_id"]),
                name=str(data["name"]),
                arguments=dict(data.get("arguments", {})),
            )
            pending[block.call_id] = block
            messages.append(Message("assistant", [block]))
        elif kind is EventKind.TOOL_RESULT:
            call_id = str(data["call_id"])
            if call_id not in pending:
                continue  # orphan result: nothing to attach it to
            del pending[call_id]
            messages.append(
                Message(
                    "user",
                    [
                        Block(
                            "tool_result",
                            text=str(data.get("output", "")),
                            call_id=call_id,
                            is_error=bool(data.get("is_error", False)),
                        )
                    ],
                )
            )
        # PROVIDER_SWITCH and CHECKPOINT carry no conversational content.
    close_pending()
    return Rendered("\n\n".join(s for s in system if s), messages)


def _merge_same_role(messages: list[Message]) -> list[Message]:
    merged: list[Message] = []
    for message in messages:
        if merged and merged[-1].role == message.role:
            merged[-1].blocks.extend(message.blocks)
        else:
            merged.append(Message(message.role, list(message.blocks)))
    return merged


def to_anthropic(rendered: Rendered) -> dict[str, Any]:
    """Strict user/assistant alternation, tool results inside user turns."""
    messages = _merge_same_role(rendered.messages)
    if not messages or messages[0].role != "user":
        messages.insert(0, Message("user", [Block("text", text="(conversation resumed)")]))
    out: list[dict[str, Any]] = []
    for message in messages:
        content: list[dict[str, Any]] = []
        for b in message.blocks:
            if b.type == "text":
                content.append({"type": "text", "text": b.text})
            elif b.type == "tool_use":
                content.append(
                    {
                        "type": "tool_use",
                        "id": b.call_id,
                        "name": b.name,
                        "input": b.arguments,
                    }
                )
            else:
                content.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": b.call_id,
                        "content": b.text,
                        "is_error": b.is_error,
                    }
                )
        out.append({"role": message.role, "content": content})
    return {"system": rendered.system, "messages": out}


def to_openai(rendered: Rendered) -> dict[str, Any]:
    """Chat-completions shape: tool calls on the assistant turn, one `tool`
    message per result."""
    out: list[dict[str, Any]] = []
    if rendered.system:
        out.append({"role": "system", "content": rendered.system})
    for message in _merge_same_role(rendered.messages):
        if message.role == "assistant":
            texts = [b.text for b in message.blocks if b.type == "text"]
            calls = [
                {
                    "id": b.call_id,
                    "type": "function",
                    "function": {"name": b.name, "arguments": json.dumps(b.arguments)},
                }
                for b in message.blocks
                if b.type == "tool_use"
            ]
            entry: dict[str, Any] = {
                "role": "assistant",
                "content": "\n".join(texts) or None,
            }
            if calls:
                entry["tool_calls"] = calls
            out.append(entry)
        else:
            for b in message.blocks:
                if b.type == "tool_result":
                    out.append({"role": "tool", "tool_call_id": b.call_id, "content": b.text})
                else:
                    out.append({"role": "user", "content": b.text})
    return {"messages": out}


RENDERERS = {"anthropic": to_anthropic, "openai": to_openai}


def render(
    events: list[TimelineEvent], target: str, *, same_provider: bool = False
) -> dict[str, Any]:
    if target not in RENDERERS:
        raise ValueError(f"unknown provider schema: {target}")
    return RENDERERS[target](to_messages(events, same_provider=same_provider))
