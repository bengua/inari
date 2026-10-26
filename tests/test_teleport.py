from __future__ import annotations

from pathlib import Path

import pytest

from inari_core.capsule import load, verify
from inari_core.errors import IllegalTransition
from inari_core.teleport import (
    CapsuleHost,
    Faulty,
    Machine,
    State,
    resolve_in_doubt,
    teleport,
)
from tests.conftest import make_capsule

AGENT = "thread-1"  # an agent is identified by its thread id


@pytest.fixture
def pair(tmp_path: Path):
    source = CapsuleHost("node-a", tmp_path / "a", clock=lambda: 5.0)
    target = CapsuleHost("node-b", tmp_path / "b", clock=lambda: 6.0)
    made = make_capsule(tmp_path / "seed")
    source.adopt(made.root)
    return source, target


def holders(*hosts: CapsuleHost) -> list[str]:
    """Nodes that believe they run the agent: must never exceed one."""
    return [h.node_id for h in hosts if h.status(AGENT) == "running"]


async def test_happy_path_moves_the_agent_and_leaves_one_holder(pair):
    source, target = pair
    out = await teleport(source, target, AGENT)
    assert out.state == "committed" and out.epoch == 2
    assert out.history[-1] == "committed" and out.error == ""
    assert holders(source, target) == ["node-b"]
    assert source.status(AGENT) == "tombstoned"
    moved = target.agents / AGENT
    assert verify(moved) == []
    assert load(moved).manifest.holder.node_id == "node-b"
    assert await target.holder(AGENT) == ("node-b", 2)


async def test_the_journal_survives_the_move_with_a_checkpoint(pair):
    source, target = pair
    await teleport(source, target, AGENT)
    kinds = [e.kind.value for e in load(target.agents / AGENT).timeline.ordered()]
    assert kinds[-1] == "checkpoint" and kinds.count("user_message") == 1


async def test_only_missing_blobs_travel(pair):
    source, target = pair
    first = await teleport(source, target, AGENT)
    # Bring it back: the origin already holds every blob.
    back = await teleport(target, source, AGENT)
    assert back.state == "committed" and back.epoch == 3
    assert back.blobs_skipped == 2
    assert back.files_sent == first.files_sent - 2  # the two blobs stayed home
    assert holders(source, target) == ["node-a"]


@pytest.mark.parametrize("step", ["offer", "receive", "verify", "prepare"])
async def test_a_failure_before_commit_resumes_the_source(pair, step):
    source, target = pair
    out = await teleport(source, Faulty(target, fail_on=step), AGENT)
    assert out.state == "rolled_back" and out.error
    assert holders(source, target) == ["node-a"]
    assert not list(target.staging.iterdir())
    assert await source.holder(AGENT) == ("node-a", 1)


async def test_a_dead_source_read_rolls_back(pair):
    source, target = pair
    out = await teleport(Faulty(source, fail_on="read"), target, AGENT)
    assert out.state == "rolled_back"
    assert source.status(AGENT) == "running"


async def test_corruption_in_transit_is_caught_by_the_digests(pair):
    source, target = pair

    class Corrupting(Faulty):
        async def receive(self, sid, rel, data):
            bad = data[:-1] + b"X" if rel == "memory/working_memory.md" else data
            await self._inner.receive(sid, rel, bad)

    out = await teleport(source, Corrupting(target), AGENT)
    assert out.state == "rolled_back" and "digest mismatch" in out.error
    assert holders(source, target) == ["node-a"]


async def test_a_lost_reply_after_activation_is_settled_by_the_epoch(pair):
    source, target = pair
    out = await teleport(source, Faulty(target, fail_on="activate", when="after"), AGENT)
    assert out.state == "committed"  # the target did take over
    assert holders(source, target) == ["node-b"]


async def test_a_lost_request_before_activation_rolls_back(pair):
    source, target = pair
    out = await teleport(source, Faulty(target, fail_on="activate", when="before"), AGENT)
    assert out.state == "rolled_back"
    assert holders(source, target) == ["node-a"]


async def test_an_unreachable_target_leaves_the_move_in_doubt_not_duplicated(pair):
    source, target = pair

    class Gone(Faulty):
        async def activate(self, sid, epoch):
            await self._inner.activate(sid, epoch)
            from inari_core.errors import TransportError

            raise TransportError("reply lost")

        async def holder(self, agent_id):
            from inari_core.errors import TransportError

            raise TransportError("unreachable")

    out = await teleport(source, Gone(target), AGENT)
    assert out.state == "in_doubt" and out.needs_attention
    assert holders(source, target) == ["node-b"]  # source stays frozen
    assert source.status(AGENT) == "frozen"
    # Later, with the link back, the epoch settles it.
    settled = await resolve_in_doubt(source, target, AGENT, out.session_id, 1, 2)
    assert settled.state == "committed" and source.status(AGENT) == "tombstoned"


async def test_a_stale_epoch_is_refused(pair):
    source, target = pair
    await teleport(source, target, AGENT)  # target now holds epoch 2
    # A second source still believing in epoch 1 tries to move it again.
    other = CapsuleHost("node-c", source.root.parent / "c")
    other.adopt(source.agents / AGENT)
    out = await teleport(other, target, AGENT)
    assert out.state == "rolled_back" and "StaleEpoch" in out.error
    assert await target.holder(AGENT) == ("node-b", 2)


async def test_recover_repairs_the_record_after_a_crash(pair, tmp_path):
    source, target = pair
    await teleport(source, target, AGENT)
    (target.root / "holders.json").unlink()  # crash between rename and record
    restarted = CapsuleHost("node-b", target.root)
    assert restarted.recover() == [AGENT]
    assert await restarted.holder(AGENT) == ("node-b", 2)


def test_the_machine_refuses_impossible_moves():
    machine = Machine()
    with pytest.raises(IllegalTransition):
        machine.fire("activate_ok")
    for event in ("freeze_ok", "offer_ok", "transfer_ok", "verify_ok", "prepare_ok"):
        machine.fire(event)
    assert machine.state is State.READY
    machine.fire("commit_sent")
    with pytest.raises(IllegalTransition):
        machine.fire("fail")  # past the point of no return
