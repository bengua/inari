"""Model providers. Each takes the rendered conversation and returns text and/or
tool calls. Endpoint, model and key name come from configuration; nothing is built in."""

from __future__ import annotations

import asyncio
import json
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from typing import Any, Protocol

from ..errors import InariError
from ..secrets import SecretResolver


class ProviderError(InariError):
    pass


@dataclass
class ToolSpec:
    name: str
    description: str
    schema: dict[str, Any]


@dataclass
class ToolCall:
    call_id: str
    name: str
    arguments: dict[str, Any]


@dataclass
class Completion:
    text: str = ""
    tool_calls: list[ToolCall] = field(default_factory=list)


@dataclass
class ProviderConfig:
    kind: str = "openai"  # "openai" (any compatible gateway) or "anthropic"
    base_url: str = ""
    model: str = ""
    api_key_ref: str = ""  # NAME of the secret, resolved at call time
    max_tokens: int = 4096
    timeout: float = 120.0
    context_tokens: int = 32000


class Provider(Protocol):
    kind: str

    async def complete(self, rendered: dict[str, Any], tools: list[ToolSpec]) -> Completion: ...


def _post(url: str, headers: dict[str, str], body: dict[str, Any], timeout: float) -> dict:
    request = urllib.request.Request(
        url, json.dumps(body).encode(), {"Content-Type": "application/json", **headers}
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:  # noqa: S310
            return json.loads(response.read())
    except urllib.error.HTTPError as exc:
        detail = exc.read()[:300].decode(errors="replace")
        raise ProviderError(f"HTTP {exc.code}: {detail}") from exc
    except (urllib.error.URLError, TimeoutError, ValueError) as exc:
        raise ProviderError(f"provider unreachable: {exc}") from exc


class OpenAIProvider:
    kind = "openai"

    def __init__(self, cfg: ProviderConfig, resolver: SecretResolver) -> None:
        self.cfg, self.resolver = cfg, resolver

    async def complete(self, rendered, tools) -> Completion:
        messages = rendered["messages"]  # the journal's system text is its first message
        body: dict[str, Any] = {
            "model": self.cfg.model,
            "messages": messages,
            "max_tokens": self.cfg.max_tokens,
        }
        if tools:
            body["tools"] = [
                {
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.schema,
                    },
                }
                for t in tools
            ]
        headers = {}
        if self.cfg.api_key_ref and (key := self.resolver.get(self.cfg.api_key_ref)):
            headers["Authorization"] = f"Bearer {key}"
        url = self.cfg.base_url.rstrip("/") + "/chat/completions"
        data = await asyncio.to_thread(_post, url, headers, body, self.cfg.timeout)
        try:
            message = data["choices"][0]["message"]
            calls = []
            for call in message.get("tool_calls") or []:
                raw = call["function"].get("arguments") or "{}"
                calls.append(
                    ToolCall(
                        call.get("id") or uuid.uuid4().hex[:12],
                        call["function"]["name"],
                        json.loads(raw),
                    )
                )
            return Completion(message.get("content") or "", calls)
        except (KeyError, IndexError, ValueError) as exc:
            raise ProviderError(f"unexpected response shape: {exc}") from exc


class AnthropicProvider:
    kind = "anthropic"

    def __init__(self, cfg: ProviderConfig, resolver: SecretResolver) -> None:
        self.cfg, self.resolver = cfg, resolver

    async def complete(self, rendered, tools) -> Completion:
        body: dict[str, Any] = {
            "model": self.cfg.model,
            "max_tokens": self.cfg.max_tokens,
            "system": rendered.get("system", ""),
            "messages": rendered["messages"],
        }
        if tools:
            body["tools"] = [
                {"name": t.name, "description": t.description, "input_schema": t.schema}
                for t in tools
            ]
        headers = {"anthropic-version": "2023-06-01"}
        if self.cfg.api_key_ref and (key := self.resolver.get(self.cfg.api_key_ref)):
            headers["x-api-key"] = key
        url = self.cfg.base_url.rstrip("/") + "/v1/messages"
        data = await asyncio.to_thread(_post, url, headers, body, self.cfg.timeout)
        try:
            text, calls = [], []
            for block in data["content"]:
                if block["type"] == "text":
                    text.append(block["text"])
                elif block["type"] == "tool_use":
                    calls.append(ToolCall(block["id"], block["name"], block["input"]))
            return Completion("\n".join(text), calls)
        except (KeyError, TypeError) as exc:
            raise ProviderError(f"unexpected response shape: {exc}") from exc


def make_provider(cfg: ProviderConfig, resolver: SecretResolver) -> Provider:
    if cfg.kind == "openai":
        return OpenAIProvider(cfg, resolver)
    if cfg.kind == "anthropic":
        return AnthropicProvider(cfg, resolver)
    raise ValueError(f"unknown provider kind {cfg.kind!r}")
