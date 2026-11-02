"""Gates: after-the-fact verification of what an envelope claims.

A gate never predicts — it runs once the phase is over, against the envelope's
own declarations and the workspace on disk. Signature: ``gate(envelope, ctx)``
with ``ctx`` carrying ``workdir``; the report lists one check per thing examined,
so a green gate says *what* it verified.
"""

import os
from dataclasses import dataclass, field


@dataclass
class GateReport:
    gate: str
    checks: list = field(default_factory=list)  # (item, ok, note)

    @property
    def ok(self):
        return all(ok for _, ok, _ in self.checks)

    def check(self, item, ok, note=""):
        self.checks.append((item, bool(ok), note))
        return ok

    @property
    def detail(self):
        bad = [f"{item}: {note}" for item, ok, note in self.checks if not ok]
        return "; ".join(bad) if bad else f"{len(self.checks)} checks green"


def _inside(workdir, path):
    full = os.path.realpath(os.path.join(workdir, path))
    root = os.path.realpath(workdir)
    return full == root or full.startswith(root + os.sep), full


def artifacts_exist(envelope, ctx):
    """Every declared artifact exists under the workspace."""
    report = GateReport("artifacts_exist")
    for art in envelope.get("artifacts", []):
        inside, full = _inside(ctx.workdir, art.get("path", ""))
        report.check(art.get("path", "?"), inside and os.path.exists(full),
                     "" if inside else "escapes the workspace")
    if not envelope.get("artifacts"):
        report.check("(none declared)", True, "vacuous")
    return report


def files_non_empty(envelope, ctx):
    """Every declared artifact file holds something."""
    report = GateReport("files_non_empty")
    for art in envelope.get("artifacts", []):
        inside, full = _inside(ctx.workdir, art.get("path", ""))
        ok = inside and os.path.isfile(full) and os.path.getsize(full) > 0
        report.check(art.get("path", "?"), ok)
    if not envelope.get("artifacts"):
        report.check("(none declared)", True, "vacuous")
    return report


def _glob_match(pattern, path):
    """fnmatch with ** meaning any number of directories."""
    import fnmatch

    if "**" in pattern:
        head, _, tail = pattern.partition("**")
        tail = tail.lstrip("/")
        if not path.startswith(head.rstrip("/")[: len(head.rstrip("/"))]):
            return False
        rest = path[len(head):]
        return not tail or fnmatch.fnmatch(rest, tail) or fnmatch.fnmatch(
            os.path.basename(rest), tail)
    return fnmatch.fnmatch(path, pattern)


def write_scope_respected(envelope, ctx):
    """Every changed file falls inside the phase's declared write scope."""
    report = GateReport("write_scope_respected")
    scope = envelope.get("write_scope", [])
    for changed in envelope.get("changed_files", []):
        inside, _ = _inside(ctx.workdir, changed)
        ok = inside and any(_glob_match(g, changed) for g in scope)
        report.check(changed, ok,
                     "" if inside else "escapes the workspace")
    if not envelope.get("changed_files"):
        report.check("(none declared)", True, "vacuous")
    return report


ALL = [artifacts_exist, files_non_empty, write_scope_respected]
