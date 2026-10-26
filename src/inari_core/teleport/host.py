"""A node's local home for agents: capsules on disk, a shared blob store, and the
fencing record of who may run what. Every write is atomic and `recover()` makes
the record agree with the disk after a crash."""

from __future__ import annotations

import json
import os
import shutil
from collections.abc import Callable
from pathlib import Path

from ..capsule import io as capsule_io
from ..capsule.blobs import BlobStore
from ..capsule.models import EventKind, Holder, Manifest
from ..errors import InariError, IntegrityError
from .protocol import BLOB_PREFIX, FrozenAgent


class StaleEpoch(InariError):
    """The offered epoch is not newer than the one this node already holds."""


class CapsuleHost:
    def __init__(
        self,
        node_id: str,
        root: Path,
        *,
        on_prepare: Callable[[capsule_io.Capsule], None] | None = None,
        clock: Callable[[], float] = lambda: 0.0,
    ) -> None:
        self.node_id = node_id
        self.root = Path(root)
        self.agents = self.root / "agents"
        self.staging = self.root / "staging"
        self.store = BlobStore(self.root / "blobs")
        self.on_prepare = on_prepare
        self.clock = clock
        self.agents.mkdir(parents=True, exist_ok=True)
        self.staging.mkdir(parents=True, exist_ok=True)
        self._record_path = self.root / "holders.json"
        self._prepared: set[str] = set()
        self._sessions: dict[str, tuple[str, int]] = {}

    # -- the fencing record -------------------------------------------------
    def _records(self) -> dict[str, dict]:
        if self._record_path.exists():
            return json.loads(self._record_path.read_text())
        return {}

    def _save(self, records: dict[str, dict]) -> None:
        tmp = self._record_path.with_suffix(".part")
        tmp.write_text(json.dumps(records, sort_keys=True))
        os.replace(tmp, self._record_path)

    def status(self, agent_id: str) -> str:
        return self._records().get(agent_id, {}).get("status", "absent")

    def _set(self, agent_id: str, *, holder: str, epoch: int, status: str) -> None:
        records = self._records()
        records[agent_id] = {"holder": holder, "epoch": epoch, "status": status}
        self._save(records)

    async def holder(self, agent_id: str) -> tuple[str, int] | None:
        record = self._records().get(agent_id)
        if not record or record["status"] == "tombstoned":
            return None
        return record["holder"], record["epoch"]

    def recover(self) -> list[str]:
        """After a crash: trust the capsule on disk, drop half-finished staging."""
        fixed: list[str] = []
        for leftover in self.staging.iterdir():
            shutil.rmtree(leftover, ignore_errors=True)
        records = self._records()
        for path in sorted(self.agents.iterdir()):
            holder = capsule_io.load(path).manifest.holder
            known = records.get(path.name)
            if (
                holder
                and holder.node_id == self.node_id
                and (known is None or known["epoch"] < holder.epoch)
            ):
                self._set(
                    path.name,
                    holder=holder.node_id,
                    epoch=holder.epoch,
                    status="running",
                )
                fixed.append(path.name)
        return fixed

    # -- local life of an agent ----------------------------------------------
    def adopt(self, capsule_dir: Path) -> str:
        """Take ownership of an existing capsule as its running holder. The
        agent's identity is the capsule's thread id; it is returned."""
        agent_id = capsule_io.load(capsule_dir).manifest.thread_id
        dest = self.agents / agent_id
        shutil.copytree(capsule_dir, dest)
        capsule = capsule_io.load(dest)
        epoch = capsule.manifest.holder.epoch if capsule.manifest.holder else 1
        capsule_io.set_holder(dest, Holder(node_id=self.node_id, epoch=epoch))
        self._absorb_blobs(dest)
        self._set(agent_id, holder=self.node_id, epoch=epoch, status="running")
        return agent_id

    def _absorb_blobs(self, capsule_dir: Path) -> None:
        for blob in (capsule_dir / BLOB_PREFIX).glob("*"):
            self.store.put(blob.read_bytes())

    def _safe(self, base: Path, rel: str) -> Path:
        target = (base / rel).resolve()
        if base.resolve() not in target.parents:
            raise IntegrityError(f"path escapes the capsule: {rel}")
        return target

    # -- source side ---------------------------------------------------------
    async def freeze(self, agent_id: str) -> FrozenAgent:
        record = self._records().get(agent_id)
        if not record or record["holder"] != self.node_id or record["status"] != "running":
            raise InariError(f"{agent_id} is not running here")
        path = self.agents / agent_id
        capsule_io.append_event(
            path,
            f"{self.node_id}@{agent_id}",
            EventKind.CHECKPOINT,
            {"reason": "teleport"},
            self.clock(),
        )
        self._set(agent_id, holder=self.node_id, epoch=record["epoch"], status="frozen")
        return FrozenAgent(capsule_io.load(path).manifest, record["epoch"])

    async def read(self, agent_id: str, rel: str) -> bytes:
        return self._safe(self.agents / agent_id, rel).read_bytes()

    async def unfreeze(self, agent_id: str, epoch: int) -> None:
        record = self._records().get(agent_id)
        if record and record["status"] == "frozen" and record["epoch"] == epoch:
            self._set(agent_id, holder=self.node_id, epoch=epoch, status="running")

    async def tombstone(self, agent_id: str, epoch: int) -> None:
        self._set(agent_id, holder="", epoch=epoch, status="tombstoned")

    # -- target side ---------------------------------------------------------
    async def offer(self, session_id: str, manifest: Manifest, epoch: int) -> set[str]:
        known = self._records().get(manifest.thread_id)
        if known and known["epoch"] >= epoch:
            raise StaleEpoch(f"epoch {epoch} is not newer than {known['epoch']}")
        stage = self.staging / session_id
        shutil.rmtree(stage, ignore_errors=True)
        stage.mkdir(parents=True)
        (stage / "manifest.json").write_text(manifest.model_dump_json(indent=2))
        self._sessions[session_id] = (manifest.thread_id, epoch)
        wanted = [rel[len(BLOB_PREFIX) :] for rel in manifest.files if rel.startswith(BLOB_PREFIX)]
        return self.store.have(wanted)

    async def receive(self, session_id: str, rel: str, data: bytes) -> None:
        target = self._safe(self.staging / session_id, rel)
        target.parent.mkdir(parents=True, exist_ok=True)
        tmp = target.with_name(target.name + ".part")
        tmp.write_bytes(data)
        os.replace(tmp, target)

    async def verify(self, session_id: str) -> None:
        stage = self.staging / session_id
        manifest = Manifest.model_validate_json((stage / "manifest.json").read_text())
        for rel in manifest.files:
            if rel.startswith(BLOB_PREFIX) and not (stage / rel).exists():
                sha = rel[len(BLOB_PREFIX) :]
                if self.store.has(sha):  # the differential part: already here
                    await self.receive(session_id, rel, self.store.get(sha))
        problems = capsule_io.verify(stage)
        if problems:
            raise IntegrityError("; ".join(problems))

    async def prepare(self, session_id: str) -> None:
        capsule = capsule_io.load(self.staging / session_id)
        if self.on_prepare:
            self.on_prepare(capsule)  # e.g. render the journal for the local provider
        self._prepared.add(session_id)

    async def activate(self, session_id: str, epoch: int) -> None:
        if session_id not in self._prepared:
            raise InariError("activate before prepare")
        agent_id, offered = self._sessions[session_id]
        if epoch != offered:
            raise StaleEpoch(f"session was offered epoch {offered}, not {epoch}")
        stage = self.staging / session_id
        capsule_io.set_holder(stage, Holder(node_id=self.node_id, epoch=epoch))
        dest = self.agents / agent_id
        shutil.rmtree(dest, ignore_errors=True)
        os.replace(stage, dest)
        self._absorb_blobs(dest)
        self._set(agent_id, holder=self.node_id, epoch=epoch, status="running")
        self._prepared.discard(session_id)

    async def abort(self, session_id: str) -> None:
        shutil.rmtree(self.staging / session_id, ignore_errors=True)
        self._prepared.discard(session_id)
        self._sessions.pop(session_id, None)
