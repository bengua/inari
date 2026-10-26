# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Newest first. An entry says what changed for someone using the thing.

## [Unreleased]

### Added

- **Web interface and gateway**: node/agent graph with drag-to-teleport, journal tree,
  live orb, approvals, phone layout; `inari-core demo` and `inari-core gateway`.
- **Transport**: authenticated TCP framing, bus links with durable replay, remote
  endpoints (teleportation across processes).
- **Sandbox**: subprocess and container runtimes; workspace snapshots update the
  capsule differentially.
- **Signatures** (`inari-core keygen|sign|verify --trusted`) and **approvals over the
  bus** with queued questions.
- **Session State Capsule (`ssc/1`)**: an agent thread as a directory of plain
  files — manifest with per-file digests, hash-chained journal, working memory,
  local vector index, content-addressed workspace snapshot, sandbox spec. Packs to
  a deterministic tar; `inari-core verify` checks a directory or archive offline.
- **Provider handover**: render the neutral journal for Anthropic or OpenAI
  schemas, drop private reasoning across providers, close tool calls cut short by a
  crash, compact for a narrower context window.
- **Teleportation**: two-phase commit with a fencing epoch, differential transfer
  (only missing blobs travel), rollback before the commit point, stateless
  resolution of the in-doubt window. Tested with injected faults.
- **Mesh**: embedded bus with NATS-style subjects, links with durable subjects and
  deduplicated replay, node registry, coordinator, telemetry and the mascot's
  state as a pure function.
- **BYOK secrets** and **signed single-use approvals** with default deny.
- JSON Schemas generated from the models (`schema/`), checked for drift by a test.
- Specifications: architecture, capsule format, teleportation, mesh, security, UX.
