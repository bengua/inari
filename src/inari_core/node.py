"""A node of the mesh: one process that holds agents, serves capsules to its peers,
drives its agents, and talks to the rest over the bus. A node with no `hub` is the
hub: it also runs the coordinator and, if configured, the gateway."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
import shutil
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .agent import AgentRunner, ProviderConfig, make_provider
from .approval_bus import NodeApprovals
from .approvals import ApprovalGate
from .capsule import fork as fork_capsule
from .capsule import io, merge
from .capsule.search import event_text
from .gateway import Gateway
from .mesh import Bus, MeshCoordinator, NodeInfo, Registry, Role
from .sandbox import ContainerRuntime, SubprocessRuntime
from .secrets import EnvResolver
from .teleport import CapsuleHost, teleport
from .transport import BusClient, BusServer, EndpointServer, RemoteEndpoint


@dataclass
class NodeConfig:
    node_id: str
    data_dir: Path
    key_file: Path  # shared mesh key (raw bytes)
    device_key_file: Path | None = None  # approval key shared with the gateway
    role: str = "heavy"
    bind_host: str = "127.0.0.1"
    bus_port: int = 0  # hub only
    endpoint_port: int = 0
    advertise_host: str = ""  # how peers reach this node's endpoint (default: bind_host)
    hub: str = ""  # HOST:PORT of the hub bus; empty means this node is the hub
    expected_peers: list[str] = field(default_factory=list)
    provider: ProviderConfig = field(default_factory=ProviderConfig)
    sandbox: str = "subprocess"  # or "container"
    engine: str = "podman"
    gateway_listen: str = ""  # HOST:PORT, hub only
    gateway_token_file: Path | None = None
    heartbeat: float = 2.0
    registry_ttl: float = 8.0

    @classmethod
    def load(cls, path: Path) -> NodeConfig:
        raw = json.loads(Path(path).read_text())
        provider = ProviderConfig(**raw.pop("provider", {}))
        for key in ("data_dir", "key_file", "device_key_file", "gateway_token_file"):
            if raw.get(key):
                raw[key] = Path(raw[key])
        return cls(provider=provider, **raw)


def _loadavg() -> float:
    try:
        return min(1.0, os.getloadavg()[0] / max(1, os.cpu_count() or 1))
    except OSError:
        return 0.0


def _packets() -> int:
    try:
        lines = Path("/proc/net/dev").read_text().splitlines()[2:]
        return sum(int(ln.split()[2]) + int(ln.split()[10]) for ln in lines)
    except (OSError, ValueError, IndexError):
        return 0


class Node:
    def __init__(self, cfg: NodeConfig) -> None:
        self.cfg = cfg
        self.is_hub = not cfg.hub
        self.key = cfg.key_file.read_bytes().strip()
        self.device_key = cfg.device_key_file.read_bytes().strip() if cfg.device_key_file else b""
        self.bus = Bus(cfg.node_id)
        self.host = CapsuleHost(cfg.node_id, cfg.data_dir / "host", clock=time.time)
        self.registry = Registry(ttl=cfg.registry_ttl)
        self.bus_port = 0
        self.endpoint_port = 0
        self.gateway: Gateway | None = None
        self.runner: AgentRunner | None = None
        self._tasks: list[asyncio.Task] = []
        self._background: set[asyncio.Task] = set()
        self._servers: list[Any] = []
        self._last_packets = _packets()

    # -- wiring --------------------------------------------------------------
    async def start(self) -> None:
        cfg = self.cfg
        self.host.recover()
        endpoint = EndpointServer(self.host, self.key, cfg.bind_host, cfg.endpoint_port)
        self.endpoint_port = await endpoint.start()
        self._servers.append(endpoint)
        if self.is_hub:
            server = BusServer(
                self.bus, self.key, cfg.bind_host, cfg.bus_port, tuple(cfg.expected_peers)
            )
            self.bus_port = await server.start()
            self._servers.append(server)
            MeshCoordinator(self.bus, self.registry, self._teleporter).start()
        else:
            host, _, port = cfg.hub.rpartition(":")
            client = BusClient(self.bus, self.key, host, int(port), retry=0.1)
            client.start()
            self._servers.append(client)
        resolver = EnvResolver()
        approvals = (
            NodeApprovals(self.bus, ApprovalGate(self.device_key)) if self.device_key else None
        )
        runtime = (
            ContainerRuntime(resolver, engine=cfg.engine)
            if cfg.sandbox == "container"
            else SubprocessRuntime(resolver)
        )
        if cfg.provider.kind == "demo" or cfg.provider.base_url:
            self.runner = AgentRunner(
                self.host, make_provider(cfg.provider, resolver), runtime,
                approvals=approvals, publish=self.bus.publish,
                context_tokens=cfg.provider.context_tokens,
            )  # fmt: skip
        self._subscribe()
        if self.is_hub and cfg.gateway_listen:
            token = cfg.gateway_token_file.read_text().strip() if cfg.gateway_token_file else ""
            self.gateway = Gateway(self.bus, token=token, device_key=self.device_key)
            lhost, _, lport = cfg.gateway_listen.rpartition(":")
            await self.gateway.start(lhost, int(lport))
        self._tasks.append(asyncio.create_task(self._beat()))

    async def stop(self) -> None:
        for task in [*self._tasks, *self._background]:
            task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await asyncio.gather(*self._tasks)
        if self.gateway:
            await self.gateway.stop()
        for server in self._servers:
            await server.stop()

    def _endpoint(self, node_id: str):
        if node_id == self.cfg.node_id:
            return self.host
        info = self.registry.get(node_id)
        address = (info.capabilities.get("endpoint") if info else "") or ""
        host, _, port = address.rpartition(":")
        return RemoteEndpoint(node_id, host, int(port), self.key)

    async def _teleporter(self, agent: str, source: str, target: str):
        return await teleport(self._endpoint(source), self._endpoint(target), agent)

    # -- liveness and telemetry ---------------------------------------------------
    async def _beat(self) -> None:
        cfg, first = self.cfg, True
        advertise = f"{cfg.advertise_host or cfg.bind_host}:{self.endpoint_port}"
        info = NodeInfo(
            cfg.node_id, Role(cfg.role), int(time.time()), capabilities={"endpoint": advertise}
        )
        while True:
            await self.bus.publish("node.announce" if first else "node.heartbeat", info.to_dict())
            first = False
            now = _packets()
            rate = max(0, now - self._last_packets) / cfg.heartbeat
            self._last_packets = now
            await self.bus.publish(
                f"node.{cfg.node_id}.load",
                {"node": cfg.node_id, "gpu": _loadavg(), "packets": rate},
            )
            for agent in self._held():
                phase = self.runner.phases.get(agent, "idle") if self.runner else "idle"
                await self.bus.publish(
                    f"agent.{agent}.state", {"agent": agent, "node": cfg.node_id, "phase": phase}
                )
            if self.is_hub:
                coordinator_lost = self.registry.expired()
                for node_id in coordinator_lost:
                    await self.bus.publish("node.lost", {"node": node_id})
            await asyncio.sleep(cfg.heartbeat)

    def _held(self) -> list[str]:
        return sorted(
            p.name for p in self.host.agents.iterdir() if self.host.status(p.name) == "running"
        )

    # -- requests from the bus ------------------------------------------------------
    def _subscribe(self) -> None:
        self.bus.subscribe("agent.create", self._on_create)
        self.bus.subscribe("agent.*.run", self._on_run)
        self.bus.subscribe("agent.*.fork", self._on_fork)
        self.bus.subscribe("agent.merge", self._on_merge)
        self.bus.subscribe("agent.history.request", self._on_history)

    async def _announce_created(self, agent: str, ok: bool = True, error: str = "") -> None:
        await self.bus.publish(
            "agent.created", {"agent": agent, "node": self.cfg.node_id, "ok": ok, "error": error}
        )

    async def _on_create(self, m) -> None:
        p = m.payload
        if p.get("node") != self.cfg.node_id or self.runner is None:
            return
        try:
            agent = self.runner.create(
                str(p["thread_id"]), title=str(p.get("title", "")),
                approval_required=list(p.get("approval_required", [])),
            )  # fmt: skip
            await self._announce_created(agent)
        except Exception as exc:  # noqa: BLE001 - reported to the requester
            await self._announce_created(
                str(p.get("thread_id")), False, f"{type(exc).__name__}: {exc}"
            )

    def _spawn(self, coro) -> None:
        """Run long work off the bus: a handler that blocks stalls the link reader, and
        an agent waiting for an approval would wait for the very frame it is blocking."""
        task = asyncio.create_task(coro)
        self._background.add(task)
        task.add_done_callback(self._background.discard)

    async def _on_run(self, m) -> None:
        agent = m.subject.split(".")[1]
        if self.runner is None or self.host.status(agent) != "running":
            return
        self._spawn(self._run_turn(agent, str(m.payload["text"])))

    async def _run_turn(self, agent: str, text: str) -> None:
        summary = await self.runner.run(agent, text)
        await self.bus.publish(
            f"agent.{agent}.done",
            {
                "agent": agent,
                "stopped": summary.stopped,
                "error": summary.error,
                "text": summary.final_text[:400],
            },
        )

    async def _on_fork(self, m) -> None:
        agent = m.subject.split(".")[1]
        if self.host.status(agent) != "running":
            return
        new = str(m.payload["new_thread"])
        staging = self.host.root / "new" / new
        try:
            shutil.rmtree(staging, ignore_errors=True)
            fork_capsule(self.host.agents / agent, staging, new, m.payload.get("upto"))
            io.set_holder(staging, None)
            created = self.host.adopt(staging)
            shutil.rmtree(staging, ignore_errors=True)
            await self._announce_created(created)
        except Exception as exc:  # noqa: BLE001
            await self._announce_created(new, False, f"{type(exc).__name__}: {exc}")

    async def _on_merge(self, m) -> None:
        target, source = str(m.payload["target"]), str(m.payload["source"])
        if self.host.status(target) != "running" or self.host.status(source) != "running":
            return
        try:
            added = merge(self.host.agents / target, self.host.agents / source)
            await self.bus.publish(
                "agent.merged", {"target": target, "source": source, "added": added, "ok": True}
            )
        except Exception as exc:  # noqa: BLE001
            await self.bus.publish(
                "agent.merged", {"target": target, "source": source, "ok": False, "error": str(exc)}
            )

    async def _on_history(self, m) -> None:
        agent = str(m.payload["agent"])
        if self.host.status(agent) != "running":
            return
        events = [
            {"id": e.id, "kind": e.kind.value, "text": event_text(e.data)[:400]}
            for e in io.load(self.host.agents / agent).timeline.ordered()
        ]
        await self.bus.publish(f"agent.{agent}.history", {"agent": agent, "events": events})
