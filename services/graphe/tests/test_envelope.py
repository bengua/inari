import unittest

from graphe.envelope import EnvelopeError, validate


def good():
    return {
        "envelope": "essaim/1",
        "run_id": "r1",
        "phase_id": "p1",
        "lane": "agent",
        "agent": "builder",
        "status": "done",
        "summary": "short",
        "artifacts": [{"path": "a.md", "sha256": "ab" * 32, "kind": "file"}],
        "changed_files": ["a.md"],
        "write_scope": ["*.md"],
        "notes_for_next_agent": "",
        "commit_message": "docs: a",
        "gate_results": [{"gate": "artifacts_exist", "ok": True, "detail": ""}],
        "trace_cursor": 42,
    }


class EnvelopeTest(unittest.TestCase):
    def test_a_complete_envelope_passes(self):
        self.assertEqual(validate(good())["envelope"], "essaim/1")

    def test_an_unknown_field_rejects(self):
        env = good()
        env["surprise"] = 1
        with self.assertRaises(EnvelopeError) as ctx:
            validate(env)
        self.assertIn("unknown field 'surprise'", str(ctx.exception))

    def test_a_missing_required_field_rejects(self):
        env = good()
        del env["status"]
        with self.assertRaises(EnvelopeError) as ctx:
            validate(env)
        self.assertIn("missing 'status'", str(ctx.exception))

    def test_a_bad_enum_rejects(self):
        env = good()
        env["lane"] = "agentt"
        with self.assertRaises(EnvelopeError):
            validate(env)
        env = good()
        env["status"] = "maybe"
        with self.assertRaises(EnvelopeError):
            validate(env)

    def test_nested_artifacts_are_checked(self):
        env = good()
        env["artifacts"] = [{"path": "a.md", "sha256": "x"}]  # no kind
        with self.assertRaises(EnvelopeError) as ctx:
            validate(env)
        self.assertIn("missing 'kind'", str(ctx.exception))
        env = good()
        env["artifacts"][0]["kind"] = "blob"
        with self.assertRaises(EnvelopeError):
            validate(env)

    def test_booleans_are_not_integers(self):
        env = good()
        env["trace_cursor"] = True
        with self.assertRaises(EnvelopeError):
            validate(env)

    def test_the_contract_version_is_fixed(self):
        env = good()
        env["envelope"] = "essaim/2"
        with self.assertRaises(EnvelopeError):
            validate(env)


if __name__ == "__main__":
    unittest.main()
