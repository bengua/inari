"""One user turn against a capsule, whoever does the thinking.

`AgentRunner` owns what every turn shares: the journal, the workspace, the phases, the
freeze check, the snapshot. The thinking itself is a *harness*: the built-in `ModelLoop`
(a provider plus our tools) or an external agent such as OpenFox over ACP. Every step is
appended to the journal as it happens, so a crash or a teleport between steps loses
nothing, and a call cut short is closed by the handover renderer on the next turn."""

from __future__ import annotations

import asyncio
import shutil
import time
import uuid
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from ..approval_bus import NodeApprovals
from ..approvals import needs_approval
from ..capsule import io
from ..capsule.models import AccessPolicy, EnvironmentSpec, EventKind, Holder
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


@dataclass
class TurnContext:
    """What a harness needs to run one user turn."""

    agent: str
    text: str
    workdir: Path
    path: Path
    node_id: str
    append: Callable[[EventKind, dict[str, Any]], Any]  # journal an event, returns it
    emit: Callable[[str, str, dict[str, Any]], Awaitable[None]]  # (kind, event id, data)
    phase: Callable[[str], Awaitable[None]]
    running: Callable[[], bool]  # False once a teleport froze the agent
    approve: Callable[[str], Awaitable[bool]]  # ask the phone about a command


class Harness(Protocol):
    async def turn(self, ctx: TurnContext) -> RunSummary: ...


class ModelLoop:
    """The built-in harness: a provider, our tools, a bounded loop."""

    def __init__(
        self,
        provider: Provider,
        runtime: Runtime,
        *,
        context_tokens: int = 32000,
        max_steps: int = 20,
    ) -> None:
        self.provider, self.runtime = provider, runtime
        self.context_tokens, self.max_steps = context_tokens, max_steps

    async def turn(self, ctx: TurnContext) -> RunSummary:
        summary = RunSummary("max_steps")
        for step in range(1, self.max_steps + 1):
            summary.steps = step
            if not ctx.running():
                summary.stopped = "frozen"  # a teleport froze us: stop at the step boundary
                break
            loaded = io.load(ctx.path)
            events = compact(
                loaded.timeline.ordered(),
                budget_tokens=self.context_tokens,
                working_memory=loaded.working_memory,
            )
            await ctx.phase("inference")
            try:
                completion = await self.provider.complete(render(events, self.provider.kind), SPECS)
            except ProviderError as exc:
                ctx.append(
                    EventKind.CHECKPOINT, {"reason": "provider_error", "detail": str(exc)[:300]}
                )
                summary.stopped, summary.error = "error", str(exc)
                break
            if completion.text:
                ev = ctx.append(EventKind.ASSISTANT_MESSAGE, {"text": completion.text})
                await ctx.emit("assistant_message", ev.id, ev.data)
                summary.final_text = completion.text
            if not completion.tool_calls:
                summary.stopped = "done"
                break
            toolbox = Toolbox(ctx.workdir, loaded.environment, self.runtime)
            patterns = loaded.manifest.access.approval_required
            for call in completion.tool_calls:
                call_id = call.call_id or uuid.uuid4().hex[:12]
                data = {"call_id": call_id, "name": call.name, "arguments": call.arguments}
                ev = ctx.append(EventKind.TOOL_CALL, data)
                await ctx.emit("tool_call", ev.id, {"name": call.name, "arguments": call.arguments})
                command = command_text(call.name, call.arguments)
                if (
                    patterns
                    and needs_approval(command, patterns)
                    and not await ctx.approve(command)
                ):
                    output, is_error = "denied: the user did not approve this command", True
                else:
                    output, is_error = await toolbox.run(call.name, call.arguments)
                ev = ctx.append(
                    EventKind.TOOL_RESULT,
                    {"call_id": call_id, "output": output, "is_error": is_error},
                )
                await ctx.emit("tool_result", ev.id, {"output": output})
        return summary


class AgentRunner:
    def __init__(
        self,
        host: CapsuleHost,
        provider: Provider | None,
        runtime: Runtime,
        *,
        harness: Harness | None = None,
        approvals: NodeApprovals | None = None,
        publish: Publish | None = None,
        context_tokens: int = 32000,
        max_steps: int = 20,
        approval_ttl: float = 120.0,
    ) -> None:
        if harness is None:
            if provider is None:
                raise ValueError("a provider or a harness is required")
            harness = ModelLoop(
                provider, runtime, context_tokens=context_tokens, max_steps=max_steps
            )
        self.host, self.harness, self.approvals, self.publish = host, harness, approvals, publish
        self.runtime = runtime
        self.approval_ttl = approval_ttl
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
                f"agent.{agent}.state", {"agent": agent, "node": self.host.node_id, "phase": phase}
            )

    def _append(self, agent: str, kind: EventKind, data: dict[str, Any]):
        capsule = io.append_event(
            self.host.agents / agent, self._author(agent), kind, data, time.time()
        )
        return capsule.timeline.ordered()[-1]

    # -- a command typed by the person, run in the agent's sandbox ---------------------------
    async def exec_user(self, agent: str, cmd: str) -> dict[str, Any]:
        """Run `cmd` in the agent's workspace, journal it as a tool call so the agent knows
        what was done, and keep the resulting files. Waits for any turn in progress."""
        lock = self._locks.setdefault(agent, asyncio.Lock())
        async with lock:
            if self.host.status(agent) != "running":
                return {"ok": False, "output": "this node does not run the agent", "code": -1}
            path, workdir = self.host.agents / agent, self.host.root / "work" / agent
            shutil.rmtree(workdir, ignore_errors=True)
            capsule = io.load(path)
            materialize(capsule, workdir)
            call_id = uuid.uuid4().hex[:12]
            ev = self._append(
                agent,
                EventKind.TOOL_CALL,
                {"call_id": call_id, "name": "bash", "arguments": {"cmd": cmd, "by": "user"}},
            )
            await self._emit(agent, "tool_call", ev.id, {"name": "bash", "arguments": {"cmd": cmd}})
            result = await self.runtime.run(
                capsule.environment, workdir, ["/bin/sh", "-c", cmd], timeout=60
            )
            output = (result.stdout + result.stderr)[:20000] or "(no output)"
            ev = self._append(
                agent,
                EventKind.TOOL_RESULT,
                {"call_id": call_id, "output": output, "is_error": result.code != 0},
            )
            await self._emit(agent, "tool_result", ev.id, {"output": output})
            io.update_snapshot(path, workdir)
            shutil.rmtree(workdir, ignore_errors=True)
            return {
                "ok": result.code == 0,
                "output": output,
                "code": result.code,
                "timed_out": result.timed_out,
            }

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

        async def approve(command: str) -> bool:
            if self.approvals is None:
                return False
            return await self.approvals.ask(
                agent, self.host.node_id, command, ttl=self.approval_ttl
            )

        ctx = TurnContext(
            agent=agent,
            text=text,
            workdir=workdir,
            path=path,
            node_id=self.host.node_id,
            append=lambda kind, data: self._append(agent, kind, data),
            emit=lambda kind, event_id, data: self._emit(agent, kind, event_id, data),
            phase=lambda phase: self._phase(agent, phase),
            running=lambda: self.host.status(agent) == "running",
            approve=approve,
        )
        summary = RunSummary("error", error="harness did not finish")
        try:
            summary = await self.harness.turn(ctx)
        except Exception as exc:  # noqa: BLE001 - a harness bug must not wedge the agent
            self._append(
                agent, EventKind.CHECKPOINT, {"reason": "harness_error", "detail": str(exc)[:300]}
            )
            summary = RunSummary("error", error=f"{type(exc).__name__}: {exc}")
        finally:
            io.update_snapshot(path, workdir)
            shutil.rmtree(workdir, ignore_errors=True)
            await self._phase(agent, "error" if summary.stopped == "error" else "success")
            await asyncio.sleep(0)
            await self._phase(agent, "idle")
        return summary
