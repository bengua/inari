from __future__ import annotations

import sys
from pathlib import Path

import pytest

from inari_core.capsule import (
    EnvironmentSpec,
    Mount,
    diff,
    load,
    update_snapshot,
    verify,
)
from inari_core.sandbox import ContainerRuntime, SubprocessRuntime, materialize
from inari_core.secrets import EnvResolver
from tests.conftest import make_capsule

PY = sys.executable


async def test_a_run_changes_the_workspace_and_the_capsule_records_only_the_delta(
    tmp_path,
):
    capsule = make_capsule(tmp_path / "caps")
    work = materialize(capsule, tmp_path / "work")
    assert (work / "a.txt").read_text() == "alpha"
    before = capsule.snapshot
    code = "open('out.txt','w').write('built'); print('done')"
    result = await SubprocessRuntime(EnvResolver()).run(
        capsule.environment, work, [PY, "-c", code]
    )
    assert result.code == 0 and result.stdout.strip() == "done"
    updated = update_snapshot(capsule.root, work)
    assert diff(before, updated.snapshot) == {
        "added": ["out.txt"], "deleted": [], "modified": [],
    }  # fmt: skip
    assert verify(capsule.root) == []
    assert load(capsule.root).snapshot.parent == before.digest()


async def test_secrets_reach_the_process_but_not_its_output_or_the_host_env(
    tmp_path, monkeypatch
):
    monkeypatch.setenv("INARI_SECRET_API_KEY", "s3cr3t-value")
    monkeypatch.setenv("HOST_ONLY", "leak-me")
    spec = EnvironmentSpec(secret_refs=["API_KEY"], env={"MODE": "test"})
    code = (
        "import os; print(os.environ['API_KEY'], "
        "os.environ.get('HOST_ONLY'), os.environ['MODE'])"
    )
    result = await SubprocessRuntime(EnvResolver()).run(
        spec, tmp_path, [PY, "-c", code]
    )
    assert result.stdout.strip() == "[redacted] None test"


async def test_a_runaway_process_is_killed_with_its_children(tmp_path):
    code = "import subprocess,time; subprocess.Popen(['sleep','60']); time.sleep(60)"
    result = await SubprocessRuntime(EnvResolver()).run(
        EnvironmentSpec(), tmp_path, [PY, "-c", code], timeout=0.5
    )
    assert result.timed_out


async def test_output_is_capped(tmp_path):
    runtime = SubprocessRuntime(EnvResolver(), max_output=100)
    result = await runtime.run(
        EnvironmentSpec(), tmp_path, [PY, "-c", "print('x'*10000)"]
    )
    assert result.truncated and len(result.stdout) == 100


def test_container_command_is_locked_down_and_keeps_secret_values_out_of_argv(tmp_path):
    spec = EnvironmentSpec(
        image="base:1", workdir="/work", env={"MODE": "t"}, secret_refs=["API_KEY"],
        mounts=[Mount(source="models", target="/models", mode="ro")],
        resources={"memory": "512m"},
    )  # fmt: skip
    rt = ContainerRuntime(EnvResolver(), volumes={"models": Path("/srv/models")})
    cmd = rt.command(spec, tmp_path, ["make", "test"])
    assert "--network=none" in cmd and "--cap-drop=ALL" in cmd
    assert "--memory=512m" in cmd and f"{tmp_path}:/work:rw" in cmd
    assert "/srv/models:/models:ro" in cmd
    assert cmd[-3:] == ["base:1", "make", "test"]
    assert "API_KEY" in cmd and not any(
        "=" in c and c.startswith("API_KEY") for c in cmd
    )


def test_an_unconfigured_volume_is_refused(tmp_path):
    spec = EnvironmentSpec(image="i", mounts=[Mount(source="nope", target="/n")])
    with pytest.raises(KeyError):
        ContainerRuntime(EnvResolver()).command(spec, tmp_path, ["x"])
