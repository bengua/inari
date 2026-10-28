# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Newest first. An entry says what changed for someone using the thing.

## [Unreleased]

### Added

- **`inari-core-rs demo`: the demo in Rust.** Three nodes in one process, the interface on
  `:8787`, live turns and an approval question; `--mascot DIR` shows FoxBot. The agent it runs is
  this binary too (`inari-core-rs demo-agent`): `write <file>: <text>`, `run: <command>` (a
  read-only command runs at once, anything else asks you first, `sudo` never runs).
- **Capsule signatures in Rust.** `inari-core-rs keygen <name>`, `sign <capsule> --key <name.key>`
  and `verify <capsule> --trusted <name.pub>`: the reference's format, byte for byte (the same key
  writes the same signature).
- **Capsules the Python reference wrote are frozen as test fixtures** (`tests/fixtures/python/`):
  the Rust core is checked to read them hash for hash, report damage in the same words and verify
  their signatures, so no existing capsule ever needs converting.
- **`cargo test` needs no Python.** The scripted agent of the tests is `inari-core-rs fake-agent`,
  a Rust port of `tests/fake_acp.py` with the same words and the same log.

- **Richer replies.** The thread and the Markdown viewer share one renderer: headings, bulleted and
  numbered lists, quotes, tables, rules, links (http and https only, opened outside the page) and
  code blocks with their language and a *Copy* button.

- **Connection state you can read.** When the page loses its core, a banner says so, how old what is
  on screen is, and that it is reconnecting; the stream reopens by itself. A refused token or pass
  brings the login back with the reason, and an error from the core is shown instead of dropped.

- **The page in English and French.** It follows the browser's language, and *Settings → Language*
  switches it. Every text goes through `t()` with its French in `web/i18n.js`; a test fails when a
  text has no French. What the core itself sends (an agent's error, a harness message) stays as sent.

- **Visible ways to do what gestures do.** Each thread shows an arrow to move it to another core,
  each core a **+** to start a thread on it; a one-time tip explains dragging; **?** lists the
  keyboard shortcuts, **/** searches, **Esc** closes a menu or a sheet.

- **The page's own fonts.** IBM Plex Sans and Mono and Bricolage Grotesque are served by the core
  (subset to Latin, woff2, about 120 KB), so every browser shows the same type. The page is now
  `index.html`, `app.css`, `app.js` and `fonts/`, served under `/assets/` by both gateways.

- **A thread asks only for what is new.** The page used to download a thread's whole journal every
  second and a half while an agent worked; it now asks `timeline?after=<last event>` and appends,
  and asks for the whole journal again only when it changed under it (a merge, a fork).

- **A *New messages* button** appears when the agent writes while you have scrolled up; the thread
  no longer pulls you down.

- **Approvals appear in the thread, and an auto mode.** An agent's request to run something is a
  card at the bottom of *its* thread (Deny, Allow once, Always allow here) instead of a line in
  the cores' pane; requests from other threads stay in the pane. A chip on each thread switches
  **auto mode** (the agent acts without asking; each action is journaled). It works for every
  harness, applies only to the owner, and is ignored on a thread under a core pass.

- **A context ring on every thread.** Agents report how full their context window is (Claude Code,
  Gemini CLI, OpenFox): the page shows a ring and a percentage next to the model, with the numbers,
  the session's cost when the agent gives one, and a *Compact* button when the agent has `/compact`.

- **Stop, rename and pin.** A **Stop** button replaces Send while an agent works (it asks the
  agent to cancel and the thread says *Stopped*); double-click a thread's title, or pick
  *Rename…*, to rename it in place; *Pin to the top* keeps a thread first in its core's list,
  marked with a star.

- **Each agent's own modes and slash commands, in the page.** Inari reads what every harness
  announces when it starts a session and shows it: a **mode** chip on the thread (Claude Code:
  Default, Accept Edits, Plan, Don't Ask, Bypass; Gemini: Default, Auto Edit, YOLO, Plan; OpenFox:
  Build, Plan) that the thread keeps and sets before each session, and the agent's **slash
  commands** offered when typing `/` in the composer. A mode change is journaled. Gemini's
  `[MODE_UPDATE]` notice, sent as message text, is kept out of the answer.

- **Pass threads can run in an environment of their own** (`pass_env` on a harness): a separate
  HOME per pass, so a child or a guest never inherits the owner's instructions, tools or
  credentials, and can bring their own login there.

- **On a phone, the mascot has a stage at the top of an open thread** (about 200 px instead of
  56) and folds into the top bar while the keyboard is up. The page follows a keyboard the
  Android app reports (`window.inariKeyboard(height)`): the composer rides it with a transform,
  the thread makes room once it settles — no layout on every frame.

- **A chosen model is made ready before the turn**: a harness may run a `prepare` command
  (`{model}` replaced by the id) — to load a local model onto its GPU, say — while the thread
  shows *loading the model*; a refusal ends the turn with its reason. `model_labels` renames
  models by suffix on every page (which GPU a slot uses); `model_hidden` keeps a harness's own
  helper models out of the menus.
- **Choosing the agent and the model is a step of creating a thread**: which agent of that core
  (Claude Code, Gemini CLI, OpenFox…), then which of its models (filterable). Asked by *New
  thread*, by a long press on a core, and before a first message sent without a choice. The model
  picker of a thread lists its own harness's models.
- **A long press on a core opens a new thread on it**; *New thread* now shows the composer on a
  phone too (the page only showed with a thread open).
- **Delete a thread** from the long-press menu (owner only): it leaves Inari and is kept in the
  core's `trash/` folder, never brought back by a restart (`/api/delete`).
- **Gestures**: drag a thread onto another to merge them (moved first if they live on two cores);
  onto the dock that appears to clone it to another harness, share it with a pass holder, or put it
  to sleep; onto the mascot to summarise it. Drop a file on a core to start a thread with it. On a
  phone, a short press lifts a thread, which is then carried like with a mouse (onto a core to move
  it); a long press without moving opens everything else one can do with it; long-press a message to
  fork from it. Core: `/api/clone`, `/api/share`, `/api/archive`; a thread shared with a pass belongs to
  it too.
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

### Removed

- **The Python reference.** Inari is Rust only: `src/inari_core`, its tests, `pyproject.toml`
  and the `uv` steps of `verify.sh` and CI are gone; nothing but the Rust binary is built,
  tested or installed. What only the reference did either moved to Rust (the demo, its agent,
  capsule signatures, the test agent) or was not used by the Rust core (the built-in model
  loop and its provider handover, the container sandbox, schema generation). Capsules the
  reference wrote are test fixtures the core reads hash for hash. The web page moved to
  `crates/inari-core/web/`.

### Android

- **The navigation bar no longer covers the composer** on Android 15, where apps are drawn edge to
  edge: the view stands clear of it, and the page is told only the height the keyboard adds.
- **A share cannot exhaust the phone's memory.** A shared file's size is read before the file; a
  file over the limit is never read, and one whose size is unknown is read only up to the limit.
- **A page that does not load says what to check** (the private network, the core, the
  certificate, the address), the technical reason kept small below; the unlock prompt is
  translated. Leftover strings from another app are gone.

### Security

- **Sessions instead of a token in the page.** The page signs in once and holds an `HttpOnly`,
  `SameSite=Strict` cookie signed by the core; the token no longer goes into URLs (`/events?token=`,
  logo addresses) nor into the browser's storage. A revoked pass or a new owner token ends the
  sessions they opened. Links with `#token=` or `#pass=`, and secrets older pages left in storage,
  are exchanged once and forgotten. *Settings → Sign out* ends the session.
- **A strict content policy.** No inline script may run (`script-src 'self'`), the page cannot be
  framed by another site, and no referrer leaves it.
- **Wrong secrets wait.** After five wrong tokens or passes from one address, each attempt waits
  longer, up to five minutes.
- **The gateway bounds its connections** (512 at once) and the time a request may take to arrive
  (60 s), so a client trickling bytes no longer holds a connection for hours.

### Fixed

- **An open tool call stays open.** The thread was rebuilt every second and a half while an agent
  worked, closing the tool call you were reading and dropping your text selection; only what changed
  is now redrawn, and a screen reader hears only the new messages.
- **Readable text and reachable buttons.** The faint and dim greys and the copper of the main
  buttons now meet WCAG AA contrast in both themes; buttons are at least 44 px on touch screens; the
  workbench tabs say which one is selected and follow the arrow keys; keyboard focus is visible.
- **Approvals are answered once.** Their cards are no longer rebuilt and re-announced on every
  update, and a tap disables the card's buttons until the core answers.
- **A draft survives the page updating itself**, and comes back in its thread.
- **Forks get names that cannot collide**, and the login field submits with Enter.
- **The gateway keeps accepting connections after a failed accept** (out of file descriptors, a
  reset) instead of stopping until restarted.

- **An agent's permission request was lost when its id was a string** (OpenFox's adapter sends
  `ofx-1`): the reader only accepted numbers, ignored the request, and the thread waited forever
  while the page showed nothing to approve. Both number and string ids are answered now.

- A thread deleted while its turn was still running came back as an empty ghost in the page
  (the turn's last phase update re-created it). Phases of a deleted thread are now ignored.

- The agent's replies reached the page cut at 400 characters (live and in the history): a
  summary's length was applied to messages too. Messages now arrive whole; other events are
  shortened to 4,000 characters.

- Two changes to one capsule at once (a model chosen while a message arrives) overwrote each
  other: the model label or the user's message was lost. Every read-change-seal of a capsule
  now holds a per-capsule lock.
- The model chosen when creating a thread travels with the creation, so the first turn uses it
  (it used to race the first message and lose).

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
