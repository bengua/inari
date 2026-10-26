"""Fork a capsule at an event, and merge one capsule's journal into another."""

from __future__ import annotations

import shutil
import uuid
from pathlib import Path

from ..errors import InariError
from . import io
from .models import Holder
from .timeline import Timeline
from .vectors import VectorIndex


def fork(src: Path, dest: Path, new_thread: str, upto_event_id: str | None = None) -> io.Capsule:
    """Copy `src` to `dest` as a new thread whose journal stops at `upto_event_id`
    (default: everything). The fork records where it came from; nobody holds it yet."""
    source = io.load(src)
    events = source.timeline.ordered()
    if upto_event_id is not None:
        ids = [e.id for e in events]
        if upto_event_id not in ids:
            raise InariError(f"no event {upto_event_id} in the source journal")
        events = events[: ids.index(upto_event_id) + 1]
    shutil.copytree(src, dest, ignore=shutil.ignore_patterns("signature"))
    timeline = Timeline(events)
    lineage = source.manifest.lineage.model_copy(
        update={
            "parent_capsule": source.manifest.capsule_id,
            "fork_event_id": events[-1].id if events else None,
        }
    )
    manifest = source.manifest.model_copy(
        update={
            "capsule_id": str(uuid.uuid4()),
            "thread_id": new_thread,
            "lineage": lineage,
            "holder": None,
        }
    )
    (dest / io.TIMELINE).write_text(timeline.dumps())
    io.seal(dest, manifest, timeline)
    return io.load(dest)


def merge(dest: Path, src: Path) -> int:
    """Bring `src`'s journal and notes into `dest`. Returns the events added.
    Working memory and workspace stay `dest`'s: the holder alone writes them."""
    target, other = io.load(dest), io.load(src)
    added = target.timeline.merge(other.timeline)  # raises ConflictError on equivocation
    index = VectorIndex() if not target.index.items else target.index
    for item_id, (text, _) in other.index.items.items():
        if item_id not in index.items:
            index.add(item_id, text)
    (Path(dest) / io.TIMELINE).write_text(target.timeline.dumps())
    (Path(dest) / io.VECTORS).write_text(index.dumps())
    merged = [*target.manifest.lineage.merged_from, other.manifest.capsule_id]
    manifest = target.manifest.model_copy(
        update={"lineage": target.manifest.lineage.model_copy(update={"merged_from": merged})}
    )
    io.seal(Path(dest), manifest, target.timeline)
    return added


__all__ = ["Holder", "fork", "merge"]
