"""Canonical JSON and hashing: one byte sequence per value, on every node."""

from __future__ import annotations

import hashlib
import json
from typing import Any


def canonical(value: Any) -> bytes:
    """Serialize `value` so equal values always yield identical bytes."""
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest_of(value: Any) -> str:
    return sha256_hex(canonical(value))
