"""The demonstration run: the §3 pipeline end to end, with one gate that fails
on purpose and then passes after a correction in the same session.

Pipeline: engineer(intention) -> agent(builder) -> code(report) -> engineer(merge).
The scripted builder first returns an envelope claiming an artifact it has not
written (the ``artifacts_exist`` gate fails), then — corrected in the SAME
session — writes the file and answers again. No model is involved: the caller
is scripted, so the demonstration runs anywhere, CI included.
"""

import json
import os
import sys

from .gates import artifacts_exist, files_non_empty
from .runner import AgentCall, Run, sha256_of
from .store import Store


class ScriptedBuilder:
    """A stand-in agent: it keeps one state per session, so a correction lands
    in the same session the first attempt was made in."""

    def __init__(self, workdir):
        self.workdir = workdir
        self.sessions = {}

    def __call__(self, session_id, prompt, correction):
        state = self.sessions.setdefault(session_id, {"calls": 0})
        state["calls"] += 1
        if correction is None:
            # first try: claims the artifact, has not written it — the gate fails
            return json.dumps({
                "status": "done",
                "summary": "the report is written",
                "artifacts": [{"path": "report.md", "sha256": "", "kind": "report"}],
                "changed_files": ["report.md"],
                "notes_for_next_agent": "read report.md",
                "commit_message": "docs: the builder's report",
            })
        # corrected in the same session: the file is written this time
        path = os.path.join(self.workdir, "report.md")
        with open(path, "w", encoding="utf-8") as f:
            f.write("# Report\n\nWritten after the gate's correction, same session.\n")
        return json.dumps({
            "status": "done",
            "summary": "the report is written, for real this time",
            "artifacts": [{"path": "report.md", "sha256": sha256_of(path),
                           "kind": "report"}],
            "changed_files": ["report.md"],
            "notes_for_next_agent": "read report.md",
            "commit_message": "docs: the builder's report",
        })


def run_demo(db_path, workdir, human_yes=True):
    """The full pipeline; returns (run_id, accepted) and leaves the WAL trace."""
    store = Store(db_path)
    run = Run.ensure(store, request="demonstration of the §3 pipeline",
                     workdir=workdir)
    builder = ScriptedBuilder(workdir)

    with run.phase("intention", "engineer", "ben",
                   "the owner states the intention") as ph:
        ph.decide(True, "go: demonstrate the pipeline")

    with run.phase("build", "agent", "builder",
                   "the builder writes its report, bounded to its scope") as ph:
        envelope = ph.call(AgentCall(
            prompt="Write report.md and answer with your envelope.",
            caller=builder,
            session_id="demo-builder",
            agent="builder",
            write_scope=["report.md", "*.md"],
            gates=[artifacts_exist, files_non_empty],
        ))
        ph.succeed()

    with run.phase("report", "code", "script",
                   "code echoes what landed, no agent needed") as ph:
        ph.run_code(lambda p: {
            "summary": f"1 artifact, {len(envelope['changed_files'])} file changed",
            "artifacts": envelope["artifacts"],
            "commit_message": envelope["commit_message"],
        })

    with run.phase("merge", "engineer", "ben",
                   "the owner's verdict closes the run") as ph:
        ph.decide(human_yes, "merged" if human_yes else "refused")

    accepted = run.finish(accepted=human_yes,
                          reason="" if human_yes else "the owner refused")
    store.close()
    return run.run_id, accepted


def main(argv=None):
    argv = argv or sys.argv[1:]
    db = argv[0] if len(argv) > 0 else "/tmp/graphe-demo.db"
    workdir = argv[1] if len(argv) > 1 else "/tmp/graphe-demo-work"
    for suffix in ("", "-wal", "-shm"):
        if os.path.exists(db + suffix):
            os.remove(db + suffix)
    os.makedirs(workdir, exist_ok=True)
    if os.path.exists(os.path.join(workdir, "report.md")):
        os.remove(os.path.join(workdir, "report.md"))
    run_id, accepted = run_demo(db, workdir)
    print(f"run {run_id}: {'accepted' if accepted else 'failed'}")
    print(f"trace: {db} (sessions, phases, events, envelopes, gate_results)")
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
