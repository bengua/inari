"""How tests/fixtures/python/ was made, kept for the record: capsules written by the Python
reference before it was retired (October 2026), so the Rust core keeps proving that it reads,
verifies and signs exactly as the reference did. It ran once, from the repository root, with
the reference installed:

    python crates/inari-core/tests/fixtures/make_python_fixtures.py \\
        crates/inari-core/tests/fixtures/python "$(mktemp -d)"

The reference is gone from the repository since: this script is not run again."""

import json
import shutil
import sys
from pathlib import Path

sys.path.insert(0, "tests")
from conftest import make_capsule  # noqa: E402

from inari_core import signature  # noqa: E402
from inari_core.capsule import EventKind, append_event, load, pack, verify  # noqa: E402

out, work = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
c = make_capsule(work / "capsule")
# timestamps that need exact float round trips, and text beyond ASCII
append_event(
    c.root, "node-a@thread-1", EventKind.ASSISTANT_MESSAGE, {"text": "fait ✓ — λ 🦊"},
    1714000000.123456789,
)  # fmt: skip
append_event(
    c.root, "node-b@thread-1", EventKind.DECISION, {"text": "moved", "score": 0.1 + 0.2},
    1714000001.987654321,
)  # fmt: skip
assert verify(c.root) == []
shutil.copytree(c.root, out / "capsule")
pack(c.root, out / "capsule.ssc")
private, public = signature.generate_keypair()
(out / "test-signer.key").write_text(private)  # a key for this test only: it signs nothing else
(out / "test-signer.pub").write_text(public)
shutil.copytree(c.root, out / "signed")
signature.sign_capsule(out / "signed", private)
loaded = load(c.root)
expected = {
    "thread": loaded.manifest.thread_id,
    "capsule": loaded.manifest.capsule_id,
    "events": len(loaded.timeline),
    "hashes": [e.hash for e in loaded.timeline.ordered()],
    "workspace": sorted(loaded.snapshot.tree),
    "key_id": signature.key_id(public),
}
(out / "expected.json").write_text(json.dumps(expected, indent=2, ensure_ascii=False) + "\n")
print("ok", expected["events"], "events")
