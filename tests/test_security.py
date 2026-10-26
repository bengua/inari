from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from inari_core import schema
from inari_core.approvals import (
    ApprovalGate,
    ApprovalResponse,
    needs_approval,
    sign,
)
from inari_core.capsule import EnvironmentSpec, pack
from inari_core.secrets import EnvResolver, MissingSecret, ephemeral_env, leaks, scrub
from tests.conftest import make_capsule

KEY = b"device-shared-key"


def test_secret_values_are_injected_for_a_run_and_never_written(tmp_path, monkeypatch):
    monkeypatch.setenv("INARI_SECRET_PROVIDER_API_KEY", "sk-very-secret-value")
    capsule = make_capsule(tmp_path / "caps")
    spec = EnvironmentSpec(env={"LANG": "C"}, secret_refs=["PROVIDER_API_KEY"])
    env = ephemeral_env(spec, EnvResolver())
    assert env["PROVIDER_API_KEY"] == "sk-very-secret-value" and env["LANG"] == "C"
    archive = tmp_path / "out.ssc"
    pack(capsule.root, archive)
    assert leaks(capsule.root, ["sk-very-secret-value"]) == []
    assert b"sk-very-secret-value" not in archive.read_bytes()
    assert scrub("token sk-very-secret-value here", ["sk-very-secret-value"]) == (
        "token [redacted] here"
    )


def test_a_missing_secret_is_named_without_its_value():
    spec = EnvironmentSpec(secret_refs=["A_TOKEN", "B_TOKEN"])
    with pytest.raises(MissingSecret) as err:
        ephemeral_env(spec, EnvResolver(prefix="NOPE_"))
    assert "A_TOKEN" in str(err.value)


def test_leak_scan_finds_a_planted_value(tmp_path):
    capsule = make_capsule(tmp_path / "caps")
    (capsule.root / "memory/working_memory.md").write_text("key=hunter2")
    assert leaks(capsule.root, ["hunter2"]) == ["memory/working_memory.md"]


def test_approval_patterns():
    assert needs_approval("rm -rf /tmp/x", ["rm *", "sudo *"])
    assert not needs_approval("ls -la", ["rm *", "sudo *"])


async def test_a_signed_response_is_accepted_once():
    gate = ApprovalGate(KEY, clock=lambda: 100.0)
    request, future = gate.open("a1", "n1", "sudo reboot")
    good = ApprovalResponse(
        request_id=request.request_id, approved=True, signature=sign(request, True, KEY)
    )
    assert gate.resolve(good) is True
    assert await gate.decide(future, 1) is True
    assert gate.resolve(good) is False  # replay


async def test_forged_flipped_and_expired_responses_are_refused():
    now = [100.0]
    gate = ApprovalGate(KEY, clock=lambda: now[0])
    request, future = gate.open("a1", "n1", "sudo reboot", ttl=10)
    forged = ApprovalResponse(
        request_id=request.request_id,
        approved=True,
        signature=sign(request, True, b"x"),
    )
    flipped = ApprovalResponse(  # a "no" re-labelled as a "yes"
        request_id=request.request_id,
        approved=True,
        signature=sign(request, False, KEY),
    )
    assert not gate.resolve(forged) and not gate.resolve(flipped)
    now[0] = 500.0
    late = ApprovalResponse(
        request_id=request.request_id, approved=True, signature=sign(request, True, KEY)
    )
    assert not gate.resolve(late)
    assert await gate.decide(future, 0.05) is False  # silence means no


def test_exported_schemas_match_the_committed_ones():
    root = Path(__file__).resolve().parent.parent / "schema"
    for name, text in schema.render().items():
        assert (root / name).read_text() == text, (
            f"{name} is stale: run `inari-core schema schema`"
        )


def test_cli_verifies_inspects_and_detects_damage(tmp_path):
    capsule = make_capsule(tmp_path / "caps")
    archive = tmp_path / "c.ssc"
    run = lambda *a: subprocess.run(  # noqa: E731
        [sys.executable, "-m", "inari_core.cli", *a], capture_output=True, text=True
    )
    assert run("pack", str(capsule.root), str(archive)).returncode == 0
    assert run("verify", str(archive)).stdout.strip() == "ok"
    info = run("inspect", str(archive)).stdout
    assert "thread-1" in info and "node-a (epoch 1)" in info
    (capsule.root / "memory/working_memory.md").write_text("tampered")
    assert run("verify", str(capsule.root)).returncode == 1


def test_manifest_schema_is_valid_json_with_the_format_pinned():
    text = schema.render()["manifest.schema.json"]
    assert json.loads(text)["properties"]["format"]["const"] == "ssc/1"
