from __future__ import annotations

import asyncio
import subprocess
import sys

from inari_core import signature
from inari_core.approval_bus import NodeApprovals, PhoneApprover
from inari_core.approvals import ApprovalGate
from inari_core.capsule import verify
from inari_core.mesh import Bus, connect
from inari_core.transport import BusClient, BusServer
from tests.conftest import make_capsule

KEY = b"device-key"


def test_a_signed_capsule_verifies_and_any_change_breaks_it(tmp_path):
    capsule = make_capsule(tmp_path / "caps")
    private, public = signature.generate_keypair()
    signature.sign_capsule(capsule.root, private)
    assert verify(capsule.root) == []  # the signature dir is not an undeclared file
    assert signature.verify_signature(capsule.root, {public}) == []
    (capsule.root / "memory/working_memory.md").write_text("changed")
    from inari_core.capsule import load, seal

    loaded = load(capsule.root)
    seal(capsule.root, loaded.manifest, loaded.timeline)  # even a re-sealed capsule
    assert (
        "signature does not match"
        in signature.verify_signature(capsule.root, {public})[0]
    )


def test_unsigned_and_foreign_signatures_are_refused(tmp_path):
    capsule = make_capsule(tmp_path / "caps")
    _, public = signature.generate_keypair()
    assert signature.verify_signature(capsule.root, {public}) == ["not signed"]
    other_private, _ = signature.generate_keypair()
    signature.sign_capsule(capsule.root, other_private)
    assert "untrusted key" in signature.verify_signature(capsule.root, {public})[0]


def test_cli_keygen_sign_and_trusted_verify(tmp_path):
    capsule = make_capsule(tmp_path / "caps")
    run = lambda *a: subprocess.run(  # noqa: E731
        [sys.executable, "-m", "inari_core.cli", *a], capture_output=True, text=True
    )
    name = str(tmp_path / "me")
    assert run("keygen", name).returncode == 0
    assert run("sign", str(capsule.root), "--key", name + ".key").returncode == 0
    ok = run("verify", str(capsule.root), "--trusted", name + ".pub")
    assert ok.returncode == 0 and ok.stdout.strip() == "ok"
    (capsule.root / "memory/working_memory.md").write_text("x")
    assert run("verify", str(capsule.root), "--trusted", name + ".pub").returncode == 1


async def test_phone_approves_over_linked_buses():
    node, phone = Bus("node"), Bus("phone")
    connect(node, phone)
    PhoneApprover(phone, KEY, lambda req: "reboot" in req.command)
    approvals = NodeApprovals(node, ApprovalGate(KEY))
    assert await approvals.ask("a1", "n1", "sudo reboot", ttl=2) is True
    assert await approvals.ask("a1", "n1", "rm -rf /", ttl=2) is False


async def test_a_phone_with_the_wrong_key_cannot_approve():
    node, phone = Bus("node"), Bus("phone")
    connect(node, phone)
    PhoneApprover(phone, b"other-key", lambda req: True)
    approvals = NodeApprovals(node, ApprovalGate(KEY))
    assert await approvals.ask("a1", "n1", "sudo reboot", ttl=0.3) is False


async def test_a_question_waits_for_a_phone_that_comes_back_in_time():
    node, phone = Bus("node"), Bus("phone")
    server = BusServer(node, KEY, expected_peers=("phone",))
    port = await server.start()
    PhoneApprover(phone, KEY, lambda req: True)
    approvals = NodeApprovals(node, ApprovalGate(KEY))
    waiting = asyncio.create_task(approvals.ask("a1", "n1", "sudo reboot", ttl=5))
    await asyncio.sleep(0.2)  # the phone is not connected yet: the request queues
    client = BusClient(phone, KEY, "127.0.0.1", port, retry=0.05)
    client.start()
    assert await asyncio.wait_for(waiting, 5) is True
    await client.stop()
    await server.stop()


async def test_a_late_answer_is_too_late():
    node, phone = Bus("node"), Bus("phone")
    connect(node, phone)

    async def slow(_):
        await asyncio.sleep(0.5)
        return True

    PhoneApprover(phone, KEY, slow)
    approvals = NodeApprovals(node, ApprovalGate(KEY))
    assert await approvals.ask("a1", "n1", "sudo reboot", ttl=0.2) is False
