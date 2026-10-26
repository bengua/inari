from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from inari_core.capsule import verify
from inari_core.mesh import (
    Bus,
    MeshCoordinator,
    NodeInfo,
    Registry,
    Role,
    Telemetry,
    connect,
    mascot_state,
    subject_matches,
)
from inari_core.teleport import CapsuleHost, teleport
from tests.conftest import make_capsule


@pytest.mark.parametrize(
    "pattern,subject,expected",
    [
        ("a.b", "a.b", True),
        ("a.*", "a.b", True),
        ("a.*", "a.b.c", False),
        ("a.>", "a.b.c", True),
        ("a.>", "a", False),
        ("*.state", "agent.state", True),
        ("agent.*.state", "agent.x.state", True),
        ("agent.*.state", "agent.x.load", False),
    ],
)
def test_subject_matching(pattern, subject, expected):
    assert subject_matches(pattern, subject) is expected


async def test_a_failing_handler_does_not_stop_the_others():
    bus, seen = Bus("a"), []

    def boom(_):
        raise RuntimeError("bad handler")

    bus.subscribe("x", boom)
    bus.subscribe("x", lambda m: seen.append(m.payload))
    assert await bus.publish("x", {"n": 1}) == 1
    assert seen == [{"n": 1}] and bus.handler_errors == 1


async def test_messages_cross_a_link_and_never_loop():
    a, b, c = Bus("a"), Bus("b"), Bus("c")
    connect(a, b)
    connect(b, c)
    connect(c, a)  # a ring: without loop protection this never ends
    got = []
    c.subscribe("hello", lambda m: got.append(m.sender))
    await a.publish("hello", {})
    assert got == ["a"]


async def test_durable_messages_survive_a_partition_and_arrive_once():
    phone, home = Bus("phone"), Bus("home")
    up, down = connect(phone, home)
    got = []
    home.subscribe("teleport.request", lambda m: got.append(m.payload["n"]))
    home.subscribe("noise.>", lambda m: got.append("noise"))
    up.disconnect()
    await phone.publish("teleport.request", {"n": 1})
    await phone.publish("noise.x", {})  # not durable: lost while offline
    assert got == []
    assert await up.reconnect() == 1
    assert got == [1]
    await up.reconnect()  # nothing is replayed twice
    assert got == [1]
    assert down.connected


def test_registry_expires_silent_nodes_and_ignores_stale_incarnations():
    now = [0.0]
    reg = Registry(ttl=10, clock=lambda: now[0])
    reg.observe(NodeInfo("n1", Role.HEAVY, incarnation=2))
    now[0] = 5
    assert not reg.observe(NodeInfo("n1", Role.HEAVY, incarnation=1))  # stale
    assert [n.node_id for n in reg.alive()] == ["n1"]
    now[0] = 30
    assert reg.alive() == [] and reg.expired() == ["n1"]


def test_registries_converge_by_merging_snapshots():
    a, b = Registry(), Registry()
    a.observe(NodeInfo("x", Role.HEAVY))
    b.observe(NodeInfo("y", Role.MOBILE))
    a.merge(b.snapshot())
    b.merge(a.snapshot())
    assert [n.node_id for n in a.alive()] == [n.node_id for n in b.alive()] == ["x", "y"]


def test_mascot_is_a_pure_function_of_telemetry():
    t = Telemetry()
    idle = mascot_state(t.snapshot())
    assert idle.colour == "#22d3ee" and idle.pulse_hz == 0.25 and idle.turbulence == 0
    t.update_agent("a1", "n1", "inference")
    t.update_node("n1", gpu=1.0, packets=4000)
    busy = mascot_state(t.snapshot())
    assert busy.colour == "#fbbf24" and busy.pulse_hz == 2.0 and busy.turbulence == 1
    t.update_agent("a2", "n1", "error")
    assert mascot_state(t.snapshot()).phase.value == "error"
    assert mascot_state(t.snapshot()) == mascot_state(t.snapshot())


async def test_phone_drags_an_agent_between_machines_end_to_end(tmp_path: Path):
    phone, home = Bus("phone"), Bus("home")
    connect(phone, home)
    src = CapsuleHost("node-a", tmp_path / "a")
    dst = CapsuleHost("node-b", tmp_path / "b")
    agent = src.adopt(make_capsule(tmp_path / "seed").root)
    hosts = {"node-a": src, "node-b": dst}

    async def teleporter(agent_id, source, target, progress):
        return await teleport(hosts[source], hosts[target], agent_id, on_state=progress)

    registry = Registry()
    coordinator = MeshCoordinator(home, registry, teleporter)
    coordinator.start()
    for node_id, role in (
        ("node-a", Role.HEAVY),
        ("node-b", Role.HEAVY),
        ("p1", Role.MOBILE),
    ):
        await home.publish("node.announce", NodeInfo(node_id, role).to_dict())

    results: asyncio.Queue = asyncio.Queue()
    phone.subscribe("teleport.result", results.put_nowait)
    await phone.publish(
        "teleport.request",
        {"request": "r1", "agent": agent, "source": "node-a", "target": "node-b"},
    )
    result = (await asyncio.wait_for(results.get(), 2)).payload
    assert result["ok"] and result["state"] == "committed"
    assert verify(dst.agents / agent) == []
    assert src.status(agent) == "tombstoned" and dst.status(agent) == "running"


async def test_the_coordinator_refuses_a_phone_as_target_and_dead_nodes(tmp_path: Path):
    bus = Bus("home")

    async def never(*_):
        raise AssertionError("must not run")

    coordinator = MeshCoordinator(bus, Registry(), never)
    coordinator.start()
    await bus.publish("node.announce", NodeInfo("big", Role.HEAVY).to_dict())
    await bus.publish("node.announce", NodeInfo("phone", Role.MOBILE).to_dict())
    replies = []
    bus.subscribe("teleport.result", lambda m: replies.append(m.payload))
    for target in ("phone", "ghost"):
        await bus.publish(
            "teleport.request",
            {"request": target, "agent": "a", "source": "big", "target": target},
        )
    for _ in range(50):
        if len(replies) == 2:
            break
        await asyncio.sleep(0.02)  # the handler runs detached
    assert [r["ok"] for r in replies] == [False, False]
    assert "heavy" in replies[0]["error"] and "alive" in replies[1]["error"]


async def test_sweep_announces_lost_nodes():
    now = [0.0]
    bus = Bus("home")
    coordinator = MeshCoordinator(bus, Registry(ttl=5, clock=lambda: now[0]), None)
    coordinator.start()
    lost = []
    bus.subscribe("node.lost", lambda m: lost.append(m.payload["node"]))
    await bus.publish("node.heartbeat", NodeInfo("n1", Role.HEAVY).to_dict())
    now[0] = 99
    assert await coordinator.sweep() == ["n1"] and lost == ["n1"]
