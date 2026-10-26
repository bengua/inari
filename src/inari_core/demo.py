"""A self-contained mesh for trying the interfaces: three nodes, three agents, live
telemetry, approval questions, and a real teleportation behind every drag."""

from __future__ import annotations

import asyncio
import contextlib
import random
import tempfile
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from .approval_bus import NodeApprovals
from .approvals import ApprovalGate
from .capsule import EnvironmentSpec, EventKind, Holder, Timeline, create, load
from .gateway import Gateway
from .mesh import Bus, MeshCoordinator, NodeInfo, Registry, Role
from .teleport import CapsuleHost, teleport

NODES = ("atlas", "borealis", "cirrus")
AGENTS = {"scribe": "atlas", "builder": "atlas", "reviewer": "borealis"}
PHASES = ["idle", "listening", "inference", "inference", "success", "idle"]
DEVICE_KEY = b"demo-device-key"


def _summary(data: dict[str, Any]) -> str:
    return str(data.get("text") or data.get("name") or data.get("output") or "")


def _seed_capsule(root: Path, thread: str) -> Path:
    t = Timeline()
    steps = [
        (EventKind.USER_MESSAGE, {"text": f"{thread}: start the task"}),
        (EventKind.DECISION, {"text": "split the work in three steps"}),
        (
            EventKind.TOOL_CALL,
            {"call_id": "c1", "name": "bash", "arguments": {"cmd": "ls"}},
        ),
        (EventKind.TOOL_RESULT, {"call_id": "c1", "output": "ok", "is_error": False}),
        (EventKind.DECISION, {"text": "write the first draft"}),
    ]
    for i, (kind, data) in enumerate(steps):
        t.append("seed", kind, data, float(i))
    create(
        root, thread_id=thread, environment=EnvironmentSpec(image="demo:1"), timeline=t,
        working_memory=f"# {thread}\n", holder=Holder(node_id="seed", epoch=1),
    )  # fmt: skip
    return root


async def run_demo(
    host: str = "127.0.0.1", port: int = 8787, token: str = "demo",
    workdir: Path | None = None, on_ready: Callable[[Gateway], None] | None = None,
) -> None:  # fmt: skip
    base = Path(workdir or tempfile.mkdtemp(prefix="inari-demo-"))
    bus = Bus("hub")
    hosts = {n: CapsuleHost(n, base / n, clock=time.time) for n in NODES}
    for agent, node in AGENTS.items():
        hosts[node].adopt(_seed_capsule(base / "seeds" / agent, agent))

    async def teleporter(agent: str, source: str, target: str):
        return await teleport(hosts[source], hosts[target], agent)

    registry = Registry(ttl=8)
    coordinator = MeshCoordinator(bus, registry, teleporter)
    coordinator.start()

    def timeline(agent: str) -> list[dict[str, Any]]:
        for h in hosts.values():
            if h.status(agent) == "running":
                events = load(h.agents / agent).timeline.ordered()
                return [
                    {"id": e.id, "kind": e.kind.value, "text": _summary(e.data)} for e in events
                ]
        return []

    gateway = Gateway(bus, token=token, device_key=DEVICE_KEY, timeline_reader=timeline)
    approvals = NodeApprovals(bus, ApprovalGate(DEVICE_KEY))
    await gateway.start(host, port)
    rng = random.Random(7)

    async def heartbeat() -> None:
        while True:
            for n in NODES:
                await bus.publish("node.heartbeat", NodeInfo(n, Role.HEAVY).to_dict())
            await coordinator.sweep()
            await asyncio.sleep(2)

    async def telemetry() -> None:
        while True:
            for n in NODES:
                mine = [a for a in AGENTS if hosts[n].status(a) == "running"]
                gpu = min(1.0, 0.1 + 0.3 * len(mine) + rng.random() * 0.4) if mine else 0.05
                await bus.publish(
                    f"node.{n}.load",
                    {"node": n, "gpu": gpu, "packets": rng.random() * 1500},
                )
                for a in mine:
                    await bus.publish(
                        f"agent.{a}.state",
                        {"agent": a, "node": n, "phase": rng.choice(PHASES)},
                    )
            await asyncio.sleep(1.2)

    async def questions() -> None:
        await asyncio.sleep(4)
        while True:
            await approvals.ask("builder", "atlas", "sudo systemctl restart demo", ttl=120)
            await asyncio.sleep(20)

    tasks = [asyncio.create_task(c()) for c in (heartbeat, telemetry, questions)]
    print(f"inari-core demo: http://{host}:{gateway.port}/#token={token}  (Ctrl+C to stop)")
    if on_ready:
        on_ready(gateway)
    try:
        await asyncio.Event().wait()
    finally:
        for t in tasks:
            t.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.gather(*tasks)
        await gateway.stop()
