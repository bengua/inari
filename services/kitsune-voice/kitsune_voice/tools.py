"""What the voice agent can do with the person's brain: only the tools the person allowed
(Kitsune settings, voice.tools), each one a call to the core."""

from __future__ import annotations

import datetime as dt
from typing import Any, Callable

from livekit.agents import RunContext, function_tool

from .core import Core, CoreError

# the brain action each tool needs (a tool appears only when its action is allowed)
NEEDS = {
    "keep_note": "note",
    "todays_reminders": "reminders",
    "read_list": "items",
    "add_to_list": "add_items",
    "check_items": "check",
    "start_assistant": "assist",
}

ASSISTANTS = ("daily-plan", "kitsune-inbox", "groceries", "weekly-review")


def build(core: Core, allowed: list[str]) -> list[Any]:
    """The function tools the session offers, for the actions the person allowed."""

    @function_tool()
    async def keep_note(context: RunContext, text: str) -> str:
        """Keep a thought in the person's brain, in their own words (it is read, sorted and given
        a reminder when one was said)."""
        try:
            note = (await core.call("capture_text", text=text)).get("id")
            return f"kept (note {note})"
        except CoreError as e:
            return f"not kept: {e}"

    @function_tool()
    async def todays_reminders(context: RunContext) -> str:
        """The reminders still to come, soonest first."""
        try:
            got = (await core.call("reminders")).get("reminders", [])
        except CoreError as e:
            return f"unavailable: {e}"
        if not got:
            return "nothing is set to come back"
        lines = []
        for n in got[:8]:
            when = n.get("due_date") or n.get("location_raw") or ""
            lines.append(f"- {n.get('clean_summary') or n.get('raw_transcript')} ({when})")
        return "\n".join(lines)

    @function_tool()
    async def read_list(context: RunContext, list_name: str) -> str:
        """The open items of a list (for example "Courses", "Garde-manger", "Today")."""
        try:
            items = (await core.call("items", list=list_name)).get("items", [])
        except CoreError as e:
            return f"unavailable: {e}"
        return "\n".join(f"- {i['text']}" for i in items) or "the list is empty"

    @function_tool()
    async def add_to_list(context: RunContext, list_name: str, kind: str, items: list[str]) -> str:
        """Add items to a list. kind is todo, shopping, pantry or custom."""
        try:
            ids = (await core.call("add_items", list=list_name, kind=kind,
                                   items=[{"text": t} for t in items])).get("ids", [])
            return f"{len(ids)} added to {list_name}"
        except CoreError as e:
            return f"not added: {e}"

    @function_tool()
    async def check_items(context: RunContext, list_name: str, items: list[str]) -> str:
        """Mark items of a list as done, by their words."""
        try:
            open_items = (await core.call("items", list=list_name)).get("items", [])
            wanted = {w.strip().lower() for w in items}
            ids = [i["id"] for i in open_items if i["text"].strip().lower() in wanted]
            if not ids:
                return "none of those is on the list"
            await core.call("check", ids=ids, done=True)
            return f"{len(ids)} done"
        except CoreError as e:
            return f"not done: {e}"

    @function_tool()
    async def start_assistant(context: RunContext, assistant: str) -> str:
        """Start one of the person's assistants: daily-plan (plan of the day), kitsune-inbox (sort
        the notes), groceries (shopping and pantry), weekly-review. It works in its own thread."""
        if assistant not in ASSISTANTS:
            return f"unknown assistant; there are: {', '.join(ASSISTANTS)}"
        try:
            await core.call("assist", workflow=assistant, title=assistant)
            return "started; it will be in Running on the Kitsune page"
        except CoreError as e:
            return f"not started: {e}"

    every: dict[str, Callable[..., Any]] = {
        "keep_note": keep_note,
        "todays_reminders": todays_reminders,
        "read_list": read_list,
        "add_to_list": add_to_list,
        "check_items": check_items,
        "start_assistant": start_assistant,
    }
    return [f for name, f in every.items() if NEEDS[name] in allowed]


def now_line() -> str:
    """Today and the time, for the instructions (the model does no date arithmetic of its own)."""
    return dt.datetime.now().strftime("%A %d %B %Y, %H:%M")
