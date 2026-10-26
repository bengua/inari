# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Newest first. An entry says what changed for someone using the thing.

## [Unreleased]

### Added

- **Rust port, step 2a**: authenticated transport, capsule host and teleportation in Rust
  (`inari-core-rs serve-endpoint | adopt | teleport | status`). Python and Rust nodes teleport
  agents to each other over TCP, in both directions.

### Fixed

- Rust read some timestamps one ULP off (`serde_json` without `float_roundtrip`), so an event's
  hash changed after another node rewrote the journal.

### Added (earlier)

- **Rust port, step 1**: the capsule format in Rust (`crates/inari-core`, binary
  `inari-core-rs`): canonical JSON, journal, manifest, verification, archives, workspace
  attachments with type detection, uploads in parts. Byte-compatible with the Python
  reference, proven by cross-language tests.
- **Interface merge**: graphite/copper and warm-paper palettes with glass and glow, FoxBot as the mascot
  (served from a configured folder), threads grouped by vertical core modules, per-thread model
  picker with family marks, normalised quota sources, attach/dictate/home composer, and an
  artifacts pane that folds, expands and resizes.
- **ACP harness**: drive OpenFox (or any Agent Client Protocol agent) as the thinking part of
  an agent; the runner now separates the shared turn from the harness (`ModelLoop` or ACP).
- **Agent driver**: OpenAI-compatible and Anthropic providers, workspace-confined tools, the
  loop that journals every step, honours a freeze at step boundaries and asks the phone for
  guarded commands.
- **Node daemon** (`inari-core node`), hub or worker, over TCP; the gateway now creates
  agents, sends messages, forks, merges and searches. The web page gains a composer, fork,
  merge, new-agent and search.
- Fork at an event, merge by journal union, journal search.
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
