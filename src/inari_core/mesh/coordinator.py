"""The mesh coordinator: discovery, liveness, telemetry and teleport requests,
all as messages on the bus. It holds no state that a restart cannot rebuild from
the next round of heartbeats (crash-only)."""

from __future__ import annotations

from collections.abc import Awaitable, Callable
from typing import Any

from ..teleport.protocol import Outcome
from .bus import Bus, Message
from .registry import NodeInfo, Registry, Role
from .telemetry import Telemetry

Teleporter = Callable[[str, str, str], Awaitable[Outcome]]  # agent, source, target


class MeshCoordinator:
    def __init__(
        self,
        bus: Bus,
        registry: Registry,
        teleporter: Teleporter,
        telemetry: Telemetry | None = None,
    ) -> None:
        self.bus, self.registry, self.teleporter = bus, registry, teleporter
        self.telemetry = telemetry or Telemetry()
        self._busy: set[str] = set()

    def start(self) -> None:
        self.bus.subscribe("node.announce", self._on_node)
        self.bus.subscribe("node.heartbeat", self._on_node)
        self.bus.subscribe("agent.*.state", self._on_agent_state)
        self.bus.subscribe("node.*.load", self._on_load)
        self.bus.subscribe("teleport.request", self._on_teleport_request)

    async def _on_node(self, message: Message) -> None:
        if self.registry.observe(NodeInfo.from_dict(message.payload)):
            await self.bus.publish("mesh.nodes", {"nodes": self.registry.snapshot()})

    def _on_agent_state(self, message: Message) -> None:
        p = message.payload
        self.telemetry.update_agent(p["agent"], p["node"], p["phase"])

    def _on_load(self, message: Message) -> None:
        p = message.payload
        self.telemetry.update_node(p["node"], gpu=p.get("gpu", 0.0), packets=p.get("packets", 0.0))

    async def sweep(self) -> list[str]:
        """Forget nodes that stopped heartbeating; tell everyone."""
        lost = self.registry.expired()
        for node_id in lost:
            self.telemetry.drop_node(node_id)
            await self.bus.publish("node.lost", {"node": node_id})
        return lost

    def _refusal(self, p: dict[str, Any]) -> str:
        source, target = self.registry.get(p["source"]), self.registry.get(p["target"])
        if source is None or target is None:
            return "source or target is not alive"
        if source.node_id == target.node_id:
            return "source and target are the same node"
        if target.role is not Role.HEAVY:
            return "only a heavy node can host an agent"
        if p["agent"] in self._busy:
            return "this agent is already moving"
        return ""

    async def _on_teleport_request(self, message: Message) -> None:
        p = message.payload
        refusal = self._refusal(p)
        if refusal:
            await self.bus.publish(
                "teleport.result",
                {"request": p["request"], "ok": False, "error": refusal},
            )
            return
        self._busy.add(p["agent"])
        try:
            outcome = await self.teleporter(p["agent"], p["source"], p["target"])
        finally:
            self._busy.discard(p["agent"])
        await self.bus.publish(
            "teleport.result",
            {
                "request": p["request"],
                "ok": outcome.state == "committed",
                "state": outcome.state,
                "error": outcome.error,
            },
        )
