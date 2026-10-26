from __future__ import annotations

import pytest

from inari_core.capsule import EventKind, append_event, fork, load, merge, search, verify
from inari_core.errors import ConflictError
from tests.conftest import make_capsule


def test_fork_stops_at_the_chosen_event_and_records_its_origin(tmp_path):
    src = make_capsule(tmp_path / "src")
    first = src.timeline.ordered()[0].id
    child = fork(src.root, tmp_path / "child", "thread-2", upto_event_id=first)
    assert [e.id for e in child.timeline.ordered()] == [first]
    assert child.manifest.thread_id == "thread-2" and child.manifest.holder is None
    assert child.manifest.lineage.parent_capsule == src.manifest.capsule_id
    assert child.manifest.lineage.fork_event_id == first
    assert verify(child.root) == [] and verify(src.root) == []


def test_two_forks_that_grew_apart_merge_without_conflict(tmp_path):
    base = make_capsule(tmp_path / "base")
    a = fork(base.root, tmp_path / "a", "thread-a")
    b = fork(base.root, tmp_path / "b", "thread-b")
    append_event(a.root, "node-1@thread-a", EventKind.DECISION, {"text": "left"}, 10.0)
    append_event(b.root, "node-1@thread-b", EventKind.DECISION, {"text": "right"}, 11.0)
    assert merge(a.root, b.root) == 1
    merged = load(a.root)
    texts = [e.data.get("text") for e in merged.timeline.ordered() if e.kind is EventKind.DECISION]
    assert texts == ["left", "right"] and verify(a.root) == []
    assert merged.manifest.lineage.merged_from == [b.manifest.capsule_id]


def test_merge_refuses_two_histories_that_claim_the_same_event(tmp_path):
    base = make_capsule(tmp_path / "base")
    a = fork(base.root, tmp_path / "a", "thread-a")
    b = fork(base.root, tmp_path / "b", "thread-b")
    append_event(a.root, "node-a", EventKind.DECISION, {"text": "x"}, 10.0)
    append_event(b.root, "node-a", EventKind.DECISION, {"text": "y"}, 10.0)
    with pytest.raises(ConflictError):
        merge(a.root, b.root)


def test_search_finds_exact_words_before_near_matches(tmp_path):
    cap = make_capsule(tmp_path / "caps")
    hits = search(cap.timeline, "build")
    assert hits and hits[0]["kind"] == "user_message"
    assert search(cap.timeline, "zzz-not-there") == []
