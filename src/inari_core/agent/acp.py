"""An external agent as a harness, over the Agent Client Protocol (JSON-RPC, one message
per line on the agent's stdin/stdout). OpenFox, through `openfox-acp`, is the first user;
anything that speaks ACP works the same.

The journal stays the source of truth. The harness maps what the agent does onto journal
events (messages, tool calls, results), routes its permission requests to the phone, and,
when the agent has no memory of this thread (a new machine after a teleport), opens a
session primed with the journal so far."""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
from pathlib import Path
from typing import Any

from ..capsule import io
from ..capsule.models import EventKind
from ..capsule.search import event_text
from ..handover import compact
from .runner import RunSummary, TurnContext

ENV_KEEP = ("HOME", "PATH", "USER", "LANG", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR")


def _text(content: Any) -> str:
    """Flatten an ACP content block (or list of them) to text."""
    if isinstance(content, dict):
        if content.get("type") == "content":
            return _text(content.get("content"))
        return str(content.get("text") or "")
    if isinstance(content, list):
        return "\n".join(t for t in (_text(c) for c in content) if t)
    return str(content or "")


class AcpHarness:
    def __init__(
        self,
        command: list[str],
        sessions_file: Path,
        *,
        timeout: float = 600.0,
        context_tokens: int = 6000,
        extra_env: dict[str, str] | None = None,
    ) -> None:
        self.command, self.sessions_file, self.timeout = command, Path(sessions_file), timeout
        self.context_tokens = context_tokens
        self.extra_env = extra_env or {}
        self._models: dict[str, str] = {}

    # -- session memory kept per node ------------------------------------------
    def _sessions(self) -> dict[str, str]:
        try:
            return json.loads(self.sessions_file.read_text())
        except (OSError, ValueError):
            return {}

    def _remember(self, agent: str, session_id: str) -> None:
        data = self._sessions() | {agent: session_id}
        self.sessions_file.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.sessions_file.with_suffix(".part")
        tmp.write_text(json.dumps(data))
        os.replace(tmp, self.sessions_file)

    # -- JSON-RPC plumbing -------------------------------------------------------
    async def _spawn(self, cwd: Path) -> asyncio.subprocess.Process:
        env = {k: os.environ[k] for k in ENV_KEEP if k in os.environ} | self.extra_env
        return await asyncio.create_subprocess_exec(
            *self.command, cwd=cwd, env=env,
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )  # fmt: skip

    async def turn(self, ctx: TurnContext) -> RunSummary:
        proc = await self._spawn(ctx.workdir)
        try:
            return await asyncio.wait_for(self._conversation(proc, ctx), self.timeout)
        except TimeoutError:
            ctx.append(EventKind.CHECKPOINT, {"reason": "harness_timeout"})
            return RunSummary("error", error=f"the agent did not finish within {self.timeout:.0f}s")
        finally:
            if proc.returncode is None:
                with contextlib.suppress(ProcessLookupError):
                    proc.terminate()
                with contextlib.suppress(Exception):
                    await asyncio.wait_for(proc.wait(), 5)
                if proc.returncode is None:
                    with contextlib.suppress(ProcessLookupError):
                        proc.kill()

    async def _conversation(self, proc, ctx: TurnContext) -> RunSummary:
        pending: dict[int, asyncio.Future] = {}
        next_id = 0
        text_parts: list[str] = []
        calls_open: dict[str, bool] = {}
        summary = RunSummary("done")

        async def send(obj: dict[str, Any]) -> None:
            proc.stdin.write((json.dumps(obj) + "\n").encode())
            await proc.stdin.drain()

        async def request(method: str, params: dict[str, Any]) -> Any:
            nonlocal next_id
            next_id += 1
            future = asyncio.get_running_loop().create_future()
            pending[next_id] = future
            await send({"jsonrpc": "2.0", "id": next_id, "method": method, "params": params})
            return await future

        async def flush_text() -> None:
            if text_parts:
                body = "".join(text_parts).strip()
                text_parts.clear()
                if body:
                    ev = ctx.append(EventKind.ASSISTANT_MESSAGE, {"text": body})
                    await ctx.emit("assistant_message", ev.id, ev.data)
                    summary.final_text = body

        async def on_update(update: dict[str, Any]) -> None:
            kind = update.get("sessionUpdate")
            if kind == "agent_message_chunk":
                text_parts.append(_text(update.get("content")))
                await ctx.phase("inference")
            elif kind == "tool_call":
                await flush_text()
                call_id = str(update.get("toolCallId"))
                data = {
                    "call_id": call_id,
                    "name": str(update.get("title") or update.get("kind") or "tool"),
                    "arguments": {"kind": update.get("kind"), "input": update.get("rawInput")},
                }
                ev = ctx.append(EventKind.TOOL_CALL, data)
                calls_open[call_id] = True
                await ctx.emit("tool_call", ev.id, {"name": data["name"]})
            elif kind == "tool_call_update" and update.get("status") in ("completed", "failed"):
                call_id = str(update.get("toolCallId"))
                if calls_open.pop(call_id, None):
                    output = _text(update.get("content")) or str(update.get("rawOutput") or "")
                    failed = update.get("status") == "failed"
                    ev = ctx.append(
                        EventKind.TOOL_RESULT,
                        {"call_id": call_id, "output": output[:20000], "is_error": failed},
                    )
                    await ctx.emit("tool_result", ev.id, {"output": output})
            # agent_thought_chunk and plan are the agent's private reasoning: not journaled.

        async def on_request(message: dict[str, Any]) -> None:
            params, ident = message.get("params") or {}, message["id"]
            if message["method"] == "session/request_permission":
                title = str((params.get("toolCall") or {}).get("title") or "an action")
                allowed = await ctx.approve(title)  # the phone decides; no approver means no
                options = {o.get("kind"): o.get("optionId") for o in params.get("options") or []}
                if allowed:
                    chosen = options.get("allow_once") or next(iter(options.values()), "allow")
                    outcome: dict[str, Any] = {"outcome": "selected", "optionId": chosen}
                else:
                    deny = options.get("reject_once")
                    outcome = (
                        {"outcome": "selected", "optionId": deny}
                        if deny
                        else {"outcome": "cancelled"}
                    )
                await send({"jsonrpc": "2.0", "id": ident, "result": {"outcome": outcome}})
            else:
                error = {"code": -32601, "message": f"unsupported: {message['method']}"}
                await send({"jsonrpc": "2.0", "id": ident, "error": error})

        async def reader() -> None:
            while True:
                line = await proc.stdout.readline()
                if not line:
                    for future in pending.values():
                        if not future.done():
                            future.set_exception(RuntimeError("the agent process ended"))
                    return
                try:
                    message = json.loads(line)
                except ValueError:
                    continue
                if "method" in message and "id" in message:
                    asyncio.create_task(on_request(message))
                elif "method" in message:
                    if message["method"] == "session/update":
                        await on_update((message.get("params") or {}).get("update") or {})
                elif message.get("id") in pending:
                    future = pending.pop(message["id"])
                    if "error" in message:
                        future.set_exception(
                            RuntimeError(str((message["error"] or {}).get("message")))
                        )
                    else:
                        future.set_result(message.get("result") or {})

        read_task = asyncio.create_task(reader())
        try:
            await request("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
            session_id, primed = await self._session(request, ctx)
            if ctx.model and ctx.model != self._models.get(session_id):
                try:
                    await request(
                        "session/set_model", {"sessionId": session_id, "modelId": ctx.model}
                    )
                    self._models[session_id] = ctx.model
                except RuntimeError as exc:  # unknown model: keep the agent's own, and say so
                    ctx.append(
                        EventKind.CHECKPOINT, {"reason": "model_refused", "detail": str(exc)[:200]}
                    )
            prompt = ctx.text if not primed else f"{primed}\n\n{ctx.text}"
            await ctx.phase("inference")
            prompt_future = asyncio.create_task(
                request(
                    "session/prompt",
                    {"sessionId": session_id, "prompt": [{"type": "text", "text": prompt}]},
                )
            )
            while not prompt_future.done():
                await asyncio.wait({prompt_future}, timeout=0.5)
                if not ctx.running() and not prompt_future.done():
                    await send(
                        {
                            "jsonrpc": "2.0",
                            "method": "session/cancel",
                            "params": {"sessionId": session_id},
                        }
                    )
                    summary.stopped = "frozen"
                    await asyncio.wait({prompt_future}, timeout=10)
                    break
            if prompt_future.done() and not prompt_future.cancelled():
                result = prompt_future.result()
                if summary.stopped != "frozen":
                    stop = str(result.get("stopReason") or "end_turn")
                    summary.stopped = "done" if stop in ("end_turn", "max_tokens") else stop
            return summary
        except RuntimeError as exc:
            ctx.append(EventKind.CHECKPOINT, {"reason": "harness_error", "detail": str(exc)[:300]})
            summary.stopped, summary.error = "error", str(exc)
            return summary
        finally:
            await flush_text()
            read_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await read_task

    async def _session(self, request, ctx: TurnContext) -> tuple[str, str]:
        """(session id, priming text). Reuse this node's session for the thread when the
        agent still has it; otherwise open a new one and give it the journal so far."""
        known = self._sessions().get(ctx.agent)
        if known:
            try:
                await request(
                    "session/load", {"sessionId": known, "cwd": str(ctx.workdir), "mcpServers": []}
                )
                return known, ""
            except RuntimeError:
                pass  # the agent forgot it: fall through to a fresh, primed session
        created = await request("session/new", {"cwd": str(ctx.workdir), "mcpServers": []})
        session_id = str(created["sessionId"])
        self._remember(ctx.agent, session_id)
        capsule = io.load(ctx.path)
        events = [e for e in capsule.timeline.ordered() if e.kind is not EventKind.CHECKPOINT]
        # the turn's own user message is already journaled last: it is sent as the prompt
        history = events[:-1] if events and events[-1].kind is EventKind.USER_MESSAGE else events
        if not [e for e in history if e.kind is not EventKind.SYSTEM]:
            return session_id, ""
        kept = compact(
            history, budget_tokens=self.context_tokens, working_memory=capsule.working_memory
        )
        lines = [f"[{e.kind.value}] {event_text(e.data)[:500]}" for e in kept if event_text(e.data)]
        return session_id, (
            "You are continuing a conversation that started elsewhere (another machine or model). "
            "Earlier journal, oldest first:\n" + "\n".join(lines) + "\n\nNew message from the user:"
        )

    async def list_models(self, cwd: Path, timeout: float = 20.0) -> list[str]:
        """The model ids the agent offers, read from a throw-away session."""
        proc = await self._spawn(cwd)
        try:

            async def ask(ident: int, method: str, params: dict) -> dict:
                proc.stdin.write(
                    (
                        json.dumps(
                            {"jsonrpc": "2.0", "id": ident, "method": method, "params": params}
                        )
                        + "\n"
                    ).encode()
                )
                await proc.stdin.drain()
                while True:
                    line = await proc.stdout.readline()
                    if not line:
                        return {}
                    try:
                        message = json.loads(line)
                    except ValueError:
                        continue  # some agents print a banner on stdout first
                    if message.get("id") == ident:
                        return message.get("result") or {}

            async def probe() -> list[str]:
                await ask(1, "initialize", {"protocolVersion": 1, "clientCapabilities": {}})
                made = await ask(2, "session/new", {"cwd": str(cwd), "mcpServers": []})
                return [m["modelId"] for m in (made.get("models") or {}).get("availableModels", [])]

            return await asyncio.wait_for(probe(), timeout)
        except (TimeoutError, OSError, ValueError, KeyError):
            return []
        finally:
            with contextlib.suppress(ProcessLookupError):
                proc.kill()
