# Porting the core to Rust

The Python package in `src/inari_core` is the reference: its formats, protocols and
tests are the specification. The Rust crate in `crates/inari-core` implements them one
component at a time. Both speak the same bytes, so a fleet can run a mix of Python and
Rust nodes and move machine by machine.

## Rules

- **Byte-compatible.** Canonical JSON, event hashes, the journal head, file digests and
  the archive layout are identical. `tests/test_rust_interop.py` drives the Rust binary
  against Python-made capsules and the other way round; a mutation of the Rust hashing
  makes it fail.
- **Same words.** Verification problems are reported in the same wording, so tools and
  people can compare.
- **Small dependency set.** serde/serde_json, sha2, tar, thiserror so far; each new crate
  is justified in its pull request.
- **Exact numbers.** `serde_json` is built with `float_roundtrip`: without it some
  timestamps parse one ULP off and an event's hash changes after another node rewrote the
  journal (found by the interop tests, about one capsule in three).
- **Pinned toolchain.** `rust-toolchain.toml`. CI installs it while the runner image does
  not carry one.

## Order

| Step | Content | State |
|---|---|---|
| 1 | Capsule: canonical JSON, journal, manifest, verify, pack/unpack, workspace attachments, uploads in parts | done |
| 2a | Transport (authenticated frames), capsule host, teleportation with fencing, endpoint server and client — Python and Rust nodes teleport agents to each other over TCP | done |
| 2b | Bus and TCP bus links (durable replay, duplicates dropped), fork/merge, and **clone to another core with another harness** (fork, switch harness, teleport the copy; the original stays) — Python and Rust buses linked both ways | done |
| 3 (next) | ACP harness, with attachments passed to the model: images as image content to Claude and Gemini, PDFs and other files placed in the workspace and named in the message | |
| 4 | Node daemon and gateway (HTTP/SSE, uploads in parts from the Android app's share sheet, camera and file picker), serving the web interface | |
| 5 | Switch the three cores to the Rust binary, retire the Python runtime | |

## Attachments, as the phone sends them

The Android app keeps its native share target (text, links, photos — JPEG, PNG, WebP,
HEIC — and PDF), camera and file picker. It sends the file to the gateway in parts; the
core assembles it (`UploadStore`: each part starts where the last ended, retries are
ignored, a size limit applies), recognises it by its bytes (`sniff`), and attaches it to
the thread's workspace (`attach`), so it travels with the agent.
