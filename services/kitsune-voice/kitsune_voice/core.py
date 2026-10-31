"""The Inari core the voice agent works for: its Kitsune brain, reached over the owner's API.

The address and the owner's token come from the environment (INARI_URL, INARI_TOKEN or
INARI_TOKEN_FILE); everything else (the LiveKit server, the mode, the model, the voice, the tools
allowed) is read from the core's own Kitsune settings, set in the GUI.
"""

from __future__ import annotations

import os
from typing import Any

import httpx


class CoreError(RuntimeError):
    """The core said no, or did not answer."""


class Core:
    def __init__(self, url: str, token: str, client: httpx.AsyncClient | None = None) -> None:
        self.url = url.rstrip("/")
        self.token = token
        self.client = client or httpx.AsyncClient(timeout=30)

    @classmethod
    def from_env(cls) -> "Core":
        url = os.environ.get("INARI_URL", "http://127.0.0.1:8765")
        token = os.environ.get("INARI_TOKEN", "")
        path = os.environ.get("INARI_TOKEN_FILE", "")
        if not token and path:
            with open(path, encoding="utf-8") as f:
                token = f.read().strip()
        if not token:
            raise CoreError("set INARI_TOKEN or INARI_TOKEN_FILE (the core's owner token)")
        return cls(url, token)

    async def status(self) -> dict[str, Any]:
        r = await self.client.get(f"{self.url}/api/kitsune", params={"token": self.token})
        return self._json(r)

    async def call(self, action: str, **args: Any) -> dict[str, Any]:
        r = await self.client.post(
            f"{self.url}/api/kitsune", params={"token": self.token}, json={"action": action, **args}
        )
        return self._json(r)

    @staticmethod
    def _json(r: httpx.Response) -> dict[str, Any]:
        try:
            body = r.json()
        except ValueError:
            body = {}
        if r.status_code >= 400:
            raise CoreError(body.get("error") or f"HTTP {r.status_code}")
        return body
