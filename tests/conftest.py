from __future__ import annotations

from pathlib import Path

import pytest

from inari_core.capsule import (
    BlobStore,
    EnvironmentSpec,
    EventKind,
    Holder,
    Timeline,
    VectorIndex,
    create,
    snapshot_tree,
)


def make_capsule(root: Path, *, node: str = "node-a", epoch: int = 1, files=None):
    """A small but complete capsule with a workspace snapshot."""
    work = root.parent / (root.name + "-work")
    work.mkdir(parents=True, exist_ok=True)
    for name, text in (files or {"a.txt": "alpha", "src/b.py": "print('b')"}).items():
        (work / name).parent.mkdir(parents=True, exist_ok=True)
        (work / name).write_text(text)
    store = BlobStore(root.parent / (root.name + "-store"))
    snapshot = snapshot_tree(work, store)
    timeline = Timeline()
    timeline.append(node, EventKind.USER_MESSAGE, {"text": "build it"}, 1.0)
    timeline.append(
        node,
        EventKind.TOOL_CALL,
        {"call_id": "c1", "name": "bash", "arguments": {"cmd": "ls"}},
        2.0,
    )
    timeline.append(
        node,
        EventKind.TOOL_RESULT,
        {"call_id": "c1", "output": "a.txt", "is_error": False},
        3.0,
    )
    index = VectorIndex()
    index.add("n1", "the build uses a makefile")
    return create(
        root,
        thread_id="thread-1",
        environment=EnvironmentSpec(image="base:1", env={"LANG": "C"}),
        timeline=timeline,
        working_memory="# Goal\nbuild it\n",
        index=index,
        snapshot=snapshot,
        blobs=store,
        holder=Holder(node_id=node, epoch=epoch),
        now=1.0,
    )


@pytest.fixture
def capsule(tmp_path: Path):
    return make_capsule(tmp_path / "caps")
