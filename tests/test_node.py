from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from inari_core.agent import ProviderConfig
from inari_core.capsule import load, verify
from inari_core.node import Node, NodeConfig

from .test_gateway import http


async def until(predicate, timeout=8.0):
    loop = asyncio.get_running_loop()
    end = loop.time() + timeout
    while loop.time() < end:
        if predicate():
            return True
        await asyncio.sleep(0.05)
    return predicate()


@pytest.fixture
async def mesh(tmp_path: Path):
    (tmp_path / "mesh.key").write_text("mesh-key")
    (tmp_path / "device.key").write_text("device-key")
    (tmp_path / "token").write_text("tok")
    base = dict(
        key_file=tmp_path / "mesh.key",
        device_key_file=tmp_path / "device.key",
        provider=ProviderConfig(kind="demo"),
        heartbeat=0.2,
    )
    hub = Node(
        NodeConfig(
            node_id="hub", data_dir=tmp_path / "hub", expected_peers=["worker"],
            gateway_listen="127.0.0.1:0", gateway_token_file=tmp_path / "token", **base,
        )
    )  # fmt: skip
    await hub.start()
    worker = Node(
        NodeConfig(
            node_id="worker", data_dir=tmp_path / "worker", hub=f"127.0.0.1:{hub.bus_port}", **base
        )
    )
    await worker.start()
    assert await until(lambda: len(hub.gateway.nodes) == 2)
    yield hub, worker
    await worker.stop()
    await hub.stop()


async def test_create_chat_move_and_keep_chatting_across_real_processes_sockets(mesh):
    hub, worker = mesh
    api = hub.gateway.port

    async def post(path, body):
        status, _ = await http(api, "POST", path, body, token="tok")
        assert status == 200

    await post("/api/agent", {"thread_id": "t1", "node": "worker", "title": "demo"})
    assert await until(lambda: worker.host.status("t1") == "running")
    await post("/api/message", {"agent": "t1", "text": "run: echo from-worker"})
    assert await until(lambda: any(n.get("stopped") == "done" for n in hub.gateway.notices))
    log = hub.gateway.log["t1"]
    assert any("from-worker" in e["text"] for e in log if e["kind"] == "tool_result")

    # drag it to the hub: a real teleport through the coordinator and the TCP endpoints
    status, _ = await http(
        api,
        "POST",
        "/api/teleport",
        {"agent": "t1", "source": "worker", "target": "hub"},
        token="tok",
    )
    assert status == 200
    assert await until(
        lambda: hub.host.status("t1") == "running" and worker.host.status("t1") == "tombstoned"
    )
    assert verify(hub.host.agents / "t1") == []
    hub.gateway.notices.clear()
    await post("/api/message", {"agent": "t1", "text": "write notes.txt: carried over"})
    assert await until(lambda: any(n.get("stopped") == "done" for n in hub.gateway.notices))
    assert "notes.txt" in load(hub.host.agents / "t1").snapshot.tree
    assert [n["node_id"] for n in hub.gateway.nodes] == ["hub", "worker"] or len(
        hub.gateway.nodes
    ) == 2


async def test_fork_merge_and_search_through_the_gateway(mesh):
    hub, worker = mesh
    api = hub.gateway.port
    await http(api, "POST", "/api/agent", {"thread_id": "base", "node": "worker"}, token="tok")
    assert await until(lambda: worker.host.status("base") == "running")
    await http(
        api,
        "POST",
        "/api/message",
        {"agent": "base", "text": "remember the codeword pineapple"},
        token="tok",
    )
    assert await until(
        lambda: any(e["kind"] == "assistant_message" for e in hub.gateway.log.get("base", []))
    )
    await http(api, "POST", "/api/fork", {"agent": "base", "new_thread": "branch"}, token="tok")
    assert await until(lambda: worker.host.status("branch") == "running")
    await http(
        api, "POST", "/api/message", {"agent": "branch", "text": "branch-only idea"}, token="tok"
    )
    assert await until(
        lambda: any("branch-only" in e["text"] for e in hub.gateway.log.get("branch", []))
    )
    await http(api, "POST", "/api/merge", {"target": "base", "source": "branch"}, token="tok")
    assert await until(
        lambda: any(n.get("subject") == "agent.merged" and n["ok"] for n in hub.gateway.notices)
    )
    texts = [e.data.get("text", "") for e in load(worker.host.agents / "base").timeline.ordered()]
    assert any("branch-only" in t for t in texts)
    status, body = await http(api, "GET", "/api/search?q=pineapple", token="tok")
    assert status == 200 and any("pineapple" in h["text"] for h in json.loads(body)["hits"])


async def test_a_guarded_command_reaches_the_phone_through_the_hub(mesh):
    hub, worker = mesh
    api = hub.gateway.port
    await http(api, "POST", "/api/agent", {"thread_id": "g", "node": "worker"}, token="tok")
    assert await until(lambda: worker.host.status("g") == "running")
    # make bash guarded for this agent, then ask for a command
    from inari_core.capsule import io, seal

    cap = io.load(worker.host.agents / "g")
    manifest = cap.manifest.model_copy(
        update={"access": cap.manifest.access.model_copy(update={"approval_required": ["bash *"]})}
    )
    seal(worker.host.agents / "g", manifest, cap.timeline)
    await http(api, "POST", "/api/message", {"agent": "g", "text": "run: echo hi"}, token="tok")
    assert await until(lambda: bool(hub.gateway.approvals))
    request_id = next(iter(hub.gateway.approvals))
    status, _ = await http(
        api, "POST", "/api/approval", {"request_id": request_id, "approved": True}, token="tok"
    )
    assert status == 200
    assert await until(
        lambda: any(e["kind"] == "tool_result" and "hi" in e["text"] for e in hub.gateway.log["g"])
    )


async def test_a_command_typed_by_the_person_runs_in_the_workspace_and_the_agent_knows(mesh):
    hub, worker = mesh
    api = hub.gateway.port
    await http(api, "POST", "/api/agent", {"thread_id": "x1", "node": "worker"}, token="tok")
    assert await until(lambda: worker.host.status("x1") == "running")
    status, body = await http(
        api,
        "POST",
        "/api/exec",
        {"agent": "x1", "cmd": "echo made-by-person > p.txt; cat p.txt"},
        token="tok",
    )
    reply = json.loads(body)
    assert status == 200 and reply["ok"] and "made-by-person" in reply["output"]
    capsule = load(worker.host.agents / "x1")
    assert "p.txt" in capsule.snapshot.tree and verify(capsule.root) == []
    call = [e for e in capsule.timeline.ordered() if e.kind.value == "tool_call"][0]
    assert call.data["arguments"]["by"] == "user"  # journaled, so the next turn sees it
    status, body = await http(api, "GET", "/api/agent/x1/file?path=p.txt", token="tok")
    assert "made-by-person" in json.loads(body)["text"]
