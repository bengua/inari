from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from inari_core.agent import (
    AgentRunner,
    Completion,
    ProviderConfig,
    ProviderError,
    ToolCall,
    make_provider,
)
from inari_core.approval_bus import NodeApprovals, PhoneApprover
from inari_core.approvals import ApprovalGate
from inari_core.capsule import load, verify
from inari_core.mesh import Bus, connect
from inari_core.sandbox import SubprocessRuntime
from inari_core.secrets import EnvResolver
from inari_core.teleport import CapsuleHost, teleport


class Scripted:
    """A provider that plays back a fixed list of completions and records what it saw."""

    kind = "openai"

    def __init__(self, *steps):
        self.steps, self.seen = list(steps), []

    async def complete(self, rendered, tools):
        self.seen.append(rendered)
        step = self.steps.pop(0)
        if isinstance(step, Exception):
            raise step
        return step


def runner(tmp_path, provider, **kw):
    host = CapsuleHost("node-a", tmp_path / "a")
    return host, AgentRunner(host, provider, SubprocessRuntime(EnvResolver()), **kw)


def kinds(host, agent):
    return [e.kind.value for e in load(host.agents / agent).timeline.ordered()]


async def test_the_agent_writes_a_file_and_the_capsule_keeps_the_work(tmp_path):
    script = Scripted(
        Completion("", [ToolCall("c1", "write_file", {"path": "hello.txt", "content": "hi"})]),
        Completion("", [ToolCall("c2", "bash", {"cmd": "cat hello.txt"})]),
        Completion("Done: the file says hi."),
    )
    host, run = runner(tmp_path, script)
    agent = run.create("t1")
    summary = await run.run(agent, "make hello.txt")
    assert summary.stopped == "done" and summary.steps == 3 and "says hi" in summary.final_text
    assert kinds(host, agent) == [
        "system",
        "user_message",
        "tool_call",
        "tool_result",
        "tool_call",
        "tool_result",
        "assistant_message",
    ]
    capsule = load(host.agents / agent)
    assert "hello.txt" in capsule.snapshot.tree and verify(capsule.root) == []
    results = [
        e.data["output"] for e in capsule.timeline.ordered() if e.kind.value == "tool_result"
    ]
    assert results == ["wrote 2 bytes", "hi"]
    # the model saw the tool result before its final answer
    assert any(m["role"] == "tool" for m in script.seen[-1]["messages"])


async def test_a_path_outside_the_workspace_is_refused_as_information_for_the_model(tmp_path):
    script = Scripted(
        Completion("", [ToolCall("c1", "read_file", {"path": "../../etc/passwd"})]),
        Completion("I cannot read that."),
    )
    host, run = runner(tmp_path, script)
    agent = run.create("t1")
    await run.run(agent, "read it")
    out = [e for e in load(host.agents / agent).timeline.ordered() if e.kind.value == "tool_result"]
    assert out[0].data["is_error"] and "leaves the workspace" in out[0].data["output"]


async def test_a_provider_failure_is_recorded_and_the_agent_can_continue_later(tmp_path):
    script = Scripted(ProviderError("HTTP 503"), Completion("recovered"))
    host, run = runner(tmp_path, script)
    agent = run.create("t1")
    first = await run.run(agent, "hello")
    assert first.stopped == "error" and "503" in first.error
    assert verify(host.agents / agent) == []
    second = await run.run(agent, "try again")
    assert second.stopped == "done" and second.final_text == "recovered"


async def test_a_command_needing_approval_waits_for_the_phone(tmp_path):
    node, phone = Bus("node"), Bus("phone")
    connect(node, phone)
    PhoneApprover(phone, b"k", lambda req: "rm" not in req.command)
    approvals = NodeApprovals(node, ApprovalGate(b"k"))
    script = Scripted(
        Completion("", [ToolCall("c1", "bash", {"cmd": "echo ok"})]),
        Completion("", [ToolCall("c2", "bash", {"cmd": "rm -rf x"})]),
        Completion("finished"),
    )
    host, run = runner(tmp_path, script, approvals=approvals)
    agent = run.create("t1", approval_required=["bash *"])
    await run.run(agent, "go")
    results = [
        e.data
        for e in load(host.agents / agent).timeline.ordered()
        if e.kind.value == "tool_result"
    ]
    assert results[0]["output"].strip() == "ok" and not results[0]["is_error"]
    assert results[1]["is_error"] and "did not approve" in results[1]["output"]


async def test_without_an_approver_a_guarded_command_is_denied(tmp_path):
    script = Scripted(Completion("", [ToolCall("c1", "bash", {"cmd": "ls"})]), Completion("ok"))
    host, run = runner(tmp_path, script)
    agent = run.create("t1", approval_required=["bash *"])
    await run.run(agent, "go")
    out = [
        e.data
        for e in load(host.agents / agent).timeline.ordered()
        if e.kind.value == "tool_result"
    ]
    assert out[0]["is_error"] and "no approver" in out[0]["output"]


async def test_freezing_during_a_run_stops_it_at_the_next_step_and_the_agent_moves(tmp_path):
    host = CapsuleHost("node-a", tmp_path / "a")
    target = CapsuleHost("node-b", tmp_path / "b")

    class Freezing(Scripted):
        async def complete(self, rendered, tools):
            result = await super().complete(rendered, tools)
            if len(self.seen) == 1:
                await host.freeze(agent)  # a teleport begins while the model is answering
            return result

    script = Freezing(
        Completion("", [ToolCall("c1", "write_file", {"path": "a.txt", "content": "1"})]),
        Completion("never asked"),
    )
    run = AgentRunner(host, script, SubprocessRuntime(EnvResolver()))
    agent = run.create("t1")
    summary = await run.run(agent, "work")
    assert summary.stopped == "frozen" and len(script.steps) == 1  # the 2nd call never happened
    await host.unfreeze(agent, 1)
    out = await teleport(host, target, agent)
    assert out.state == "committed" and verify(target.agents / agent) == []
    # the new holder continues the same conversation
    script2 = Scripted(Completion("continuing on node-b"))
    run_b = AgentRunner(target, script2, SubprocessRuntime(EnvResolver()))
    again = await run_b.run(agent, "carry on")
    assert again.stopped == "done"
    roles = [m["role"] for m in script2.seen[0]["messages"]]
    assert "tool" in roles and roles[-1] == "user"  # the history crossed the machine


async def test_a_run_publishes_events_and_phases(tmp_path):
    seen = []

    async def publish(subject, payload):
        seen.append((subject, payload.get("phase") or payload.get("kind")))

    host, run = runner(tmp_path, Scripted(Completion("hi")), publish=publish)
    agent = run.create("t1")
    await run.run(agent, "hello")
    subjects = [s for s, _ in seen]
    assert "agent.t1.event" in subjects
    phases = [v for s, v in seen if s == "agent.t1.state"]
    assert phases == ["listening", "inference", "success", "idle"]


# -- the HTTP adapters, against a local fake -----------------------------------
async def fake_server(reply):
    requests = []

    async def handle(reader, writer):
        head = await reader.readuntil(b"\r\n\r\n")
        lines = head.decode().split("\r\n")
        headers = {k.lower(): v for k, v in (ln.split(": ", 1) for ln in lines[1:] if ": " in ln)}
        body = json.loads(await reader.readexactly(int(headers["content-length"])))
        requests.append((lines[0].split()[1], headers, body))
        data = json.dumps(reply).encode()
        writer.write(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
            + f"Content-Length: {len(data)}\r\nConnection: close\r\n\r\n".encode()
            + data
        )
        await writer.drain()
        writer.close()

    server = await asyncio.start_server(handle, "127.0.0.1", 0)
    return server, server.sockets[0].getsockname()[1], requests


async def test_the_openai_adapter_speaks_the_chat_completions_shape(monkeypatch):
    monkeypatch.setenv("INARI_SECRET_GW_KEY", "sk-test")
    reply = {
        "choices": [
            {
                "message": {
                    "content": "ok",
                    "tool_calls": [
                        {
                            "id": "x1",
                            "type": "function",
                            "function": {"name": "bash", "arguments": '{"cmd": "ls"}'},
                        }
                    ],
                }
            }
        ]
    }
    server, port, seen = await fake_server(reply)
    provider = make_provider(
        ProviderConfig("openai", f"http://127.0.0.1:{port}/v1", "m1", "GW_KEY"), EnvResolver()
    )
    from inari_core.agent import SPECS

    done = await provider.complete({"messages": [{"role": "user", "content": "hi"}]}, SPECS)
    server.close()
    path, headers, body = seen[0]
    assert path == "/v1/chat/completions" and headers["authorization"] == "Bearer sk-test"
    assert body["model"] == "m1" and body["tools"][0]["function"]["name"] == "bash"
    assert done.text == "ok" and done.tool_calls[0].arguments == {"cmd": "ls"}


async def test_the_anthropic_adapter_speaks_the_messages_shape(monkeypatch):
    monkeypatch.setenv("INARI_SECRET_A_KEY", "ak-test")
    reply = {
        "content": [
            {"type": "text", "text": "hello"},
            {"type": "tool_use", "id": "t9", "name": "bash", "input": {"cmd": "pwd"}},
        ]
    }
    server, port, seen = await fake_server(reply)
    provider = make_provider(
        ProviderConfig("anthropic", f"http://127.0.0.1:{port}", "m2", "A_KEY"), EnvResolver()
    )
    done = await provider.complete({"system": "be brief", "messages": []}, [])
    server.close()
    path, headers, body = seen[0]
    assert path == "/v1/messages" and headers["x-api-key"] == "ak-test"
    assert body["system"] == "be brief" and "tools" not in body
    assert done.text == "hello" and done.tool_calls[0].call_id == "t9"


async def test_an_unreachable_provider_raises_a_clean_error():
    provider = make_provider(ProviderConfig("openai", "http://127.0.0.1:1/v1", "m"), EnvResolver())
    with pytest.raises(ProviderError):
        await provider.complete({"messages": []}, [])


def test_unknown_provider_kind_is_refused():
    with pytest.raises(ValueError):
        make_provider(ProviderConfig(kind="nope"), EnvResolver())


def _unused(_: Path):  # keeps the Path import honest for type readers
    return None
