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


async def rust_endpoint(root, node, key_file):
    """Start a Rust endpoint server; returns (process, port)."""
    import asyncio

    proc = await asyncio.create_subprocess_exec(
        str(BIN), "serve-endpoint", str(root), node, str(key_file), "0",
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )  # fmt: skip
    line = (await asyncio.wait_for(proc.stdout.readline(), 10)).decode()
    assert line.startswith("listening "), line
    return proc, int(line.split()[1])


async def test_a_python_node_teleports_an_agent_to_a_rust_node_over_tcp(tmp_path):
    from inari_core.transport import RemoteEndpoint

    (tmp_path / "mesh.key").write_text("mesh-key")
    source = CapsuleHost("node-a", tmp_path / "a")
    agent = source.adopt(make_capsule(tmp_path / "seed").root)
    proc, port = await rust_endpoint(tmp_path / "r", "node-r", tmp_path / "mesh.key")
    try:
        out = await teleport(
            source, RemoteEndpoint("node-r", "127.0.0.1", port, b"mesh-key"), agent
        )
        assert out.state == "committed", out
        assert source.status(agent) == "tombstoned"
        assert verify(tmp_path / "r" / "agents" / agent) == []
        assert rs("status", tmp_path / "r", "node-r", agent).stdout.strip() == "running"
        # and the wrong key never gets in
        wrong = await teleport(
            CapsuleHost("node-c", tmp_path / "c"),
            RemoteEndpoint("node-r", "127.0.0.1", port, b"x"),
            "none",
        )
        assert wrong.state == "rolled_back"
    finally:
        proc.kill()
        await proc.wait()


async def test_a_rust_node_teleports_an_agent_to_a_python_node_over_tcp(tmp_path):
    import asyncio

    from inari_core.transport import EndpointServer

    (tmp_path / "mesh.key").write_text("mesh-key")
    seed = make_capsule(tmp_path / "seed")
    assert rs("adopt", tmp_path / "r", "node-r", seed.root).returncode == 0
    target = CapsuleHost("node-b", tmp_path / "b")
    server = EndpointServer(target, b"mesh-key")
    port = await server.start()
    try:
        proc = await asyncio.create_subprocess_exec(
            str(BIN), "teleport", str(tmp_path / "r"), "node-r", "thread-1", "node-b",
            f"127.0.0.1:{port}", str(tmp_path / "mesh.key"), stdout=asyncio.subprocess.PIPE,
        )  # fmt: skip
        out = json.loads((await asyncio.wait_for(proc.communicate(), 30))[0])
        assert out["state"] == "committed", out
        assert await target.holder("thread-1") == ("node-b", 2)
        assert verify(target.agents / "thread-1") == []
        assert rs("status", tmp_path / "r", "node-r", "thread-1").stdout.strip() == "tombstoned"
    finally:
        await server.stop()


def test_a_rust_fork_and_merge_are_valid_python_capsules(tmp_path):
    base = make_capsule(tmp_path / "base")
    first = base.timeline.ordered()[0].id
    assert rs("fork", base.root, tmp_path / "child", "thread-2", first).returncode == 0
    child = load(tmp_path / "child")
    assert [e.id for e in child.timeline.ordered()] == [first]
    assert child.manifest.lineage.parent_capsule == base.manifest.capsule_id
    assert child.manifest.holder is None and verify(tmp_path / "child") == []
    # two forks grow apart, Rust merges them, Python agrees
    from inari_core.capsule import fork

    fork(base.root, tmp_path / "a", "ta")
    assert rs("fork", base.root, tmp_path / "b", "tb").returncode == 0
    append_event(tmp_path / "a", "n@ta", EventKind.DECISION, {"text": "left"}, 9.0)
    assert rs("append", tmp_path / "b", "n@tb", "decision", "right").returncode == 0
    assert rs("merge", tmp_path / "a", tmp_path / "b").stdout.strip() == "1"
    merged = load(tmp_path / "a")
    assert verify(tmp_path / "a") == []
    texts = {e.data.get("text") for e in merged.timeline.ordered()}
    assert {"left", "right"} <= texts


async def test_python_and_rust_buses_exchange_messages_both_ways(tmp_path):
    import asyncio

    from inari_core.mesh import Bus
    from inari_core.transport import BusClient, BusServer

    (tmp_path / "k").write_text("bus-key")
    # Rust server, Python client: the Rust side echoes what it receives
    proc = await asyncio.create_subprocess_exec(
        str(BIN), "bus-echo", "rust-hub", str(tmp_path / "k"), "py-phone",
        stdout=asyncio.subprocess.PIPE,
    )  # fmt: skip
    port = int((await asyncio.wait_for(proc.stdout.readline(), 10)).split()[1])
    phone = Bus("py-phone")
    got: asyncio.Queue = asyncio.Queue()
    phone.subscribe("echo.>", got.put_nowait)
    client = BusClient(phone, b"bus-key", "127.0.0.1", port, retry=0.05)
    client.start()
    try:
        for _ in range(100):
            if client.link.connected:
                break
            await asyncio.sleep(0.03)
        await phone.publish("teleport.request", {"n": 42})
        echo = await asyncio.wait_for(got.get(), 5)
        assert echo.subject == "echo.teleport.request" and echo.payload == {"n": 42}
    finally:
        await client.stop()
        proc.kill()
        await proc.wait()

    # Python server, Rust client
    home = Bus("py-home")
    seen = []
    home.subscribe("approval.>", lambda m: seen.append(m.payload))
    home.subscribe(
        "approval.request",
        lambda m: asyncio.ensure_future(home.publish("approval.response", {"ok": True})),
    )
    server = BusServer(home, b"bus-key", expected_peers=("rust-phone",))
    port = await server.start()
    try:
        proc = await asyncio.create_subprocess_exec(
            str(BIN), "bus-say", "rust-phone", str(tmp_path / "k"), f"127.0.0.1:{port}",
            "approval.request", '{"cmd": "ls"}', "approval.response",
            stdout=asyncio.subprocess.PIPE,
        )  # fmt: skip
        out, _ = await asyncio.wait_for(proc.communicate(), 15)
        assert proc.returncode == 0, out
        assert json.loads(out)["payload"] == {"ok": True}
        assert {"cmd": "ls"} in seen
    finally:
        await server.stop()


def test_a_rust_turn_sends_the_photo_and_python_reads_the_journal(tmp_path):
    capsule = make_capsule(tmp_path / "py")
    node = tmp_path / "node"
    agent = rs("adopt", node, "n-rs", capsule.root).stdout.strip()
    photo = tmp_path / "photo.png"
    photo.write_bytes(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
    log = tmp_path / "acp.log"
    fake = ["env", f"FAKE_ACP_LOG={log}", "FAKE_ACP_IMAGES=1"]
    fake += ["python3", ROOT / "tests/fake_acp.py"]
    out = rs("turn", node, "n-rs", agent, "what is this", photo, "--", *fake)
    assert out.returncode == 0, out.stdout + out.stderr
    assert json.loads(out.stdout)["text"].startswith("fake reply to:")
    sent = json.loads(log.read_text().splitlines()[-1])
    assert sent["blocks"] == ["text", "image"]
    held = node / "agents" / agent
    assert verify(held) == []
    events = load(held).timeline.ordered()
    user = next(e for e in events if e.kind == EventKind.USER_MESSAGE and e.data.get("attachments"))
    assert user.data["attachments"][0]["mime"] == "image/png"
    assert events[-1].kind == EventKind.ASSISTANT_MESSAGE
    assert "photo.png" in load(held).snapshot.tree
