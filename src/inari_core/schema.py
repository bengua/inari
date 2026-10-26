"""JSON Schemas generated from the models: the contract for non-Python clients."""

from __future__ import annotations

import json
from pathlib import Path

from .approvals import ApprovalRequest, ApprovalResponse
from .capsule.models import EnvironmentSpec, Manifest, Snapshot, TimelineEvent

MODELS = {
    "manifest": Manifest,
    "timeline-event": TimelineEvent,
    "environment": EnvironmentSpec,
    "snapshot": Snapshot,
    "approval-request": ApprovalRequest,
    "approval-response": ApprovalResponse,
}


def render() -> dict[str, str]:
    return {
        f"{name}.schema.json": json.dumps(m.model_json_schema(), indent=2, sort_keys=True) + "\n"
        for name, m in MODELS.items()
    }


def export(dest: Path) -> list[Path]:
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    written = []
    for name, text in render().items():
        (dest / name).write_text(text)
        written.append(dest / name)
    return written
