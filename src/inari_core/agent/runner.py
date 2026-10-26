"""The agent loop: model, tools, journal, snapshot. Every step is appended to the
journal as it happens, so a crash or a teleport between steps loses nothing, and a
call cut short is closed by the handover renderer on the next turn."""

from __future__ import annotations

import asyncio
import shutil
import time
import uuid
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Any

from ..approval_bus import NodeApprovals
from ..approvals import needs_approval
from ..capsule import io
from ..capsule.models import EnvironmentSpec, EventKind, Holder
from ..capsule.search import event_text
from ..capsule.timeline import Timeline
from ..handover import compact, render
from ..sandbox import Runtime, materialize
from ..teleport.host import CapsuleHost
from .providers import Provider, ProviderError
from .tools import SPECS, Toolbox, command_text

Publish = Callable[[str, dict[str, Any]], Awaitable[None]]
DEFAULT_SYSTEM = (
    "You are an agent working in a sandboxed workspace. Use the tools to inspect and change "
    "files. Be concise; say when you are done."
)


@dataclass
class RunSummary:
    stopped: str  # done | frozen | max_steps | error
    steps: int = 0
    final_text: str = ""
    error: str = ""


class AgentRunner:
    def __init__(
        self,
        host: CapsuleHost,
        provider: Provider,
        runtime: Runtime,
        *,
        approvals: NodeApprovals | None = None,
        publish: Publish | None = None,
        context_tokens: int = 32000,
        max_steps: int = 20,
        approval_ttl: float = 120.0,
    ) -> None:
        self.host, self.provider, self.runtime = host, provider, runtime
        self.approvals, self.publish = approvals, publish
        self.context_tokens, self.max_steps, self.approval_ttl = (
            context_tokens,
            max_steps,
            approval_ttl,
        )
        self.phases: dict[str, str] = {}
        self._locks: dict[str, asyncio.Lock] = {}

    # -- lifecycle -----------------------------------------------------------
    def create(
        self,
        thread_id: str,
        *,
        system: str = DEFAULT_SYSTEM,
        title: str = "",
        environment: EnvironmentSpec | None = None,
        approval_required: list[str] | None = None,
    ) -> str:
        """Make a new agent held by this node; returns its id (the thread id)."""
        from ..capsule.models import AccessPolicy

        staging = self.host.root / "new" / thread_id
        shutil.rmtree(staging, ignore_errors=True)
        timeline = Timeline()
        timeline.append(self._author(thread_id), EventKind.SYSTEM, {"text": system}, time.time())
        io.create(
            staging,
            thread_id=thread_id,
            environment=environment or EnvironmentSpec(image=""),
            timeline=timeline,
            title=title,
            now=time.time(),
            holder=Holder(node_id=self.host.node_id, epoch=1),
            access=AccessPolicy(approval_required=approval_required or []),
        )
        agent = self.host.adopt(staging)
        shutil.rmtree(staging, ignore_errors=True)
        return agent

    def _author(self, agent: str) -> str:
        return f"{self.host.node_id}@{agent}"

    async def _emit(self, agent: str, kind: str, event_id: str, data: dict[str, Any]) -> None:
        if self.publish:
            await self.publish(
                f"agent.{agent}.event",
                {"agent": agent, "id": event_id, "kind": kind, "text": event_text(data)[:400]},
            )

    async def _phase(self, agent: str, phase: str) -> None:
        self.phases[agent] = phase
        if self.publish:
            await self.publish(
                f"agent.{agent}.state",
                {"agent": agent, "node": self.host.node_id, "phase": phase},
            )

    def _append(self, agent: str, kind: EventKind, data: dict[str, Any]):
        capsule = io.append_event(
            self.host.agents / agent, self._author(agent), kind, data, time.time()
        )
        return capsule.timeline.ordered()[-1]

    # -- one user turn -----------------------------------------------------------
    async def run(self, agent: str, text: str) -> RunSummary:
        lock = self._locks.setdefault(agent, asyncio.Lock())
        async with lock:
            return await self._run(agent, text)

    async def _run(self, agent: str, text: str) -> RunSummary:
        if self.host.status(agent) != "running":
            return RunSummary("frozen", error="this node does not run the agent")
        path = self.host.agents / agent
        await self._phase(agent, "listening")
        event = self._append(agent, EventKind.USER_MESSAGE, {"text": text})
        await self._emit(agent, "user_message", event.id, event.data)
        workdir = self.host.root / "work" / agent
        shutil.rmtree(workdir, ignore_errors=True)
        materialize(io.load(path), workdir)
        summary = RunSummary("max_steps")
        try:
            for step in range(1, self.max_steps + 1):
                summary.steps = step
                if self.host.status(agent) != "running":
                    summary.stopped = "frozen"  # a teleport froze us: stop at the step boundary
                    break
                loaded = io.load(path)
                events = compact(
                    loaded.timeline.ordered(),
                    budget_tokens=self.context_tokens,
                    working_memory=loaded.working_memory,
                )
                await self._phase(agent, "inference")
                try:
                    completion = await self.provider.complete(
                        render(events, self.provider.kind), SPECS
                    )
                except ProviderError as exc:
                    self._append(
                        agent,
                        EventKind.CHECKPOINT,
                        {"reason": "provider_error", "detail": str(exc)[:300]},
                    )
                    summary.stopped, summary.error = "error", str(exc)
                    break
                if completion.text:
                    ev = self._append(agent, EventKind.ASSISTANT_MESSAGE, {"text": completion.text})
                    await self._emit(agent, "assistant_message", ev.id, ev.data)
                    summary.final_text = completion.text
                if not completion.tool_calls:
                    summary.stopped = "done"
                    break
                toolbox = Toolbox(workdir, loaded.environment, self.runtime)
                for call in completion.tool_calls:
                    call_id = call.call_id or uuid.uuid4().hex[:12]
                    ev = self._append(
                        agent,
                        EventKind.TOOL_CALL,
                        {"call_id": call_id, "name": call.name, "arguments": call.arguments},
                    )
                    await self._emit(
                        agent, "tool_call", ev.id, {"name": call.name, "arguments": call.arguments}
                    )
                    output, is_error = await self._execute(agent, loaded, toolbox, call)
                    ev = self._append(
                        agent,
                        EventKind.TOOL_RESULT,
                        {"call_id": call_id, "output": output, "is_error": is_error},
                    )
                    await self._emit(agent, "tool_result", ev.id, {"output": output})
        finally:
            io.update_snapshot(path, workdir)
            shutil.rmtree(workdir, ignore_errors=True)
            await self._phase(agent, "error" if summary.stopped == "error" else "success")
            await asyncio.sleep(0)
            await self._phase(agent, "idle")
        return summary

    async def _execute(self, agent, loaded, toolbox, call) -> tuple[str, bool]:
        patterns = loaded.manifest.access.approval_required
        if patterns and needs_approval(command_text(call.name, call.arguments), patterns):
            if self.approvals is None:
                return "denied: this command needs approval and no approver is connected", True
            ok = await self.approvals.ask(
                agent,
                self.host.node_id,
                command_text(call.name, call.arguments),
                ttl=self.approval_ttl,
            )
            if not ok:
                return "denied: the user did not approve this command", True
        return await toolbox.run(call.name, call.arguments)
