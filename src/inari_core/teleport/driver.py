"""Run a teleportation between two endpoints, and settle it if it was cut short."""

from __future__ import annotations

import uuid
from collections.abc import Awaitable, Callable

from ..errors import TransportError
from .machine import Machine, State
from .protocol import BLOB_PREFIX, Endpoint, Outcome


async def teleport(
    source: Endpoint,
    target: Endpoint,
    agent_id: str,
    *,
    session_id: str | None = None,
    on_state: Callable[[str], Awaitable[None]] | None = None,
) -> Outcome:
    sid = session_id or uuid.uuid4().hex
    machine = Machine()

    async def step(event: str) -> None:
        machine.fire(event)
        if on_state is not None:
            try:
                await on_state(machine.state.value)
            except Exception:  # noqa: BLE001 - progress is a courtesy, never a failure
                pass

    out = Outcome(session_id=sid, state=machine.state.value)
    epoch = 0
    frozen = None

    async def rollback(reason: str) -> Outcome:
        if machine.state in _fail_from:
            await step("fail")
        out.error = reason
        try:
            await target.abort(sid)
        except Exception:  # noqa: BLE001 - best effort, the target may be gone
            pass
        try:
            if frozen is not None:
                await source.unfreeze(agent_id, frozen.epoch)
            await step("rollback_ok")
        except Exception as exc:  # noqa: BLE001
            out.error += f"; source could not resume: {exc}"
            out.needs_attention = True
        return _finish(out, machine)

    try:
        frozen = await source.freeze(agent_id)
        await step("freeze_ok")
        epoch = out.epoch = frozen.epoch + 1
        have = await target.offer(sid, frozen.manifest, epoch)
        await step("offer_ok")
        for rel in sorted(frozen.manifest.files):
            if rel.startswith(BLOB_PREFIX) and rel[len(BLOB_PREFIX) :] in have:
                out.blobs_skipped += 1
                continue
            data = await source.read(agent_id, rel)
            await target.receive(sid, rel, data)
            out.files_sent += 1
            out.bytes_sent += len(data)
        await step("transfer_ok")
        await target.verify(sid)
        await step("verify_ok")
        await target.prepare(sid)
        await step("prepare_ok")
    except Exception as exc:  # noqa: BLE001
        return await rollback(f"{type(exc).__name__}: {exc}")

    await step("commit_sent")
    try:
        await target.activate(sid, epoch)
    except TransportError as exc:
        await step("ack_lost")
        out.error = f"reply lost: {exc}"
        settled = await resolve_in_doubt(source, target, agent_id, sid, frozen.epoch, epoch)
        out.needs_attention = settled.state == State.IN_DOUBT.value
        out.state = settled.state
        out.history = [s.value for s in machine.history]
        return out
    except Exception as exc:  # noqa: BLE001 - the target said no, so it did not activate
        await step("activate_refused")
        return await rollback(f"{type(exc).__name__}: {exc}")
    await step("activate_ok")
    try:
        await source.tombstone(agent_id, epoch)
    except Exception as exc:  # noqa: BLE001 - fencing already protects us
        out.error = f"tombstone pending: {exc}"
    return _finish(out, machine)


_fail_from = {
    State.PLANNED,
    State.FROZEN,
    State.OFFERED,
    State.TRANSFERRED,
    State.VERIFIED,
    State.READY,
}


def _finish(out: Outcome, machine: Machine) -> Outcome:
    out.state = machine.state.value
    out.history = [s.value for s in machine.history]
    return out


async def resolve_in_doubt(
    source: Endpoint,
    target: Endpoint,
    agent_id: str,
    session_id: str,
    old_epoch: int,
    new_epoch: int,
) -> Outcome:
    """Settle a move whose last reply was lost, by asking who holds the epoch.

    Stateless on purpose: it can run after a crash, from either side. If the
    target cannot be reached the answer stays IN_DOUBT and the agent stays
    frozen on the source: nothing runs twice, nothing is lost."""
    out = Outcome(session_id=session_id, state=State.IN_DOUBT.value, epoch=new_epoch)
    try:
        holder = await target.holder(agent_id)
    except Exception as exc:  # noqa: BLE001
        out.error = f"target unreachable: {exc}"
        return out
    if holder == (target.node_id, new_epoch):
        await source.tombstone(agent_id, new_epoch)
        out.state = State.COMMITTED.value
        return out
    try:
        await target.abort(session_id)
        await source.unfreeze(agent_id, old_epoch)
    except Exception as exc:  # noqa: BLE001
        out.error = f"rollback incomplete: {exc}"
        out.state = State.ROLLING_BACK.value
        out.needs_attention = True
        return out
    out.state = State.ROLLED_BACK.value
    return out
