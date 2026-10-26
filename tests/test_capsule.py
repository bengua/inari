from __future__ import annotations

import json
import tarfile

import pytest
from pydantic import ValidationError

from inari_core.capsule import (
    BlobStore,
    EnvironmentSpec,
    EventKind,
    Timeline,
    VectorIndex,
    append_event,
    diff,
    pack,
    restore,
    snapshot_tree,
    unpack,
    verify,
)
from inari_core.errors import ConflictError, IntegrityError


def test_fresh_capsule_verifies(capsule):
    assert verify(capsule.root) == []


def test_capsule_is_plain_files_a_human_can_read(capsule):
    names = {p.relative_to(capsule.root).as_posix() for p in capsule.root.rglob("*")}
    assert "manifest.json" in names and "memory/timeline_events.jsonl" in names
    assert json.loads((capsule.root / "manifest.json").read_text())["format"] == "ssc/1"
    assert "build it" in (capsule.root / "memory/timeline_events.jsonl").read_text()


def test_tampering_with_a_file_is_detected(capsule):
    (capsule.root / "memory/working_memory.md").write_text("# rewritten\n")
    assert any("digest mismatch" in p for p in verify(capsule.root))


def test_tampering_with_the_journal_is_detected(capsule):
    path = capsule.root / "memory/timeline_events.jsonl"
    path.write_text(path.read_text().replace("build it", "delete it"))
    problems = verify(capsule.root)
    assert any("hash does not match" in p for p in problems)


def test_extra_and_missing_files_are_reported(capsule):
    (capsule.root / "stray.txt").write_text("x")
    gone = next((capsule.root / "artifacts/blobs").iterdir())
    gone.unlink()
    problems = verify(capsule.root)
    assert "undeclared file: stray.txt" in problems
    assert f"missing file: artifacts/blobs/{gone.name}" in problems


def test_pack_is_deterministic_and_round_trips(capsule, tmp_path):
    one, two = tmp_path / "one.ssc", tmp_path / "two.ssc"
    pack(capsule.root, one)
    pack(capsule.root, two)
    assert one.read_bytes() == two.read_bytes()
    unpack(one, tmp_path / "out")
    assert verify(tmp_path / "out") == []


def test_unpack_refuses_path_traversal(tmp_path):
    evil = tmp_path / "evil.tar"
    with tarfile.open(evil, "w") as tar:
        info = tarfile.TarInfo("../escape.txt")
        info.size = 0
        tar.addfile(info)
    with pytest.raises(IntegrityError):
        unpack(evil, tmp_path / "dest")


def test_environment_refuses_secret_values():
    with pytest.raises(ValidationError):
        EnvironmentSpec(env={"API_TOKEN": "abc"})
    assert EnvironmentSpec(secret_refs=["API_TOKEN"]).secret_refs == ["API_TOKEN"]


def test_append_event_keeps_the_capsule_sound(capsule):
    updated = append_event(capsule.root, "node-a", EventKind.DECISION, {"text": "go"}, 9.0)
    assert len(updated.timeline) == 4
    assert verify(capsule.root) == []


def test_timeline_merge_is_a_union_and_order_independent():
    a, b = Timeline(), Timeline()
    a.append("n1", EventKind.USER_MESSAGE, {"text": "1"}, 1.0)
    b.append("n2", EventKind.USER_MESSAGE, {"text": "2"}, 1.5)
    b.append("n2", EventKind.ASSISTANT_MESSAGE, {"text": "3"}, 2.0)
    left, right = Timeline(a.ordered()), Timeline(b.ordered())
    left.merge(b)
    right.merge(a)
    assert left.head() == right.head()
    assert [e.id for e in left.ordered()] == [e.id for e in right.ordered()]
    assert left.merge(b) == 0  # idempotent
    assert left.verify() == []


def test_timeline_rejects_two_events_with_one_id():
    a, b = Timeline(), Timeline()
    a.append("n", EventKind.USER_MESSAGE, {"text": "left"}, 1.0)
    b.append("n", EventKind.USER_MESSAGE, {"text": "right"}, 1.0)
    with pytest.raises(ConflictError):
        a.merge(b)


def test_vector_index_finds_the_relevant_note():
    index = VectorIndex()
    index.add("a", "the build uses a makefile and gcc")
    index.add("b", "the user prefers dark themes in the editor")
    assert index.search("the makefile", k=1)[0][0] == "a"
    assert VectorIndex.loads(index.dumps()).search("makefile", k=1)[0][0] == "a"


def test_snapshots_are_differential(tmp_path):
    store = BlobStore(tmp_path / "store")
    work = tmp_path / "work"
    work.mkdir()
    (work / "keep.txt").write_text("same")
    (work / "edit.txt").write_text("v1")
    first = snapshot_tree(work, store)
    (work / "edit.txt").write_text("v2")
    (work / "new.txt").write_text("n")
    second = snapshot_tree(work, store, parent=first)
    assert diff(first, second) == {
        "added": ["new.txt"],
        "deleted": [],
        "modified": ["edit.txt"],
    }
    out = tmp_path / "restored"
    restore(second, store, out)
    assert (out / "edit.txt").read_text() == "v2"
