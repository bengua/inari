"""Live state of the mesh, and its translation into the mascot's behaviour.

The mascot is a gauge, not a decoration: every visual parameter below is a pure
function of real telemetry, so the same snapshot always draws the same creature."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Any


class Phase(StrEnum):
    IDLE = "idle"
    LISTENING = "listening"
    INFERENCE = "inference"
    SUCCESS = "success"
    ERROR = "error"


# Palette from the design brief; ERROR is the one addition (a failure needs a colour).
COLOURS = {
    Phase.IDLE: "#22d3ee",  # cyan
    Phase.LISTENING: "#e879f9",  # magenta
    Phase.INFERENCE: "#fbbf24",  # amber
    Phase.SUCCESS: "#34d399",  # emerald
    Phase.ERROR: "#f87171",
}
PRIORITY = [Phase.ERROR, Phase.INFERENCE, Phase.LISTENING, Phase.SUCCESS, Phase.IDLE]


@dataclass
class Telemetry:
    agents: dict[str, dict[str, Any]] = field(default_factory=dict)
    nodes: dict[str, dict[str, float]] = field(default_factory=dict)

    def update_agent(self, agent_id: str, node_id: str, phase: str, **extra: Any) -> None:
        self.agents[agent_id] = {"node": node_id, "phase": Phase(phase), **extra}

    def update_node(self, node_id: str, *, gpu: float = 0.0, packets: float = 0.0) -> None:
        self.nodes[node_id] = {"gpu": gpu, "packets": packets}

    def drop_node(self, node_id: str) -> None:
        self.nodes.pop(node_id, None)

    def snapshot(self) -> dict[str, Any]:
        return {"agents": dict(self.agents), "nodes": dict(self.nodes)}


@dataclass(frozen=True)
class MascotParams:
    phase: Phase
    colour: str
    pulse_hz: float  # heartbeat of the orb
    turbulence: float  # 0 calm .. 1 storm: surface noise amplitude
    glow: float  # 0..1 halo strength


def mascot_state(snapshot: dict[str, Any]) -> MascotParams:
    phases = {a["phase"] for a in snapshot["agents"].values()}
    phase = next((p for p in PRIORITY if p in phases), Phase.IDLE)
    gpu = max((n["gpu"] for n in snapshot["nodes"].values()), default=0.0)
    packets = sum(n["packets"] for n in snapshot["nodes"].values())
    busy = sum(1 for a in snapshot["agents"].values() if a["phase"] is not Phase.IDLE)
    return MascotParams(
        phase=phase,
        colour=COLOURS[phase],
        pulse_hz=round(0.25 + 1.75 * min(1.0, gpu), 3),
        turbulence=round(min(1.0, packets / 2000.0), 3),
        glow=round(min(1.0, 0.2 + 0.2 * busy), 3),
    )
