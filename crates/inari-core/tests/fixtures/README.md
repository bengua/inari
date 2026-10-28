# Test fixtures

## `python/`: capsules the Python reference wrote

Before the Python reference was retired (October 2026), it wrote a capsule (`capsule/`), its
archive (`capsule.ssc`) and a signed copy (`signed/`, key pair `test-signer.key` and
`test-signer.pub`, made for this test and signing nothing else), with what they read as
(`expected.json`). `tests/python_capsules.rs` checks the Rust core against them: the same event
hashes, damage reported in the same words, appends that stay valid, and signatures identical byte
for byte. Capsules written by the old engine therefore never need converting.

They were made once, from the repository root, with the reference installed, by this script
(kept here for the record; the reference is no longer in the repository):

```python
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
```
