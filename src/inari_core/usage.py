"""Quotas and usage of the model providers, in one normalised shape.

Sources are adapters; the gateway polls them and puts the result in its snapshot, so the
interface can show "41 % of the five-hour window, resets at 21:40" next to the model that is
in use. A local model has no quota: it is listed as such."""

from __future__ import annotations

import asyncio
import json
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass, field
from typing import Any, Protocol


@dataclass
class UsageWindow:
    label: str
    used_percent: float
    resets_at: str = ""


@dataclass
class ProviderUsage:
    provider_id: str  # matches a model family: "claude", "codex", "local"...
    name: str
    status: str = "ok"  # ok | unauthenticated | error | unlimited
    plan: str = ""
    tint: str = ""
    logo: str = ""  # a path the gateway serves (see Gateway.logos), or empty
    windows: list[UsageWindow] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class UsageSource(Protocol):
    async def fetch(self) -> list[ProviderUsage]: ...


def _http_json(url: str, body: dict | None, timeout: float) -> Any:
    request = urllib.request.Request(
        url,
        json.dumps(body).encode() if body is not None else None,
        {"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310
        return json.loads(response.read())


class StaticUsageSource:
    """Providers described in configuration: a local model (unlimited) or a manual quota."""

    def __init__(self, items: list[dict[str, Any]]) -> None:
        self.items = items

    async def fetch(self) -> list[ProviderUsage]:
        return [
            ProviderUsage(
                provider_id=i["provider_id"],
                name=i.get("name", i["provider_id"]),
                status=i.get("status", "unlimited"),
                plan=i.get("plan", ""),
                tint=i.get("tint", ""),
                windows=[UsageWindow(**w) for w in i.get("windows", [])],
            )
            for i in self.items
        ]


class CloudroomUsageSource:
    """Reads the provider-usage service of an existing Cloudroom-compatible server."""

    FAMILY = {"claude-code": "claude", "codex": "codex", "kimi": "kimi"}

    def __init__(self, base_url: str, providers: list[str], timeout: float = 40.0) -> None:
        self.base, self.providers, self.timeout = base_url.rstrip("/"), providers, timeout
        self.logos: dict[str, str] = {}  # provider id -> absolute logo URL

    async def fetch(self) -> list[ProviderUsage]:
        hosts = await asyncio.to_thread(_http_json, self.base + "/api/v1/hosts", None, self.timeout)
        ids = [h["id"] for h in hosts if h.get("status") == "connected"]
        found: dict[str, ProviderUsage] = {}
        for provider in self.providers:
            body = {"force": False, "machineIds": ids, "providerId": provider, "maxAgeMs": 600000}
            url = self.base + "/api/v1/plugins/provider-usage/rpc/getUsage"
            data = await asyncio.to_thread(_http_json, url, body, self.timeout)
            for machine in (data.get("result") or {}).get("machines", []):
                for entry in machine.get("providers", []):
                    if entry["providerId"] != provider or entry["providerId"] in found:
                        continue
                    usage = entry.get("usage") or {}
                    family = self.FAMILY.get(provider, provider)
                    if entry.get("logoUrl"):
                        self.logos[family] = self.base + entry["logoUrl"]
                    tint = (entry.get("strings", {}).get("iconTint") or {}).get("dark", "")
                    found[provider] = ProviderUsage(
                        provider_id=family,
                        name=entry.get("displayName", provider),
                        status=usage.get("status", "error"),
                        plan=usage.get("planLabel") or "",
                        tint=tint or "",
                        logo=f"/api/logo/{family}" if family in self.logos else "",
                        windows=[
                            UsageWindow(
                                w["label"], float(w["usedPercent"]), w.get("resetsAt") or ""
                            )
                            for w in usage.get("windows", [])
                            if w.get("usedPercent") is not None
                        ],
                    )
        return list(found.values())


async def poll(sources: list[UsageSource], interval: float, publish) -> None:
    """Fetch every source forever; one failing source never hides the others."""
    while True:
        items: list[ProviderUsage] = []
        for source in sources:
            try:
                items += await source.fetch()
            except (OSError, urllib.error.URLError, ValueError, KeyError, TimeoutError):
                continue
        await publish([i.to_dict() for i in items])
        await asyncio.sleep(interval)
