"""Schemas of the Session State Capsule (SSC). One source of truth: these models
generate the JSON Schemas in `schema/`, which TypeScript and Rust clients consume."""

from __future__ import annotations

import re
from enum import StrEnum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from ..canon import digest_of

FORMAT = "ssc/1"
SECRET_NAME = re.compile(r"(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL)", re.I)


class EventKind(StrEnum):
    SYSTEM = "system"
    USER_MESSAGE = "user_message"
    ASSISTANT_MESSAGE = "assistant_message"
    THOUGHT = "thought"
    DECISION = "decision"
    TOOL_CALL = "tool_call"
    TOOL_RESULT = "tool_result"
    PROVIDER_SWITCH = "provider_switch"
    CHECKPOINT = "checkpoint"


class TimelineEvent(BaseModel):
    """One immutable entry. `prev` chains the events of a single node; `lamport`
    gives the cross-node order, so merging two timelines is a set union."""

    model_config = ConfigDict(frozen=True)

    id: str
    node_id: str
    node_seq: int = Field(ge=1)
    lamport: int = Field(ge=1)
    ts: float
    kind: EventKind
    data: dict[str, Any] = Field(default_factory=dict)
    prev: str = ""
    hash: str = ""

    def body(self) -> dict[str, Any]:
        return self.model_dump(mode="json", exclude={"hash"})

    def compute_hash(self) -> str:
        return digest_of(self.body())

    @classmethod
    def make(
        cls,
        *,
        node_id: str,
        node_seq: int,
        lamport: int,
        ts: float,
        kind: EventKind,
        data: dict[str, Any] | None = None,
        prev: str = "",
    ) -> TimelineEvent:
        draft = cls(
            id=f"{node_id}:{node_seq}",
            node_id=node_id,
            node_seq=node_seq,
            lamport=lamport,
            ts=ts,
            kind=kind,
            data=data or {},
            prev=prev,
        )
        return draft.model_copy(update={"hash": draft.compute_hash()})


class FileDigest(BaseModel):
    sha256: str
    size: int = Field(ge=0)


class Holder(BaseModel):
    """Who may run the agent. The highest `epoch` wins: a fencing token."""

    node_id: str
    epoch: int = Field(ge=0)


class Lineage(BaseModel):
    parent_capsule: str | None = None
    fork_event_id: str | None = None
    merged_from: list[str] = Field(default_factory=list)


class AccessPolicy(BaseModel):
    owners: list[str] = Field(default_factory=list)
    readers: list[str] = Field(default_factory=list)
    # Glob patterns over tool names / commands that need a human decision.
    approval_required: list[str] = Field(default_factory=list)


class Manifest(BaseModel):
    format: Literal["ssc/1"] = FORMAT
    capsule_id: str
    thread_id: str
    created_at: float
    title: str = ""
    labels: dict[str, str] = Field(default_factory=dict)
    lineage: Lineage = Field(default_factory=Lineage)
    access: AccessPolicy = Field(default_factory=AccessPolicy)
    holder: Holder | None = None
    timeline_head: str = ""
    files: dict[str, FileDigest] = Field(default_factory=dict)

    def content_digest(self) -> str:
        """Identity of the capsule's content (what a transfer must reproduce)."""
        return digest_of({k: v.model_dump() for k, v in sorted(self.files.items())})


class Mount(BaseModel):
    source: str  # a logical volume name, never a host path
    target: str
    mode: Literal["ro", "rw"] = "rw"


class EnvironmentSpec(BaseModel):
    """Reproducible sandbox definition. Holds secret NAMES, never values."""

    image: str = ""
    workdir: str = "/work"
    mounts: list[Mount] = Field(default_factory=list)
    env: dict[str, str] = Field(default_factory=dict)
    secret_refs: list[str] = Field(default_factory=list)
    resources: dict[str, str] = Field(default_factory=dict)

    @field_validator("env")
    @classmethod
    def _no_secret_in_env(cls, value: dict[str, str]) -> dict[str, str]:
        bad = sorted(k for k in value if SECRET_NAME.search(k))
        if bad:
            raise ValueError(f"{bad}: secrets go in secret_refs and are injected at run time")
        return value


class Snapshot(BaseModel):
    """A workspace tree as path -> content hash. Blobs live in a content store,
    so moving a workspace only moves the blobs the target lacks."""

    parent: str | None = None
    tree: dict[str, str] = Field(default_factory=dict)

    def digest(self) -> str:
        return digest_of(self.model_dump())
