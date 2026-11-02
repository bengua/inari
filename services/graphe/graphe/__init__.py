"""The deterministic graph: phases, lanes, typed envelopes, gates, WAL trace.

Code owns sequencing, retries and acceptance; an agent owns only the work
inside one bounded phase (model adopted from disler/sssf, MIT; the code here
is original). The trace reuses the CloudFoxRoom heritage: SQLite WAL, one
numbered events table, reads by cursor.
"""

from .envelope import EnvelopeError, validate
from .gates import GateReport, artifacts_exist, files_non_empty, write_scope_respected
from .roster import Roster, RosterError
from .runner import AgentCall, Phase, PhaseError, Run
from .store import Store

__all__ = [
    "AgentCall",
    "EnvelopeError",
    "GateReport",
    "Phase",
    "PhaseError",
    "Roster",
    "RosterError",
    "Run",
    "Store",
    "artifacts_exist",
    "files_non_empty",
    "validate",
    "write_scope_respected",
]
