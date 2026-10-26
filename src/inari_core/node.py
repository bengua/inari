"""A node of the mesh: one process that holds agents, serves capsules to its peers,
drives its agents, and talks to the rest over the bus. A node with no `hub` is the
hub: it also runs the coordinator and, if configured, the gateway."""

from __future__ import annotations

import asyncio
import base64
import contextlib
import json
import os
import shutil
import time
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .agent import AcpHarness, AgentRunner, ProviderConfig, make_provider
from .approval_bus import NodeApprovals
from .approvals import ApprovalGate
from .capsule import fork as fork_capsule
from .capsule import io, merge
from .capsule.search import event_summary, event_text
from .gateway import Gateway
from .mesh import Bus, MeshCoordinator, NodeInfo, Registry, Role
from .sandbox import ContainerRuntime, SubprocessRuntime
from .secrets import EnvResolver
from .teleport import CapsuleHost, teleport
from .transport import BusClient, BusServer, EndpointServer, RemoteEndpoint
from .usage import CloudroomUsageSource, StaticUsageSource


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
    harness: dict = field(default_factory=dict)  # {"kind": "acp", "command": [...], "timeout": 600}
    # Several named harnesses, the first being the default: {"claude": {"command": [...]}, ...}
    harnesses: dict = field(default_factory=dict)
    sandbox: str = "subprocess"  # or "container"
    engine: str = "podman"
    gateway_listen: str = ""  # HOST:PORT, hub only
    gateway_token_file: Path | None = None
    gateway_ui: dict = field(default_factory=dict)  # {"footer": "..."}
    mascot_dir: Path | None = None  # a sprite engine folder served at /fox/ (optional)
    usage: list[dict] = field(default_factory=list)  # quota sources, see docs/node.md
    heartbeat: float = 2.0
    registry_ttl: float = 8.0

    @classmethod
    def load(cls, path: Path) -> NodeConfig:
        raw = json.loads(Path(path).read_text())
        provider = ProviderConfig(**raw.pop("provider", {}))
        for key in ("data_dir", "key_file", "device_key_file", "gateway_token_file", "mascot_dir"):
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
        common = dict(approvals=approvals, publish=self.bus.publish)
        specs = dict(cfg.harnesses)
        if cfg.harness.get("kind") == "acp":
            specs = {"default": cfg.harness, **specs}
        if specs:
            built = {
                name: AcpHarness(
                    list(spec["command"]),
                    cfg.data_dir / "host" / f"acp_sessions.{name}.json",
                    timeout=float(spec.get("timeout", 600)),
                    extra_env=dict(spec.get("env", {})),
                )
                for name, spec in specs.items()
            }
            self.runner = AgentRunner(self.host, None, runtime, harnesses=built, **common)
            self.host.on_prepare = self._can_host
        elif cfg.provider.kind == "demo" or cfg.provider.base_url:
            self.runner = AgentRunner(
                self.host, make_provider(cfg.provider, resolver), runtime,
                context_tokens=cfg.provider.context_tokens, **common,
            )  # fmt: skip
        self._subscribe()
        if self.is_hub and cfg.gateway_listen:
            token = cfg.gateway_token_file.read_text().strip() if cfg.gateway_token_file else ""
            sources = []
            for spec in cfg.usage:
                if spec.get("kind") == "cloudroom":
                    sources.append(
                        CloudroomUsageSource(spec["url"], list(spec.get("providers", [])))
                    )
                elif spec.get("kind") == "static":
                    sources.append(StaticUsageSource(list(spec.get("items", []))))
            self.gateway = Gateway(
                self.bus,
                token=token,
                device_key=self.device_key,
                ui=cfg.gateway_ui,
                mascot_dir=cfg.mascot_dir,
                usage_sources=sources,
            )
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

    async def _teleporter(self, agent: str, source: str, target: str, progress):
        return await teleport(
            self._endpoint(source), self._endpoint(target), agent, on_state=progress
        )

    # -- liveness and telemetry ---------------------------------------------------
    async def _beat(self) -> None:
        cfg, first = self.cfg, True
        advertise = f"{cfg.advertise_host or cfg.bind_host}:{self.endpoint_port}"
        info = NodeInfo(
            cfg.node_id, Role(cfg.role), int(time.time()), capabilities={"endpoint": advertise}
        )
        models_at = 0.0
        while True:
            if time.time() - models_at > 300:  # the catalogue changes rarely: look every 5 minutes
                info.capabilities["models"] = await self._models()
                info.capabilities["harnesses"] = await self._harness_models()
                models_at = time.time()
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
                    f"agent.{agent}.state",
                    {"agent": agent, "node": cfg.node_id, "phase": phase, **self._meta(agent)},
                )
            if self.is_hub:
                coordinator_lost = self.registry.expired()
                for node_id in coordinator_lost:
                    await self.bus.publish("node.lost", {"node": node_id})
            await asyncio.sleep(cfg.heartbeat)

    def _can_host(self, capsule) -> None:
        """Refuse an agent whose harness this node lacks, before the point of no return."""
        wanted = capsule.manifest.labels.get("harness", "")
        if wanted and self.runner is not None and wanted not in self.runner.harnesses:
            raise ValueError(f"this core has no {wanted!r} harness")

    async def _harness_models(self) -> dict[str, list[str]]:
        found: dict[str, list[str]] = {}
        scratch = self.cfg.data_dir / "scratch"
        scratch.mkdir(parents=True, exist_ok=True)
        for name, harness in self.runner.harnesses.items() if self.runner else []:
            if isinstance(harness, AcpHarness):
                found[name] = await harness.list_models(scratch, timeout=60)
        return found

    async def _models(self) -> list[str]:
        cfg = self.cfg
        try:
            if cfg.harness.get("kind") == "acp" and isinstance(self.runner.harness, AcpHarness):
                scratch = cfg.data_dir / "scratch"
                scratch.mkdir(parents=True, exist_ok=True)
                return await self.runner.harness.list_models(scratch)
            if cfg.provider.base_url:
                url = cfg.provider.base_url.rstrip("/") + "/models"
                headers = {}
                key = (
                    EnvResolver().get(cfg.provider.api_key_ref)
                    if cfg.provider.api_key_ref
                    else None
                )
                if key:
                    headers["Authorization"] = f"Bearer {key}"
                req = urllib.request.Request(url, headers=headers)
                raw = await asyncio.to_thread(lambda: urllib.request.urlopen(req, timeout=8).read())
                return [m["id"] for m in json.loads(raw).get("data", [])]
        except (OSError, ValueError, KeyError, AttributeError):
            pass
        return [cfg.provider.model] if cfg.provider.model else []

    def _meta(self, agent: str) -> dict:
        """What a card shows about an agent: cheap facts from its capsule."""
        try:
            capsule = io.load(self.host.agents / agent)
        except (OSError, ValueError):
            return {}
        events = capsule.timeline.ordered()
        last = next((e for e in reversed(events) if e.kind.value == "assistant_message"), None)
        return {
            "title": capsule.manifest.title or agent,
            "events": len(events),
            "files": len(capsule.snapshot.tree),
            "last": event_text(last.data)[:160] if last else "",
            "model": capsule.manifest.labels.get("model", ""),
            "harness": capsule.manifest.labels.get("harness", "")
            or (next(iter(self.runner.harnesses)) if self.runner else ""),
        }

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
        self.bus.subscribe("agent.files.request", self._on_files)
        self.bus.subscribe("agent.file.request", self._on_file)
        self.bus.subscribe("agent.exec.request", self._on_exec)
        self.bus.subscribe("agent.upload.request", self._on_upload)
        self.bus.subscribe("agent.*.model", self._on_model)
        self.bus.subscribe("agent.*.harness", self._on_harness)

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
                str(p["thread_id"]),
                title=str(p.get("title", "")),
                harness=str(p.get("harness", "")),
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
        self._spawn(self._run_turn(agent, str(m.payload["text"]), str(m.payload.get("model", ""))))

    async def _run_turn(self, agent: str, text: str, model: str = "") -> None:
        summary = await self.runner.run(agent, text, model)
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
            {"id": e.id, "kind": e.kind.value, "text": event_summary(e.kind.value, e.data)[:400]}
            for e in io.load(self.host.agents / agent).timeline.ordered()
        ]
        await self.bus.publish(f"agent.{agent}.history", {"agent": agent, "events": events})

    async def _on_files(self, m) -> None:
        agent = str(m.payload["agent"])
        if self.host.status(agent) != "running":
            return
        capsule = io.load(self.host.agents / agent)
        store = capsule.blobs()
        files = [
            {"path": path, "size": store.path(sha).stat().st_size}
            for path, sha in sorted(capsule.snapshot.tree.items())
            if store.has(sha)
        ]
        await self.bus.publish(m.payload["reply"], {"agent": agent, "files": files})

    async def _on_file(self, m) -> None:
        agent, rel = str(m.payload["agent"]), str(m.payload["path"])
        if self.host.status(agent) != "running":
            return
        capsule = io.load(self.host.agents / agent)
        sha = capsule.snapshot.tree.get(rel)
        reply: dict = {
            "agent": agent,
            "path": rel,
            "size": 0,
            "b64": "",
            "text": "",
            "binary": False,
        }
        if sha and capsule.blobs().has(sha):
            raw = capsule.blobs().get(sha)
            reply["size"] = len(raw)
            try:
                reply["text"] = raw[:200_000].decode()
            except UnicodeDecodeError:
                reply["binary"] = True
            if reply["binary"] and len(raw) <= 3_000_000:  # images and the like: sent whole
                reply["b64"] = base64.b64encode(raw).decode()
        await self.bus.publish(m.payload["reply"], reply)

    async def _on_exec(self, m) -> None:
        agent = str(m.payload["agent"])
        if self.runner is None or self.host.status(agent) != "running":
            return
        self._spawn(self._exec(agent, str(m.payload["cmd"]), m.payload["reply"]))

    async def _exec(self, agent: str, cmd: str, reply: str) -> None:
        try:
            result = await self.runner.exec_user(agent, cmd)
        except Exception as exc:  # noqa: BLE001 - the person gets the reason, not silence
            result = {"ok": False, "output": f"{type(exc).__name__}: {exc}", "code": -1}
        await self.bus.publish(reply, {"agent": agent, **result})

    async def _on_upload(self, m) -> None:
        agent = str(m.payload["agent"])
        if self.runner is None or self.host.status(agent) != "running":
            return
        try:
            result = await self.runner.upload(
                agent, str(m.payload["path"]), base64.b64decode(m.payload["b64"])
            )
        except Exception as exc:  # noqa: BLE001
            result = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
        await self.bus.publish(m.payload["reply"], {"agent": agent, **result})

    async def _on_model(self, m) -> None:
        agent = m.subject.split(".")[1]
        if self.runner is None or self.host.status(agent) != "running":
            return
        event = self.runner.set_model(agent, str(m.payload["model"]))
        if event is not None:
            await self.runner._emit(agent, "provider_switch", event.id, event.data)

    async def _on_harness(self, m) -> None:
        agent = m.subject.split(".")[1]
        if self.runner is None or self.host.status(agent) != "running":
            return
        try:
            event = self.runner.set_harness(agent, str(m.payload["harness"]))
        except ValueError:
            return
        if event is not None:
            await self.runner._emit(agent, "provider_switch", event.id, event.data)
