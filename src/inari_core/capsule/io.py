"""Reading, writing, verifying and archiving a capsule directory.

Layout (every file is plain JSON, JSONL or Markdown; the archive is a tar):

    manifest.json
    memory/working_memory.md
    memory/timeline_events.jsonl
    memory/vector_index/index.json
    artifacts/snapshot.json
    artifacts/blobs/<sha256>
    environment/spec.json
"""

from __future__ import annotations

import io
import os
import tarfile
import uuid
from dataclasses import dataclass
from pathlib import Path

from ..canon import sha256_hex
from ..errors import IntegrityError
from .blobs import BlobStore, snapshot_tree
from .models import (
    AccessPolicy,
    EnvironmentSpec,
    EventKind,
    FileDigest,
    Holder,
    Lineage,
    Manifest,
    Snapshot,
)
from .timeline import Timeline
from .vectors import VectorIndex

MANIFEST = "manifest.json"
WORKING_MEMORY = "memory/working_memory.md"
TIMELINE = "memory/timeline_events.jsonl"
VECTORS = "memory/vector_index/index.json"
SNAPSHOT = "artifacts/snapshot.json"
BLOBS = "artifacts/blobs"
ENVIRONMENT = "environment/spec.json"


@dataclass
class Capsule:
    root: Path
    manifest: Manifest
    environment: EnvironmentSpec
    working_memory: str
    timeline: Timeline
    index: VectorIndex
    snapshot: Snapshot

    def blobs(self) -> BlobStore:
        return BlobStore(self.root / BLOBS)


def _atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".part")
    tmp.write_bytes(data)
    os.replace(tmp, path)


def _scan(root: Path) -> dict[str, FileDigest]:
    files: dict[str, FileDigest] = {}
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        rel = path.relative_to(root).as_posix()
        if rel == MANIFEST or rel.startswith("signature/") or rel.endswith(".part"):
            continue
        data = path.read_bytes()
        files[rel] = FileDigest(sha256=sha256_hex(data), size=len(data))
    return files


def seal(root: Path, manifest: Manifest, timeline: Timeline) -> Manifest:
    """Recompute the digests and write `manifest.json`."""
    sealed = manifest.model_copy(update={"files": _scan(root), "timeline_head": timeline.head()})
    _atomic_write(root / MANIFEST, sealed.model_dump_json(indent=2).encode())
    return sealed


def create(
    root: Path,
    *,
    thread_id: str,
    environment: EnvironmentSpec,
    timeline: Timeline,
    working_memory: str = "",
    index: VectorIndex | None = None,
    snapshot: Snapshot | None = None,
    blobs: BlobStore | None = None,
    holder: Holder | None = None,
    lineage: Lineage | None = None,
    access: AccessPolicy | None = None,
    title: str = "",
    now: float = 0.0,
    capsule_id: str | None = None,
) -> Capsule:
    root = Path(root)
    snapshot = snapshot or Snapshot()
    index = index or VectorIndex()
    _atomic_write(root / WORKING_MEMORY, working_memory.encode())
    _atomic_write(root / TIMELINE, timeline.dumps().encode())
    _atomic_write(root / VECTORS, index.dumps().encode())
    _atomic_write(root / SNAPSHOT, snapshot.model_dump_json(indent=2).encode())
    _atomic_write(root / ENVIRONMENT, environment.model_dump_json(indent=2).encode())
    (root / BLOBS).mkdir(parents=True, exist_ok=True)
    if blobs is not None:
        for sha in sorted(set(snapshot.tree.values())):
            _atomic_write(root / BLOBS / sha, blobs.get(sha))
    manifest = Manifest(
        capsule_id=capsule_id or str(uuid.uuid4()),
        thread_id=thread_id,
        created_at=now,
        title=title,
        lineage=lineage or Lineage(),
        access=access or AccessPolicy(),
        holder=holder,
    )
    seal(root, manifest, timeline)
    return load(root)


def load(root: Path) -> Capsule:
    root = Path(root)
    return Capsule(
        root=root,
        manifest=Manifest.model_validate_json((root / MANIFEST).read_text()),
        environment=EnvironmentSpec.model_validate_json((root / ENVIRONMENT).read_text()),
        working_memory=(root / WORKING_MEMORY).read_text(),
        timeline=Timeline.loads((root / TIMELINE).read_text()),
        index=VectorIndex.loads((root / VECTORS).read_text()),
        snapshot=Snapshot.model_validate_json((root / SNAPSHOT).read_text()),
    )


def verify(root: Path) -> list[str]:
    """Every problem found; an empty list means the capsule is sound."""
    root = Path(root)
    problems: list[str] = []
    try:
        manifest = Manifest.model_validate_json((root / MANIFEST).read_text())
    except (OSError, ValueError) as exc:
        return [f"unreadable manifest: {exc}"]
    declared, actual = manifest.files, _scan(root)
    for rel in sorted(set(declared) - set(actual)):
        problems.append(f"missing file: {rel}")
    for rel in sorted(set(actual) - set(declared)):
        problems.append(f"undeclared file: {rel}")
    for rel in sorted(set(declared) & set(actual)):
        if declared[rel] != actual[rel]:
            problems.append(f"digest mismatch: {rel}")
    try:
        capsule = load(root)
    except (OSError, ValueError) as exc:
        return problems + [f"unreadable: {exc}"]
    problems += capsule.timeline.verify()
    if capsule.timeline.head() != manifest.timeline_head:
        problems.append("timeline head does not match the manifest")
    store = capsule.blobs()
    for rel, sha in capsule.snapshot.tree.items():
        if not store.has(sha):
            problems.append(f"snapshot blob absent: {rel}")
    return problems


def append_event(root: Path, node_id: str, kind: EventKind, data: dict, ts: float) -> Capsule:
    """Add one event to a live capsule and reseal it."""
    capsule = load(root)
    capsule.timeline.append(node_id, kind, data, ts)
    _atomic_write(Path(root) / TIMELINE, capsule.timeline.dumps().encode())
    seal(Path(root), capsule.manifest, capsule.timeline)
    return load(root)


def set_holder(root: Path, holder: Holder | None) -> Capsule:
    capsule = load(root)
    manifest = capsule.manifest.model_copy(update={"holder": holder})
    seal(Path(root), manifest, capsule.timeline)
    return load(root)


def pack(root: Path, archive: Path) -> None:
    """Write a deterministic tar: same capsule, same bytes."""
    root = Path(root)
    with tarfile.open(archive, "w") as tar:
        for path in sorted(p for p in root.rglob("*") if p.is_file()):
            rel = path.relative_to(root).as_posix()
            if rel.endswith(".part"):
                continue
            info = tarfile.TarInfo(rel)
            data = path.read_bytes()
            info.size, info.mtime, info.mode = len(data), 0, 0o644
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            tar.addfile(info, io.BytesIO(data))


def unpack(archive: Path, dest: Path) -> None:
    dest = Path(dest).resolve()
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r") as tar:
        for member in tar.getmembers():
            target = (dest / member.name).resolve()
            if not member.isfile() or dest not in target.parents:
                raise IntegrityError(f"unsafe archive member: {member.name}")
            extracted = tar.extractfile(member)
            assert extracted is not None
            _atomic_write(target, extracted.read())


def update_snapshot(root: Path, workdir: Path) -> Capsule:
    """Record the workspace as the capsule's new snapshot (differential: only new
    or changed files add a blob) and reseal."""
    capsule = load(root)
    snapshot = snapshot_tree(Path(workdir), capsule.blobs(), parent=capsule.snapshot)
    _atomic_write(Path(root) / SNAPSHOT, snapshot.model_dump_json(indent=2).encode())
    seal(Path(root), capsule.manifest, capsule.timeline)
    return load(root)
