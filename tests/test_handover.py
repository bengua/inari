from __future__ import annotations

import json

from inari_core.capsule import EventKind, Timeline
from inari_core.handover import compact, estimate_tokens, render


def journal(*, dangling: bool = False) -> Timeline:
    t = Timeline()
    t.append("n", EventKind.SYSTEM, {"text": "You are an agent."}, 1)
    t.append("n", EventKind.USER_MESSAGE, {"text": "list files"}, 2)
    t.append("n", EventKind.THOUGHT, {"text": "private reasoning"}, 3)
    t.append(
        "n",
        EventKind.TOOL_CALL,
        {"call_id": "c1", "name": "bash", "arguments": {"cmd": "ls"}},
        4,
    )
    if not dangling:
        t.append(
            "n",
            EventKind.TOOL_RESULT,
            {"call_id": "c1", "output": "a b", "is_error": False},
            5,
        )
        t.append("n", EventKind.ASSISTANT_MESSAGE, {"text": "two files"}, 6)
    return t


def test_anthropic_alignment():
    out = render(journal().ordered(), "anthropic")
    assert out["system"] == "You are an agent."
    roles = [m["role"] for m in out["messages"]]
    assert roles == ["user", "assistant", "user", "assistant"]
    assert out["messages"][1]["content"][0]["type"] == "tool_use"
    result = out["messages"][2]["content"][0]
    assert result["type"] == "tool_result" and result["tool_use_id"] == "c1"


def test_openai_alignment():
    out = render(journal().ordered(), "openai")["messages"]
    assert out[0]["role"] == "system"
    call = next(m for m in out if m.get("tool_calls"))["tool_calls"][0]
    assert call["function"]["name"] == "bash"
    assert json.loads(call["function"]["arguments"]) == {"cmd": "ls"}
    assert {"role": "tool", "tool_call_id": "c1", "content": "a b"} in out


def test_reasoning_is_dropped_across_providers_but_kept_within_one():
    events = journal().ordered()
    assert "private reasoning" not in json.dumps(render(events, "anthropic"))
    assert "private reasoning" in json.dumps(
        render(events, "anthropic", same_provider=True)
    )


def test_a_call_cut_short_by_a_crash_gets_an_error_result():
    out = render(journal(dangling=True).ordered(), "anthropic")
    last = out["messages"][-1]
    assert last["role"] == "user"
    block = last["content"][0]
    assert block["tool_use_id"] == "c1" and block["is_error"] is True
    openai = render(journal(dangling=True).ordered(), "openai")["messages"]
    assert openai[-1]["role"] == "tool"


def test_rendering_round_trips_between_providers_without_loss():
    events = journal().ordered()
    a = render(events, "anthropic")
    b = render(events, "openai")
    again = render(events, "anthropic")
    assert a == again  # pure function of the journal
    assert len(b["messages"]) >= 4


def test_compaction_keeps_pairs_whole_and_adds_a_digest():
    t = Timeline()
    for i in range(12):
        t.append("n", EventKind.USER_MESSAGE, {"text": f"question {i} " * 20}, i)
        t.append(
            "n",
            EventKind.TOOL_CALL,
            {"call_id": f"c{i}", "name": "bash", "arguments": {"cmd": "x" * 80}},
            i + 0.1,
        )
        t.append(
            "n",
            EventKind.TOOL_RESULT,
            {"call_id": f"c{i}", "output": "ok", "is_error": False},
            i + 0.2,
        )
    events = t.ordered()
    out = compact(events, budget_tokens=400, working_memory="goal: finish")
    assert out[0].kind is EventKind.SYSTEM and "Handover digest" in out[0].data["text"]
    assert "goal: finish" in out[0].data["text"]
    assert len(out) < len(events)
    calls = {e.data["call_id"] for e in out if e.kind is EventKind.TOOL_CALL}
    results = {e.data["call_id"] for e in out if e.kind is EventKind.TOOL_RESULT}
    assert calls == results
    total = sum(estimate_tokens(json.dumps(e.data)) for e in out)
    assert total <= 400


def test_small_journals_are_left_alone():
    events = journal().ordered()
    assert compact(events, budget_tokens=10_000) == events
