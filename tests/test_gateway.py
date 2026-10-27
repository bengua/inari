from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from inari_core.approval_bus import NodeApprovals
from inari_core.approvals import ApprovalGate
from inari_core.demo import run_demo
from inari_core.gateway import Gateway
from inari_core.mesh import Bus

KEY = b"device"


async def http(port, method, path, body=None, token="t"):
    reader, writer = await asyncio.open_connection("127.0.0.1", port)
    data = json.dumps(body).encode() if body is not None else b""
    auth = f"Authorization: Bearer {token}\r\n" if token else ""
    if body is not None:
        auth += "Content-Type: application/json\r\n"
    writer.write(
        f"{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {len(data)}\r\n\r\n".encode()
        + data
    )
    await writer.drain()
    raw = await reader.read()
    writer.close()
    head, _, payload = raw.partition(b"\r\n\r\n")
    return int(head.split()[1]), payload


@pytest.fixture
async def gw():
    bus = Bus("hub")
    gateway = Gateway(bus, token="t", device_key=KEY)
    await gateway.start()
    yield bus, gateway
    await gateway.stop()


async def test_the_page_is_public_but_data_needs_the_token(gw):
    _, gateway = gw
    status, page = await http(gateway.port, "GET", "/", token="")
    assert status == 200 and b'id="fox"' in page
    assert (await http(gateway.port, "GET", "/events", token=""))[0] == 401
    assert (await http(gateway.port, "POST", "/api/teleport", {}, token="nope"))[0] == 401


async def test_state_streams_as_events(gw):
    bus, gateway = gw
    await bus.publish("mesh.nodes", {"nodes": [{"node_id": "n1", "role": "heavy"}]})
    await bus.publish("agent.a1.state", {"agent": "a1", "node": "n1", "phase": "inference"})
    await bus.publish("node.n1.load", {"node": "n1", "gpu": 1.0, "packets": 4000})
    reader, writer = await asyncio.open_connection("127.0.0.1", gateway.port)
    writer.write(b"GET /events?token=t HTTP/1.1\r\nHost: x\r\n\r\n")
    await writer.drain()
    seen = b""
    while b"data:" not in seen:
        seen += await asyncio.wait_for(reader.read(4096), 2)
    snap = json.loads(seen.split(b"data: ", 1)[1].split(b"\n\n")[0])
    assert snap["agents"]["a1"]["phase"] == "inference"
    assert snap["mascot"]["colour"] == "#fbbf24" and snap["mascot"]["pulse_hz"] == 2.0
    await bus.publish("agent.a1.state", {"agent": "a1", "node": "n1", "phase": "success"})
    more = await asyncio.wait_for(reader.read(4096), 2)
    assert b'"success"' in more
    writer.close()


async def test_a_teleport_request_reaches_the_bus(gw):
    bus, gateway = gw
    got = []
    bus.subscribe("teleport.request", lambda m: got.append(m.payload))
    status, body = await http(
        gateway.port,
        "POST",
        "/api/teleport",
        {"agent": "a", "source": "n1", "target": "n2"},
    )
    assert (
        status == 200
        and got[0]["target"] == "n2"
        and got[0]["request"] == json.loads(body)["request"]
    )
    assert (await http(gateway.port, "POST", "/api/teleport", {"agent": "a"}))[0] == 400


async def test_an_approval_travels_phone_to_node_with_a_valid_signature(gw):
    bus, gateway = gw
    approvals = NodeApprovals(bus, ApprovalGate(KEY))
    asking = asyncio.create_task(approvals.ask("a1", "n1", "sudo reboot", ttl=5))
    await asyncio.sleep(0.1)
    pending = list(gateway.approvals)
    assert len(pending) == 1
    status, _ = await http(
        gateway.port,
        "POST",
        "/api/approval",
        {"request_id": pending[0], "approved": True},
    )
    assert status == 200 and await asyncio.wait_for(asking, 2) is True
    again = {"request_id": pending[0], "approved": True}
    assert (await http(gateway.port, "POST", "/api/approval", again))[0] == 404  # one use


async def test_the_demo_mesh_really_moves_an_agent_when_asked(tmp_path: Path):
    started: list[Gateway] = []
    task = asyncio.create_task(run_demo(port=0, workdir=tmp_path, on_ready=started.append))
    for _ in range(100):
        if started:
            break
        await asyncio.sleep(0.1)
    gateway = started[0]
    assert [n["node_id"] for n in gateway.nodes] == ["atlas", "borealis", "cirrus"]
    request = {"agent": "scribe", "source": "atlas", "target": "cirrus"}
    status, _ = await http(gateway.port, "POST", "/api/teleport", request, token="demo")
    assert status == 200
    for _ in range(100):
        if gateway.results:
            break
        await asyncio.sleep(0.1)
    assert gateway.results[-1]["state"] == "committed"
    kinds = []
    for _ in range(100):
        status, body = await http(gateway.port, "GET", "/api/agent/scribe/timeline", token="demo")
        kinds = [e["kind"] for e in json.loads(body)["events"]]
        if "checkpoint" in kinds:
            break
        await asyncio.sleep(0.1)
    assert status == 200 and "checkpoint" in kinds and "tool_result" in kinds
    task.cancel()
    await asyncio.gather(task, return_exceptions=True)


async def test_agent_cards_get_their_facts_and_moves_are_reported_live(gw):
    bus, gateway = gw
    await bus.publish(
        "agent.a1.state",
        {
            "agent": "a1",
            "node": "n1",
            "phase": "idle",
            "title": "Draft",
            "events": 7,
            "files": 2,
            "last": "hi",
        },
    )
    await bus.publish("agent.a1.state", {"agent": "a1", "node": "n1", "phase": "inference"})
    snap = gateway.snapshot()
    assert snap["agents"]["a1"]["title"] == "Draft" and snap["agents"]["a1"]["phase"] == "inference"
    await bus.publish(
        "teleport.state",
        {"request": "r", "agent": "a1", "source": "n1", "target": "n2", "state": "transferred"},
    )
    assert gateway.snapshot()["moves"]["a1"]["state"] == "transferred"
