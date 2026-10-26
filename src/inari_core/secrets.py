"""BYOK: secrets live in the host's keychain and are injected for one run.

A capsule holds secret NAMES. This module resolves them at the last moment, into
the environment of a child process, and gives tests a way to prove that no value
ever reached a file."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Protocol

from .capsule.models import EnvironmentSpec
from .errors import InariError


class MissingSecret(InariError):
    pass


class SecretResolver(Protocol):
    def get(self, name: str) -> str | None: ...


class EnvResolver:
    """Reads `PREFIX + name` from the process environment (CI, containers)."""

    def __init__(self, prefix: str = "INARI_SECRET_") -> None:
        self.prefix = prefix

    def get(self, name: str) -> str | None:
        return os.environ.get(self.prefix + name)


class KeyringResolver:
    """The operating system keychain, through the optional `keyring` package."""

    def __init__(self, service: str = "inari-core") -> None:
        import keyring  # imported late: the dependency is optional

        self._keyring, self.service = keyring, service

    def get(self, name: str) -> str | None:
        return self._keyring.get_password(self.service, name)


def ephemeral_env(spec: EnvironmentSpec, resolver: SecretResolver) -> dict[str, str]:
    """The environment for one child process. Missing names are reported by name
    only; values never appear in an error."""
    env = dict(spec.env)
    missing: list[str] = []
    for name in spec.secret_refs:
        value = resolver.get(name)
        if value is None:
            missing.append(name)
        else:
            env[name] = value
    if missing:
        raise MissingSecret(f"no value for: {', '.join(sorted(missing))}")
    return env


def scrub(text: str, values: list[str]) -> str:
    for value in sorted((v for v in values if v), key=len, reverse=True):
        text = text.replace(value, "[redacted]")
    return text


def leaks(root: Path, values: list[str]) -> list[str]:
    """Files under `root` that contain any of the given secret values."""
    found: list[str] = []
    needles = [v.encode() for v in values if v]
    for path in sorted(p for p in Path(root).rglob("*") if p.is_file()):
        data = path.read_bytes()
        if any(n in data for n in needles):
            found.append(path.relative_to(root).as_posix())
    return found
