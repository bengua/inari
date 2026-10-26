"""Fault injection for tests and drills: wrap an endpoint so one call fails."""

from __future__ import annotations

from typing import Any

from ..errors import TransportError


class Faulty:
    """Delegates to `inner`. The call named `fail_on` raises TransportError, either
    `before` it runs (the request was lost) or `after` it ran (the reply was lost)."""

    def __init__(self, inner: Any, *, fail_on: str = "", when: str = "before") -> None:
        self._inner, self._fail_on, self._when = inner, fail_on, when
        self.node_id = inner.node_id
        self.calls: list[str] = []

    def __getattr__(self, name: str):
        attr = getattr(self._inner, name)
        if not callable(attr):
            return attr

        async def call(*args: Any, **kwargs: Any):
            self.calls.append(name)
            if name == self._fail_on and self._when == "before":
                raise TransportError(f"{name}: request lost")
            result = await attr(*args, **kwargs)
            if name == self._fail_on and self._when == "after":
                raise TransportError(f"{name}: reply lost")
            return result

        return call
