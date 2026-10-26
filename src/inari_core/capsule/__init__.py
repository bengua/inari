"""The Session State Capsule: a self-describing, inspectable unit of agent state."""

from .blobs import BlobStore, diff, restore, snapshot_tree
from .io import (
    Capsule,
    append_event,
    create,
    load,
    pack,
    seal,
    set_holder,
    unpack,
    update_snapshot,
    verify,
)
from .models import (
    AccessPolicy,
    EnvironmentSpec,
    EventKind,
    FileDigest,
    Holder,
    Lineage,
    Manifest,
    Mount,
    Snapshot,
    TimelineEvent,
)
from .timeline import Timeline
from .vectors import HashingEmbedder, VectorIndex

__all__ = [
    "AccessPolicy",
    "BlobStore",
    "Capsule",
    "EnvironmentSpec",
    "EventKind",
    "FileDigest",
    "HashingEmbedder",
    "Holder",
    "Lineage",
    "Manifest",
    "Mount",
    "Snapshot",
    "Timeline",
    "TimelineEvent",
    "VectorIndex",
    "append_event",
    "create",
    "diff",
    "load",
    "pack",
    "restore",
    "seal",
    "set_holder",
    "snapshot_tree",
    "unpack",
    "update_snapshot",
    "verify",
]
