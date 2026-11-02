import json
import os
import tempfile
import unittest

from graphe.demo import run_demo
from graphe.gates import artifacts_exist
from graphe.runner import AgentCall, PhaseError, Run
from graphe.store import Store


class RunnerTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.work = os.path.join(self.dir.name, "work")
        self.store = Store(os.path.join(self.dir.name, "trace.db"))
        self.run = Run.ensure(self.store, request="test", workdir=self.work)

    def tearDown(self):
        self.store.close()
        self.dir.cleanup()

    def test_a_phase_defaults_to_fail_success_is_earned(self):
        with self.run.phase("p", "code", "script") as ph:
            pass  # no succeed(): the phase fails even without an error
        self.assertEqual(self.store.phases_of(self.run.run_id)[0]["status"], "fail")

    def test_the_code_lane_generates_its_envelope(self):
        with self.run.phase("c", "code", "script") as ph:
            env = ph.run_code(lambda p: {"summary": "ran"})
        self.assertEqual(env["lane"], "code")
        self.assertEqual(self.store.phases_of(self.run.run_id)[0]["status"], "success")

    def test_the_engineer_lane_carries_the_verdict(self):
        with self.run.phase("e", "engineer", "ben") as ph:
            env = ph.decide(False, "refused", "not good enough")
        self.assertEqual(env["status"], "blocked")
        self.assertEqual(self.store.phases_of(self.run.run_id)[0]["status"], "fail")

    def test_an_invalid_envelope_is_re_asked_in_the_same_session(self):
        calls = []

        def caller(session_id, prompt, correction):
            calls.append((session_id, correction))
            if correction is None:
                return "not json at all"
            return json.dumps({"summary": "fixed", "status": "done"})

        with self.run.phase("a", "agent", "builder") as ph:
            env = ph.call(AgentCall(prompt="x", caller=caller, session_id="s1"))
            ph.succeed()
        self.assertEqual(env["summary"], "fixed")
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0][0], calls[1][0], "same session")
        self.assertIsNotNone(calls[1][1], "the correction names the defect")
        envelopes = self.store.envelopes_of(self.run.run_id)
        self.assertEqual([e["valid"] for e in envelopes], [0, 1])

    def test_a_phase_gives_up_after_too_many_corrections(self):
        def caller(session_id, prompt, correction):
            return "still not json"

        with self.assertRaises(PhaseError):
            with self.run.phase("a", "agent", "builder") as ph:
                ph.call(AgentCall(prompt="x", caller=caller, session_id="s1",
                                  max_corrections=2))
        self.assertEqual(self.store.phases_of(self.run.run_id)[0]["status"], "fail")

    def test_a_failing_gate_loops_back_then_passes(self):
        state = {"calls": 0}

        def caller(session_id, prompt, correction):
            state["calls"] += 1
            if correction is None:
                return json.dumps({
                    "summary": "claimed",
                    "artifacts": [{"path": "out.md", "sha256": "", "kind": "file"}],
                    "changed_files": ["out.md"],
                })
            with open(os.path.join(self.work, "out.md"), "w") as f:
                f.write("real")
            return json.dumps({
                "summary": "done",
                "artifacts": [{"path": "out.md", "sha256": "", "kind": "file"}],
                "changed_files": ["out.md"],
            })

        with self.run.phase("a", "agent", "builder") as ph:
            env = ph.call(AgentCall(prompt="x", caller=caller, session_id="s1",
                                    gates=[artifacts_exist]))
            ph.succeed()
        self.assertEqual(state["calls"], 2)
        self.assertEqual(env["summary"], "done")
        gates = self.store.gates_of(self.run.run_id)
        self.assertEqual([g["ok"] for g in gates], [0, 1],
                         "the failure is in the trace before the pass")


class DemoRunTest(unittest.TestCase):
    """The acceptance criterion: the §3 pipeline, one gate failing on purpose
    then passing after a correction in the same session, the WAL trace readable
    by cursor."""

    def test_the_demo_pipeline(self):
        with tempfile.TemporaryDirectory() as d:
            run_id, accepted = run_demo(os.path.join(d, "trace.db"),
                                        os.path.join(d, "work"))
            self.assertTrue(accepted)
            store = Store(os.path.join(d, "trace.db"))
            # the four phases of the pipeline, in order, all earned
            phases = store.phases_of(run_id)
            self.assertEqual([p["name"] for p in phases],
                             ["intention", "build", "report", "merge"])
            self.assertEqual([p["lane"] for p in phases],
                             ["engineer", "agent", "code", "engineer"])
            self.assertTrue(all(p["status"] == "success" for p in phases))
            # the voluntary failure is in the trace: the build phase holds two
            # envelopes — the failed claim, then the corrected one, same session
            envelopes = store.envelopes_of(run_id)
            self.assertTrue(all(e["valid"] == 1 for e in envelopes))
            build = [p for p in phases if p["name"] == "build"][0]
            build_envs = [e for e in envelopes if e["phase_id"] == build["phase_id"]]
            self.assertEqual(len(build_envs), 2,
                             "the failed claim and the correction, same phase")
            gates = store.gates_of(run_id)
            build_gates = [g for g in gates if g["phase_id"] == build["phase_id"]]
            first_try = [g["ok"] for g in build_gates
                         if g["envelope_id"] == build_envs[0]["id"]]
            corrected = [g["ok"] for g in build_gates
                         if g["envelope_id"] == build_envs[1]["id"]]
            self.assertIn(0, first_try, "the first claim fails a gate on purpose")
            self.assertNotIn(1, first_try, "nothing green in the failed claim")
            self.assertTrue(corrected and all(ok == 1 for ok in corrected),
                            "everything green after the correction")
            # readable by cursor: sessions, phases, events, envelopes, gate_results
            events = store.events_after(run_id, 0)
            kinds = {e["kind"] for e in events}
            self.assertIn("session_started", kinds)
            self.assertIn("phase_started", kinds)
            self.assertIn("gate_result", kinds)
            self.assertIn("envelope_recorded", kinds)
            self.assertIn("session_finished", kinds)
            cursor = events[len(events) // 2]["seq"]
            rest = store.events_after(run_id, cursor)
            self.assertEqual(rest[0]["seq"], cursor + 1)
            self.assertEqual(store.session_of(run_id)["status"], "accepted")
            store.close()

    def test_the_demo_reports_a_refused_merge(self):
        with tempfile.TemporaryDirectory() as d:
            run_id, accepted = run_demo(os.path.join(d, "trace.db"),
                                        os.path.join(d, "work"), human_yes=False)
            self.assertFalse(accepted)
            store = Store(os.path.join(d, "trace.db"))
            self.assertEqual(store.session_of(run_id)["reason"], "the owner refused")
            store.close()


if __name__ == "__main__":
    unittest.main()
