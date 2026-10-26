"""Approvals over the bus. The node asks; the phone, whenever it is reachable
within the request's lifetime, answers with a signed response. `approval.>` is a
durable subject, so a phone that was offline still receives the question."""

from __future__ import annotations

import inspect
from collections.abc import Awaitable, Callable

from .approvals import ApprovalGate, ApprovalRequest, ApprovalResponse, sign
from .mesh.bus import Bus, Message


class NodeApprovals:
    def __init__(self, bus: Bus, gate: ApprovalGate) -> None:
        self.bus, self.gate = bus, gate
        bus.subscribe("approval.response", self._on_response)

    def _on_response(self, message: Message) -> None:
        self.gate.resolve(ApprovalResponse.model_validate(message.payload))

    async def ask(self, agent_id: str, node_id: str, command: str, ttl: float = 60.0) -> bool:
        request, future = self.gate.open(agent_id, node_id, command, ttl)
        await self.bus.publish("approval.request", request.model_dump(mode="json"))
        return await self.gate.decide(future, ttl)


class PhoneApprover:
    """`decide` stands in for the UI plus the biometric prompt that unlocks the key."""

    def __init__(
        self,
        bus: Bus,
        key: bytes,
        decide: Callable[[ApprovalRequest], Awaitable[bool] | bool],
    ) -> None:
        self.bus, self._key, self._decide = bus, key, decide
        bus.subscribe("approval.request", self._on_request)

    async def _on_request(self, message: Message) -> None:
        request = ApprovalRequest.model_validate(message.payload)
        verdict = self._decide(request)
        approved = await verdict if inspect.isawaitable(verdict) else verdict
        response = ApprovalResponse(
            request_id=request.request_id,
            approved=bool(approved),
            signature=sign(request, bool(approved), self._key),
        )
        await self.bus.publish("approval.response", response.model_dump(mode="json"))
