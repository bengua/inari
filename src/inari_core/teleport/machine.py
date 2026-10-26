"""The teleportation state machine: a two-phase commit with a fencing epoch.

    PLANNED -> FROZEN -> OFFERED -> TRANSFERRED -> VERIFIED -> READY
            -> COMMITTING -> COMMITTED

Anything before COMMITTING can fail into ROLLING_BACK -> ROLLED_BACK, which
resumes the agent on the source with its old epoch. The one ambiguous moment is
the reply to `activate`: if it is lost the move is IN_DOUBT, and the epoch
decides who runs the agent (see `driver.resolve_in_doubt`)."""

from __future__ import annotations

from enum import StrEnum

from ..errors import IllegalTransition


class State(StrEnum):
    PLANNED = "planned"
    FROZEN = "frozen"
    OFFERED = "offered"
    TRANSFERRED = "transferred"
    VERIFIED = "verified"
    READY = "ready"
    COMMITTING = "committing"
    COMMITTED = "committed"
    IN_DOUBT = "in_doubt"
    ROLLING_BACK = "rolling_back"
    ROLLED_BACK = "rolled_back"


TERMINAL = {State.COMMITTED, State.ROLLED_BACK}
_PRE_COMMIT = [
    State.PLANNED,
    State.FROZEN,
    State.OFFERED,
    State.TRANSFERRED,
    State.VERIFIED,
    State.READY,
]

TRANSITIONS: dict[tuple[State, str], State] = {
    (State.PLANNED, "freeze_ok"): State.FROZEN,
    (State.FROZEN, "offer_ok"): State.OFFERED,
    (State.OFFERED, "transfer_ok"): State.TRANSFERRED,
    (State.TRANSFERRED, "verify_ok"): State.VERIFIED,
    (State.VERIFIED, "prepare_ok"): State.READY,
    (State.READY, "commit_sent"): State.COMMITTING,
    (State.COMMITTING, "activate_ok"): State.COMMITTED,
    (State.COMMITTING, "activate_refused"): State.ROLLING_BACK,
    (State.COMMITTING, "ack_lost"): State.IN_DOUBT,
    (State.IN_DOUBT, "resolved_committed"): State.COMMITTED,
    (State.IN_DOUBT, "resolved_aborted"): State.ROLLING_BACK,
    (State.ROLLING_BACK, "rollback_ok"): State.ROLLED_BACK,
    **{(s, "fail"): State.ROLLING_BACK for s in _PRE_COMMIT},
}


class Machine:
    def __init__(self) -> None:
        self.state = State.PLANNED
        self.history: list[State] = [State.PLANNED]

    def fire(self, event: str) -> State:
        try:
            self.state = TRANSITIONS[(self.state, event)]
        except KeyError:
            raise IllegalTransition(f"{event!r} is not allowed in {self.state}") from None
        self.history.append(self.state)
        return self.state
