"""The Rust core and the Python reference read and write the same capsules.

The binary is built by `./verify.sh` (or `cargo build`) and found through
INARI_RS_BIN or the default target directory."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from inari_core.capsule import EventKind, append_event, load, verify
from inari_core.teleport import CapsuleHost, teleport

from .conftest import make_capsule

ROOT = Path(__file__).resolve().parent.parent
BIN = Path(os.environ.get("INARI_RS_BIN") or ROOT / "target" / "debug" / "inari-core-rs")
pytestmark = pytest.mark.skipif(
    not BIN.exists() and not os.environ.get("CI"), reason="Rust binary not built (cargo build)"
)


def rs(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run([str(BIN), *map(str, args)], capture_output=True, text=True, timeout=60)


def test_rust_verifies_a_python_capsule_and_reads_the_same_journal(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    out = rs("verify", capsule.root)
    assert out.returncode == 0 and out.stdout.strip() == "ok", out.stderr
    info = json.loads(rs("inspect", capsule.root).stdout)
    assert info["events"] == len(capsule.timeline) and info["thread"] == "thread-1"


def test_python_verifies_what_rust_appended_hash_for_hash(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    text = "decided in Rust ✓ λ"
    assert rs("append", capsule.root, "rust@thread-1", "decision", text).returncode == 0
    assert verify(capsule.root) == []
    last = load(capsule.root).timeline.ordered()[-1]
    assert last.node_id == "rust@thread-1" and last.data["text"] == text
    assert last.hash == last.compute_hash()


def test_a_rust_capsule_lives_on_in_python_and_back(tmp_path):
    root = tmp_path / "rs"
    assert rs("create", root, "t9", "node-r").returncode == 0
    assert verify(root) == []
    append_event(root, "node-p@t9", EventKind.USER_MESSAGE, {"text": "from python"}, 2.0)
    assert rs("verify", root).returncode == 0


def test_both_report_tampering_in_the_same_words(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    (capsule.root / "memory/working_memory.md").write_text("tampered")
    out = rs("verify", capsule.root)
    assert out.returncode == 1
    assert "digest mismatch: memory/working_memory.md" in out.stderr.splitlines()
    assert "digest mismatch: memory/working_memory.md" in verify(capsule.root)


def test_a_photo_attached_by_rust_is_in_the_workspace_python_sees(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    photo = tmp_path / "shot.jpg"
    photo.write_bytes(b"\xff\xd8\xff\xe0" + b"\x00" * 64)
    got = json.loads(rs("attach", capsule.root, "inbox/shot.jpg", photo).stdout)
    assert got["mime"] == "image/jpeg" and got["path"] == "inbox/shot.jpg"
    assert verify(capsule.root) == []
    assert load(capsule.root).snapshot.tree["inbox/shot.jpg"] == got["sha256"]


def test_archives_cross_languages(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    assert rs("pack", capsule.root, tmp_path / "r.ssc").returncode == 0
    from inari_core.capsule import pack, unpack

    unpack(tmp_path / "r.ssc", tmp_path / "from-rust")
    assert verify(tmp_path / "from-rust") == []
    pack(capsule.root, tmp_path / "p.ssc")
    assert rs("verify", tmp_path / "p.ssc").returncode == 0


async def test_a_rust_made_capsule_teleports_between_python_nodes(tmp_path):
    root = tmp_path / "rs"
    assert rs("create", root, "t7", "node-a").returncode == 0
    a, b = CapsuleHost("node-a", tmp_path / "a"), CapsuleHost("node-b", tmp_path / "b")
    agent = a.adopt(root)
    out = await teleport(a, b, agent)
    assert out.state == "committed" and verify(b.agents / agent) == []
    assert rs("verify", b.agents / agent).returncode == 0
