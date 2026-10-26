"""A whole mesh in one process, for trying the interface: three real nodes talking over
loopback TCP, a rule-based model, live agents, and approval questions. Every drag in
the browser performs a genuine teleportation."""

from __future__ import annotations

import asyncio
import contextlib
import random
import tempfile
from collections.abc import Callable
from pathlib import Path

from .agent import ProviderConfig
from .gateway import Gateway
from .node import Node, NodeConfig

NAMES = ("atlas", "borealis", "cirrus")
START = {"scribe": "atlas", "builder": "atlas", "reviewer": "borealis"}


async def run_demo(
    host: str = "127.0.0.1",
    port: int = 8787,
    token: str = "demo",
    workdir: Path | None = None,
    on_ready: Callable[[Gateway], None] | None = None,
) -> None:
    base = Path(workdir or tempfile.mkdtemp(prefix="inari-demo-"))
    base.mkdir(parents=True, exist_ok=True)
    for name, text in (
        ("mesh.key", "demo-mesh-key"),
        ("device.key", "demo-device-key"),
        ("token", token),
    ):
        (base / name).write_text(text)
    common = dict(
        key_file=base / "mesh.key", device_key_file=base / "device.key",
        provider=ProviderConfig(kind="demo"), heartbeat=1.0,
    )  # fmt: skip
    hub = Node(
        NodeConfig(
            node_id=NAMES[0], data_dir=base / NAMES[0], expected_peers=list(NAMES[1:]),
            gateway_listen=f"{host}:{port}", gateway_token_file=base / "token", **common,
        )
    )  # fmt: skip
    await hub.start()
    nodes = {NAMES[0]: hub}
    for name in NAMES[1:]:
        node = Node(
            NodeConfig(
                node_id=name, data_dir=base / name, hub=f"127.0.0.1:{hub.bus_port}", **common
            )
        )
        await node.start()
        nodes[name] = node
    gateway = hub.gateway
    assert gateway is not None
    while len(gateway.nodes) < len(NAMES):
        await asyncio.sleep(0.1)
    for agent, where in START.items():
        nodes[where].runner.create(agent, title=agent, approval_required=[])
    await asyncio.sleep(0.5)
    for agent in START:
        await hub.bus.publish(
            f"agent.{agent}.run", {"text": f"write notes.txt: hello from {agent}"}
        )
    rng = random.Random(7)

    async def chatter() -> None:
        while True:
            await asyncio.sleep(5)
            agent = rng.choice(list(START))
            await hub.bus.publish(f"agent.{agent}.run", {"text": "run: sleep 1; ls"})

    async def questions() -> None:
        await asyncio.sleep(6)
        while True:
            await hub.runner.approvals.ask(
                "builder", "atlas", "sudo systemctl restart demo", ttl=120
            )
            await asyncio.sleep(25)

    tasks = [asyncio.create_task(c()) for c in (chatter, questions)]
    print(
        f"inari-core demo: http://{host}:{gateway.port}/#token={token}  (Ctrl+C to stop)",
        flush=True,
    )
    if on_ready:
        on_ready(gateway)
    try:
        await asyncio.Event().wait()
    finally:
        for t in tasks:
            t.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.gather(*tasks)
        for node in reversed(list(nodes.values())):
            await node.stop()
