import os
import tempfile
import unittest

from graphe.gates import (
    artifacts_exist,
    files_non_empty,
    write_scope_respected,
)
from graphe.runner import _Ctx


def env(**kw):
    base = {"artifacts": [], "changed_files": [], "write_scope": []}
    base.update(kw)
    return base


class GatesTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.ctx = _Ctx(self.dir.name)

    def tearDown(self):
        self.dir.cleanup()

    def write(self, name, text="x"):
        path = os.path.join(self.dir.name, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return name

    def test_artifacts_exist(self):
        self.write("report.md")
        e = env(artifacts=[{"path": "report.md", "sha256": "", "kind": "file"}])
        self.assertTrue(artifacts_exist(e, self.ctx).ok)
        e = env(artifacts=[{"path": "missing.md", "sha256": "", "kind": "file"}])
        report = artifacts_exist(e, self.ctx)
        self.assertFalse(report.ok)
        self.assertIn("missing.md", report.detail)

    def test_files_non_empty(self):
        self.write("empty.md", "")
        e = env(artifacts=[{"path": "empty.md", "sha256": "", "kind": "file"}])
        self.assertFalse(files_non_empty(e, self.ctx).ok)
        self.write("full.md", "hello")
        e = env(artifacts=[{"path": "full.md", "sha256": "", "kind": "file"}])
        self.assertTrue(files_non_empty(e, self.ctx).ok)

    def test_write_scope_globs(self):
        e = env(changed_files=["src/a.rs", "docs/b.md"],
                write_scope=["src/**", "docs/*.md"])
        self.assertTrue(write_scope_respected(e, self.ctx).ok)
        e = env(changed_files=["etc/passwd"], write_scope=["src/**"])
        report = write_scope_respected(e, self.ctx)
        self.assertFalse(report.ok)
        self.assertIn("etc/passwd", report.detail)

    def test_nothing_escapes_the_workspace(self):
        e = env(artifacts=[{"path": "../outside.md", "sha256": "", "kind": "file"}])
        self.assertFalse(artifacts_exist(e, self.ctx).ok)

    def test_a_green_gate_says_what_it_checked(self):
        self.write("a.md")
        e = env(artifacts=[{"path": "a.md", "sha256": "", "kind": "file"}])
        self.assertIn("1 checks green", artifacts_exist(e, self.ctx).detail)


if __name__ == "__main__":
    unittest.main()
