import os
import tempfile
import unittest

from graphe.store import Store


class StoreTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.db = os.path.join(self.dir.name, "trace.db")
        self.store = Store(self.db)

    def tearDown(self):
        self.store.close()
        self.dir.cleanup()

    def test_ensure_pins_the_session(self):
        self.store.ensure_session("r1", "first")
        self.store.ensure_session("r1", "second attempt")
        self.assertEqual(self.store.session_of("r1")["request"], "first")
        self.assertEqual(self.store.session_of("r1")["status"], "running")

    def test_finish_settles_status_accepted_and_reason(self):
        self.store.ensure_session("r1", "x")
        self.store.finish_session("r1", False, "the owner refused")
        s = self.store.session_of("r1")
        self.assertEqual((s["status"], s["accepted"], s["reason"]),
                         ("failed", 0, "the owner refused"))

    def test_phases_land_in_order(self):
        self.store.ensure_session("r1", "x")
        self.store.start_phase("p2", "r1", 2, "build", "agent", "builder", "")
        self.store.start_phase("p1", "r1", 1, "plan", "agent", "planner", "")
        self.store.end_phase("p1", "r1", True)
        phases = self.store.phases_of("r1")
        self.assertEqual([p["name"] for p in phases], ["plan", "build"])
        self.assertEqual(phases[0]["status"], "success")
        self.assertEqual(phases[1]["status"], "running")

    def test_events_are_read_by_cursor(self):
        self.store.ensure_session("r1", "x")
        first = self.store.emit("r1", None, "a", {})
        self.store.emit("r1", None, "b", {})
        seen = self.store.events_after("r1", first)
        self.assertEqual([e["kind"] for e in seen], ["b"])
        everything = self.store.events_after("r1", 0)
        self.assertGreaterEqual(len(everything), 3)

    def test_envelopes_and_gates_are_kept(self):
        self.store.ensure_session("r1", "x")
        self.store.start_phase("p1", "r1", 1, "build", "agent", "builder", "")
        eid = self.store.record_envelope("r1", "p1", {"status": "done"}, True)
        self.store.record_gate("r1", "p1", eid, "artifacts_exist", False, "missing")
        envelopes = self.store.envelopes_of("r1")
        self.assertEqual((envelopes[0]["valid"], envelopes[0]["id"]), (1, eid))
        gates = self.store.gates_of("r1")
        self.assertEqual((gates[0]["gate"], gates[0]["ok"]),
                         ("artifacts_exist", 0))

    def test_the_wal_is_real(self):
        with open(f"{self.db}-wal", "rb") as _:
            pass  # WAL sidecar exists once a write happened


if __name__ == "__main__":
    unittest.main()
