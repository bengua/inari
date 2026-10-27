# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Newest first. An entry says what changed for someone using the thing.

## [Unreleased]

### Added

- **Core passes**: a person gets threads on your core — *supervision* (a child: you follow every
  thread and answer the agents' requests; they always see the banner) or *collaboration*. Passes
  name the allowed harnesses and may end on a date; created and revoked from **+**.
- **Attach a core from the interface**: **+** next to the cores creates a one-time invitation;
  `inari-core-rs join <hub>:<port> <code> --service` on the new machine receives the mesh's keys,
  writes its configuration and starts it (`enroll_port` on the hub).
- **Inari is this project**: the README presents Inari, how it differs from other agent cores,
  and thanks David Ondrej for Cloudroom. MIT throughout.
- **Each machine serves its own interface** (`gateway_listen` on any node,
  `gateway_trust_loopback` for pages opened on the machine). Behind a gate that adds the
  access itself, the page needs no token.
- **Claude quotas from Claude itself** (`usage` kind `claude`): five-hour and weekly windows and
  extra usage, read with the machine's Claude Code login; the token goes to curl on its input.
- **Harness updates**: a harness may name the npm package it comes from (`update` in its
  configuration); each Rust node reports installed and latest versions, and the sidebar badge
  (download mark and the logos of what is out of date) installs the update on any core, runs the
  harness's restart command (OpenFox's server), and the next message uses the new version.
- **Provider usage** opens as a card inside the sidebar, as in Inari v1, instead of a popover.
- **Rust port, step 4**: the node and the gateway in Rust (`inari-core-rs node <config>`), same
  configuration file and data directory as the Python node. A Python node joins a Rust hub and
  agents teleport between them. Files from the phone arrive in parts (up to 25 MB) and reach
  the model with the message. Measured on the laptop lab: 10 MB of memory, where the Python
  node takes 40 to 45 MB.
- **Rust port, step 3**: the ACP harness and the runner turn in Rust. Photos the person
  attaches reach the model as images (Claude, Gemini), other files as links into the workspace;
  permission requests go through the approver; a freeze cancels the turn; a new session on
  another node is primed with the journal (`inari-core-rs turn`).
- **Rust port, step 2b**: event bus and TCP bus links (durable subjects replayed, duplicates
  dropped, expected peers queue before their first visit), fork and merge, and `clone_to`: copy
  an agent, give the copy another harness, send it to another core; the original stays.
- **Rust port, step 2a**: authenticated transport, capsule host and teleportation in Rust
  (`inari-core-rs serve-endpoint | adopt | teleport | status`). Python and Rust nodes teleport
  agents to each other over TCP, in both directions.

### Fixed

- The web page sent attachments in one request, which the 64 KiB body limit refused above
  about 48 KB; it now sends them in parts and names them in the message. Parts need the Rust
  gateway; the Python gateway, retired at the next step, still takes small files only.

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
