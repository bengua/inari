"""A compact local vector index. The default embedder is a deterministic hashing
trick (no model, no network); swap in a real one through the `Embedder` protocol."""

from __future__ import annotations

import hashlib
import json
import math
import re
from typing import Protocol

WORD = re.compile(r"[a-z0-9_]+")


class Embedder(Protocol):
    name: str
    dim: int

    def embed(self, text: str) -> list[float]: ...


class HashingEmbedder:
    name = "hashing-v1"

    def __init__(self, dim: int = 64) -> None:
        self.dim = dim

    def embed(self, text: str) -> list[float]:
        vec = [0.0] * self.dim
        for word in WORD.findall(text.lower()):
            h = hashlib.blake2b(word.encode(), digest_size=8).digest()
            slot = int.from_bytes(h[:4], "big") % self.dim
            vec[slot] += 1.0 if h[4] & 1 else -1.0
        norm = math.sqrt(sum(x * x for x in vec)) or 1.0
        return [round(x / norm, 4) for x in vec]


class VectorIndex:
    def __init__(self, embedder: Embedder | None = None) -> None:
        self.embedder: Embedder = embedder or HashingEmbedder()
        self.items: dict[str, tuple[str, list[float]]] = {}

    def add(self, item_id: str, text: str) -> None:
        self.items[item_id] = (text, self.embedder.embed(text))

    def search(self, query: str, k: int = 3) -> list[tuple[str, float]]:
        q = self.embedder.embed(query)
        scored = [
            (item_id, sum(a * b for a, b in zip(q, vec, strict=True)))
            for item_id, (_, vec) in self.items.items()
        ]
        scored.sort(key=lambda pair: (-pair[1], pair[0]))
        return scored[:k]

    def dumps(self) -> str:
        return json.dumps(
            {
                "embedder": self.embedder.name,
                "dim": self.embedder.dim,
                "items": {i: {"text": t, "vec": v} for i, (t, v) in self.items.items()},
            },
            sort_keys=True,
            separators=(",", ":"),
        )

    @classmethod
    def loads(cls, text: str) -> VectorIndex:
        raw = json.loads(text)
        index = cls(HashingEmbedder(raw["dim"]))
        index.items = {
            i: (entry["text"], entry["vec"]) for i, entry in raw["items"].items()
        }
        return index
