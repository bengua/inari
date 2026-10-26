"""Content-addressed blobs and workspace snapshots: differential by construction."""

from __future__ import annotations

import os
from collections.abc import Iterable
from pathlib import Path

from ..canon import sha256_hex
from ..errors import IntegrityError
from .models import Snapshot


class BlobStore:
    def __init__(self, root: Path) -> None:
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)

    def path(self, sha: str) -> Path:
        return self.root / sha

    def has(self, sha: str) -> bool:
        return self.path(sha).is_file()

    def have(self, shas: Iterable[str]) -> set[str]:
        return {s for s in shas if self.has(s)}

    def put(self, data: bytes) -> str:
        sha = sha256_hex(data)
        target = self.path(sha)
        if not target.exists():
            tmp = target.with_suffix(".part")
            tmp.write_bytes(data)
            os.replace(tmp, target)  # atomic: a crash leaves no half blob
        return sha

    def get(self, sha: str) -> bytes:
        data = self.path(sha).read_bytes()
        if sha256_hex(data) != sha:
            raise IntegrityError(f"blob {sha[:12]} is corrupt")
        return data


def snapshot_tree(root: Path, store: BlobStore, parent: Snapshot | None = None):
    """Hash every file under `root` into `store`; return the snapshot."""
    tree: dict[str, str] = {}
    for path in sorted(p for p in Path(root).rglob("*") if p.is_file()):
        tree[path.relative_to(root).as_posix()] = store.put(path.read_bytes())
    return Snapshot(parent=parent.digest() if parent else None, tree=tree)


def diff(old: Snapshot, new: Snapshot) -> dict[str, list[str]]:
    added = sorted(set(new.tree) - set(old.tree))
    deleted = sorted(set(old.tree) - set(new.tree))
    both = set(old.tree) & set(new.tree)
    changed = sorted(p for p in both if old.tree[p] != new.tree[p])
    return {"added": added, "deleted": deleted, "modified": changed}


def restore(snapshot: Snapshot, store: BlobStore, dest: Path) -> None:
    dest = Path(dest)
    for rel, sha in snapshot.tree.items():
        target = (dest / rel).resolve()
        if dest.resolve() not in target.parents:
            raise IntegrityError(f"path escapes the workspace: {rel}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(store.get(sha))
