import os
import tempfile
import unittest

from graphe.roster import Roster, RosterError

YAML = """
defaults:
  harness: openfox
  model: local/Q3.8

agents:
  - name: scout
    purpose: recon
    writes: []
  - name: builder
    purpose: build
    writes:
      - crates/**
"""


class RosterTest(unittest.TestCase):
    def load(self, text=YAML):
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as f:
            f.write(text)
            path = f.name
        self.addCleanup(os.remove, path)
        return Roster.load(path)

    def test_names_never_models(self):
        roster = self.load()
        self.assertEqual(roster.agent("builder")["model"], "local/Q3.8")
        with self.assertRaises(RosterError):
            self.load(YAML + "  - name: local/Q3.8\n    model: local/Q3.8\n")

    def test_require_fails_before_anything_spawns(self):
        roster = self.load()
        with self.assertRaises(RosterError) as ctx:
            roster.require(["scout", "reviewer"])
        self.assertIn("reviewer", str(ctx.exception))
        roster.require(["scout", "builder"])

    def test_write_scope(self):
        roster = self.load()
        self.assertEqual(roster.write_scope("builder"), ["crates/**"])
        self.assertEqual(roster.write_scope("scout"), [])

    def test_a_duplicate_name_is_an_error(self):
        with self.assertRaises(RosterError):
            self.load(YAML + "  - name: scout\n    purpose: again\n")

    def test_the_shipped_roster_is_valid(self):
        roster = Roster.load(os.path.join(os.path.dirname(__file__), "..", "agents.yaml"))
        roster.require(["scout", "planner", "builder", "reviewer"])
        for name in ("scout", "planner", "builder", "reviewer"):
            self.assertNotEqual(roster.agent(name)["model"], name)


if __name__ == "__main__":
    unittest.main()
