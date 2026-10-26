"""Run an agent's commands inside the sandbox its capsule describes.

Two runtimes behind one protocol. `SubprocessRuntime` is a plain child process
with a scrubbed environment (development, CI, trusted code). `ContainerRuntime`
builds a Podman/Docker command: no network by default, the workspace as the only
writable mount, secrets passed by NAME so values never appear in argv."""

from __future__ import annotations

import asyncio
import os
import shutil
import signal
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from .capsule.blobs import restore
from .capsule.io import Capsule
from .capsule.models import EnvironmentSpec
from .secrets import SecretResolver, ephemeral_env, scrub

SAFE_PATH = "/usr/local/bin:/usr/bin:/bin"


@dataclass
class RunResult:
    code: int
    stdout: str
    stderr: str
    timed_out: bool = False
    truncated: bool = False


class Runtime(Protocol):
    async def run(
        self,
        spec: EnvironmentSpec,
        workdir: Path,
        argv: list[str],
        *,
        timeout: float = 60,
    ) -> RunResult: ...


def materialize(capsule: Capsule, dest: Path) -> Path:
    """Recreate the capsule's workspace in `dest`."""
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    restore(capsule.snapshot, capsule.blobs(), dest)
    return dest


async def _exec(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    secrets: list[str],
    timeout: float,
    max_output: int,
) -> RunResult:
    proc = await asyncio.create_subprocess_exec(
        *argv, cwd=cwd, env=env, stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        start_new_session=True,
    )  # fmt: skip
    timed_out = False
    try:
        out, err = await asyncio.wait_for(proc.communicate(), timeout)
    except TimeoutError:
        timed_out = True
        os.killpg(proc.pid, signal.SIGKILL)  # the whole group: no orphaned children
        out, err = await proc.communicate()
    truncated = len(out) > max_output or len(err) > max_output

    def text(raw: bytes) -> str:
        return scrub(raw[:max_output].decode(errors="replace"), secrets)

    return RunResult(proc.returncode or 0, text(out), text(err), timed_out, truncated)


class SubprocessRuntime:
    def __init__(self, resolver: SecretResolver, max_output: int = 1_000_000) -> None:
        self.resolver, self.max_output = resolver, max_output

    async def run(
        self,
        spec: EnvironmentSpec,
        workdir: Path,
        argv: list[str],
        *,
        timeout: float = 60,
    ) -> RunResult:
        env = {
            "PATH": SAFE_PATH,
            **ephemeral_env(spec, self.resolver),
        }  # nothing inherited
        values = [env[n] for n in spec.secret_refs]
        return await _exec(
            argv, cwd=workdir, env=env, secrets=values, timeout=timeout,
            max_output=self.max_output,
        )  # fmt: skip


class ContainerRuntime:
    def __init__(
        self,
        resolver: SecretResolver,
        *,
        engine: str = "podman",
        volumes: dict[str, Path] | None = None,
        max_output: int = 1_000_000,
    ) -> None:
        self.resolver, self.engine = resolver, engine
        self.volumes = volumes or {}  # logical volume name -> host directory
        self.max_output = max_output

    def command(
        self, spec: EnvironmentSpec, workdir: Path, argv: list[str]
    ) -> list[str]:
        cmd = [
            self.engine, "run", "--rm", "--interactive=false", "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            f"--network={spec.resources.get('network', 'none')}",
            "-w", spec.workdir, "-v", f"{workdir}:{spec.workdir}:rw",
        ]  # fmt: skip
        if "memory" in spec.resources:
            cmd.append(f"--memory={spec.resources['memory']}")
        for mount in spec.mounts:
            if mount.source not in self.volumes:
                raise KeyError(
                    f"volume {mount.source!r} is not configured on this node"
                )
            cmd += ["-v", f"{self.volumes[mount.source]}:{mount.target}:{mount.mode}"]
        for key, value in sorted(spec.env.items()):
            cmd += ["-e", f"{key}={value}"]
        for name in sorted(spec.secret_refs):
            cmd += ["-e", name]  # the value comes from the child's environment
        return [*cmd, spec.image, *argv]

    async def run(
        self,
        spec: EnvironmentSpec,
        workdir: Path,
        argv: list[str],
        *,
        timeout: float = 60,
    ) -> RunResult:
        if shutil.which(self.engine) is None:
            raise FileNotFoundError(f"{self.engine} is not installed on this node")
        secret_env = ephemeral_env(
            EnvironmentSpec(secret_refs=spec.secret_refs), self.resolver
        )
        env = {"PATH": SAFE_PATH, **secret_env}
        return await _exec(
            self.command(spec, workdir, argv), cwd=workdir, env=env,
            secrets=list(secret_env.values()), timeout=timeout,
            max_output=self.max_output,
        )  # fmt: skip
