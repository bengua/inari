from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path

from inari_core.agent import AcpHarness, AgentRunner
from inari_core.approval_bus import NodeApprovals, PhoneApprover
from inari_core.approvals import ApprovalGate
from inari_core.capsule import load, verify
from inari_core.mesh import Bus, connect
from inari_core.sandbox import SubprocessRuntime
from inari_core.secrets import EnvResolver
from inari_core.teleport import CapsuleHost, teleport

FAKE = str(Path(__file__).with_name("fake_acp.py"))


def make(tmp_path, name="a", **kw):
    host = CapsuleHost(f"node-{name}", tmp_path / name)
    log = tmp_path / f"{name}.log"
    harness = AcpHarness(
        [sys.executable, FAKE],
        host.root / "acp.json",
        extra_env={"FAKE_ACP_LOG": str(log)},
        **kw.pop("h", {}),
    )
    runner = AgentRunner(host, None, SubprocessRuntime(EnvResolver()), harness=harness, **kw)
    return host, runner, log


def prompts(log):
    return [json.loads(x) for x in log.read_text().splitlines()]


def kinds(host, agent):
    return [e.kind.value for e in load(host.agents / agent).timeline.ordered()]


async def test_a_turn_is_journaled_and_the_workspace_snapshotted(tmp_path):
    host, run, log = make(tmp_path)
    agent = run.create("t1")
    out = await run.run(agent, "please make a file")
    assert out.stopped == "done" and out.final_text.startswith("fake reply to:")
    assert kinds(host, agent) == [
        "system",
        "user_message",
        "tool_call",
        "tool_result",
        "assistant_message",
    ]
    events = load(host.agents / agent).timeline.ordered()
    assert "secret thoughts" not in json.dumps([e.data for e in events])  # reasoning stays private
    assert events[3].data["output"] == "ok written" and not events[3].data["is_error"]
    capsule = load(host.agents / agent)
    assert "made.txt" in capsule.snapshot.tree and verify(capsule.root) == []
    assert prompts(log)[0]["cwd"].endswith(f"work/{agent}")


async def test_a_permission_request_goes_to_the_phone(tmp_path):
    node, phone = Bus("node"), Bus("phone")
    connect(node, phone)
    PhoneApprover(phone, b"k", lambda req: "rm" in req.command)
    host, run, _ = make(tmp_path, approvals=NodeApprovals(node, ApprovalGate(b"k")))
    agent = run.create("t1")
    out = await run.run(agent, "perm please")
    assert "permission result: allow" in out.final_text
    # and a refusal reaches the agent as a refusal
    host2, run2, _ = make(tmp_path, "b")
    agent2 = run2.create("t1")
    out2 = await run2.run(agent2, "perm please")  # no approver connected
    assert "permission result: deny" in out2.final_text


async def test_the_session_is_reused_on_the_same_node_without_repeating_the_history(tmp_path):
    host, run, log = make(tmp_path)
    agent = run.create("t1")
    await run.run(agent, "first question")
    await run.run(agent, "second question")
    first, second = prompts(log)
    assert first["prompt"] == "first question" and second["prompt"] == "second question"


async def test_after_a_teleport_the_new_machine_primes_its_agent_with_the_journal(tmp_path):
    a, run_a, log_a = make(tmp_path, "a")
    b, run_b, log_b = make(tmp_path, "b")
    agent = run_a.create("t1")
    await run_a.run(agent, "remember the codeword pineapple")
    assert (await teleport(a, b, agent)).state == "committed"
    await run_b.run(agent, "what was the codeword?")
    sent = prompts(log_b)[0]["prompt"]
    assert "continuing a conversation" in sent and "pineapple" in sent
    assert sent.rstrip().endswith("what was the codeword?")
    assert verify(b.agents / agent) == []


async def test_freezing_cancels_the_agent_and_stops_the_turn(tmp_path):
    host, run, _ = make(tmp_path)
    agent = run.create("t1")
    task = asyncio.create_task(run.run(agent, "slow one"))
    await asyncio.sleep(1.0)
    await host.freeze(agent)  # a teleport begins mid-turn
    out = await asyncio.wait_for(task, 20)
    assert out.stopped == "frozen"


async def test_a_crashing_agent_is_reported_and_the_thread_stays_usable(tmp_path):
    host, run, _ = make(tmp_path)
    agent = run.create("t1")
    out = await run.run(agent, "crash now")
    assert out.stopped == "error" and verify(host.agents / agent) == []
    again = await run.run(agent, "hello again")
    assert again.stopped == "done"
