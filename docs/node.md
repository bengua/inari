# Running a node

`inari-core node --config node.json` starts one node: it holds agents, serves its
capsules to peers, runs the agent loop, and joins the bus. A node without `hub` is the
hub: it also runs the coordinator and, with `gateway_listen`, the web interface.

`inari-core-rs node node.json` is the Rust node. Any node may serve the interface
(`gateway_listen`), not only the hub: its bus sees the whole mesh. With
`"gateway_trust_loopback": true`, a page opened on the machine itself
(`http://127.0.0.1:…`) needs no token; another site's page, a non-loopback host name and a
non-JSON POST are refused. It reads the same file and the same data
directory, speaks the same bus and endpoint protocols, and serves the same interface, so
a fleet can mix both and a machine can switch in place. It drives agents through ACP
harnesses only (`harness` with `"kind": "acp"`, or `harnesses`); the Python node's built-in
model loop (`provider`) and container sandbox were retired with it. It adds
`"approval_ttl"` (seconds an approval waits for the phone, default 120).

```json
{
  "node_id": "workstation",
  "data_dir": "/var/lib/inari-core",
  "key_file": "/etc/inari-core/mesh.key",
  "device_key_file": "/etc/inari-core/device.key",
  "role": "heavy",
  "bind_host": "0.0.0.0",
  "advertise_host": "<address peers use to reach this machine>",
  "hub": "",
  "bus_port": 7400,
  "endpoint_port": 7401,
  "expected_peers": ["laptop"],
  "gateway_listen": "127.0.0.1:8787",
  "gateway_token_file": "/etc/inari-core/token",
  "provider": {
    "kind": "openai",
    "base_url": "<an OpenAI-compatible endpoint>",
    "model": "<model name>",
    "api_key_ref": "GATEWAY_KEY",
    "context_tokens": 32000
  }
}
```

A worker sets `hub` to the hub's `HOST:PORT` (its bus port) and a `node_id` listed in
the hub's `expected_peers`. Nothing here is built in: addresses, models and key names
are all configuration, and every default is empty.

- **Keys.** `key_file` is the shared mesh key (all nodes), `device_key_file` the
  approval key shared by the nodes and the gateway. Provider keys are never in this
  file: `api_key_ref` names a secret read from `INARI_SECRET_<NAME>` or the keychain.
- **Harness.** Instead of the built-in model loop, a node can drive an external agent over
  the Agent Client Protocol: `"harness": {"kind": "acp", "command": ["openfox-acp"], "timeout": 600}`.
  OpenFox is the first one. The harness journals the agent's messages and tool calls, sends
  its permission requests to the phone, cancels it when a teleport freezes the agent, and,
  on a machine whose agent has no memory of the thread, opens a session primed with the
  journal so far.
- **OpenFox, natively.** `"openfox": {"kind": "openfox"}` drives the machine's own OpenFox
  through its REST API and WebSocket, the ones its web interface uses, with no bridge process
  (`src/openfox.rs`). One WebSocket per thread, kept between turns; a dropped one is opened
  again and a quiet session asked how it is, so the end of a turn is never missed; a message
  sent during a turn goes to OpenFox's own queue, which takes it at its next step. Its settings
  (address, the agent a new thread starts with, a turn's limit), its sign-in and its default
  model are set from the interface (Settings → OpenFox) and kept in
  `<data_dir>/openfox/<harness>/` (`settings.json`, `token` in mode 600, `sessions.json`). A node
  that never set them reads `openfox-acp`'s (`~/.config/openfox-acp/{url,token}`, or
  `$OPENFOX_ACP_DIR`). OpenFox must be served over `http://` on the core's own machine: it works
  on the thread's files, which the core lays out locally. `prepare` and `update` work as for
  any harness. Providers are read without their API keys.
- **Getting a model ready.** A harness may name a command run before each turn of a thread
  whose model was chosen, with `{model}` replaced by the model id:
  `"prepare": ["load-model", "{model}"], "prepare_timeout": 600`. It is how a local model is
  loaded onto a GPU before the agent asks it anything; the thread shows *loading the model*
  meanwhile, and a non-zero exit ends the turn with the command's message. The command gets
  the harness's environment, no shell.
- **Pass threads.** `"pass_env": {"HOME": "/path/pass-homes/{pass}"}` on a harness gives the
  threads that run under a core pass an environment of their own (`{pass}` = the pass id; the
  directory is created). The agent then reads none of the owner's instructions, skills or
  credentials: a child or a guest starts from nothing, and brings their own login or API key
  in that home. Without it, pass threads share the owner's environment.
- **Model names.** `"model_labels": [["-c0", " · GPU 0"], ["-x2", " · both GPUs"]]` renames,
  on every page, the models whose id ends with a suffix — to say which card a slot uses, say.
  `"model_hidden": ["-cpu"]` keeps models out of the menus: helpers the harness uses on its
  own (its sub-agents), not models to talk to.
- **Provider kinds.** `openai` (any compatible gateway), `anthropic`, and `demo` (rules
  only, no network).
- **Sandbox.** `"sandbox": "subprocess"` (default) or `"container"` with `"engine"`.
- **Network.** The transport authenticates and signs frames but does not encrypt: bind to
  a private interface (WireGuard, Tailscale) or tunnel it.
  A message sent while a turn runs waits for the turn to end, as ACP has it (one prompt at a time);
  the agent's own way of working is never interrupted. A harness with `"steer": true` lets the
  message in sooner: at the next tool boundary the core cancels the step (the agent keeps what it
  did) and sends the message to the same session, journaling a `steered` checkpoint. Off by
  default; worth it for an agent that takes a cancel well (Claude Code through claude-agent-acp).

## Interface options (hub with a gateway)

```json
"gateway_ui": {"footer": "<text shown at the bottom of the sidebar>", "brand": "<name shown next to Inari in the top bar>",
               "owner": "<the owner's name, in a pass holder's banner>"},
"mascot_dir": "<folder holding a FoxBot-style sprite engine: index.html, js/, sprites/>",
"usage": [
  {"kind": "claude"},
  {"kind": "static", "items": [{"provider_id": "local", "name": "Local models", "status": "unlimited"}]}
]
```

- **Mascot.** `mascot_dir` is served at `/fox/` and driven through the engine's `window.fox`
  contract (activity working/waiting/idle, harness persona, health, notify events). Without it
  the interface falls back to a canvas orb. No sprite is shipped in this repository.
- **Quotas.** `claude` reads Claude's own usage endpoint with the machine's Claude Code login
  (five-hour and weekly windows, extra usage paid per token); `static` lists providers by hand;
  `cloudroom` reads a Cloudroom-compatible usage service, for interoperability. Sources are polled every minute and normalised to `{provider_id, name, status,
  plan, windows: [{label, used_percent, resets_at}]}`. The sidebar pill and the model chip show a
  ring for the family of the model in use; a provider logo is proxied from its source when it has
  one, otherwise a neutral family mark is drawn.
- **Models.** Each node lists the models its harness (ACP `session/new`) or provider (`/models`)
  offers and announces them; the picker in the composer reads that list. Choosing one is stored
  on the thread, journaled as a `provider_switch`, and applied on the next turn.

## Bus subjects a node answers

| Subject | Effect |
|---|---|
| `agent.create` `{thread_id, node, title}` | create an agent on that node, reply `agent.created` |
| `agent.<id>.run` `{text, attachments?}` | one user turn on the holder; publishes `agent.<id>.event` as it goes and `agent.<id>.done`. `attachments` lists workspace paths sent with the message (Rust node) |
| `agent.upload.request` `{agent, path, b64, reply}` | put a file in the agent's workspace |
| `agent.files.request`, `agent.file.request`, `agent.exec.request` | list, read, or run a command in the workspace; the reply goes to `reply` |
| `agent.<id>.model`, `agent.<id>.harness` | choose the thread's model or harness |
| `agent.<id>.fork` `{new_thread, upto}` | fork on the holder |
| `agent.merge` `{target, source}` | union two journals on the same holder |
| `agent.history.request` `{agent}` | the holder replies `agent.<id>.history` |

Long work runs off the bus: a handler that blocks stalls the link reader, and an agent
waiting for an approval would be waiting for the very frame it blocks (found by a test).

## Files from the phone

`POST /api/upload` takes `{agent, path, b64}` for a small file, or the same with
`upload_id`, `offset` and `total` for one part of a larger one (the page sends 512 KiB
per part, up to 25 MB). Each part must start where the previous one ended; a part sent
twice is ignored. When the last part arrives the file goes into the agent's workspace,
and `POST /api/message` with `attachments: [path, …]` sends it to the model with the
text (Rust node): images as image content to agents that accept it, other files as
links into the workspace.

## Harness updates (Rust node)

A harness may say where it comes from:

```json
"claude": {"command": ["<path to claude-agent-acp>"],
           "update": {"npm": "@zed-industries/claude-agent-acp", "prefix": "<npm global prefix>"}},
"openfox": {"kind": "openfox",
            "update": {"npm": "openfox", "restart": ["systemctl", "--user", "restart", "openfox.service"]}}
```

The node reads the installed version from `<prefix>/lib/node_modules/<package>/package.json`,
asks `npm view` for the latest at start and every six hours, and announces both in its
capabilities (`updates`). `POST /api/update {node, harness}` installs the latest version
(`npm install -g --prefix …`) and then runs `restart`, if any; `POST /api/update/check`
asks every node to look again. Progress is published on `node.<id>.update.state` and shown
in the page's update badge. An ACP agent is started for each turn, so the next message
already runs the new version; `restart` is for a server the harness talks to.

## Attaching a new core

The hub takes invitations when `"enroll_port"` is set; it announces the address in its
capabilities (`enroll`). In the interface, **+** next to the cores, then *Attach a core you
deployed*, gives a one-time code valid 30 minutes and the command to run on the new machine
(a VPS, a new VM, another workstation), once its `inari-core-rs` binary is there:

    inari-core-rs join <hub address>:<enroll port> <code> --service

It proves it holds the code (the transport's handshake is keyed by it), receives the mesh
and approval keys and where the bus is, writes `~/.config/inari-core/node.json` (its own
token, the harnesses it finds, Claude quotas when Claude Code is logged in), and with
`--service` installs and starts a user service. The new core joins through the private
network the cores share (Tailscale, WireGuard, an SSH tunnel): the keys travel on it, as the
bus does, so the hub's bus and enroll ports must be reachable there and nowhere else.

## Core passes: family supervision and collaboration

A pass lets a person who is not the core's owner have threads on it. **+** next to the cores,
then *Core pass for a person*: their name, the role, the harnesses their threads may use, and
an optional end date. The page shows the pass once (a code and a link); the person enters it
in the Inari app or a browser.

- **Supervision** (a child, say): the owner reads every thread of the pass and answers the
  agents' permission requests; local models are offered by default. The person always sees
  a banner saying so — supervision is stated, never hidden.
- **Collaboration** (another adult): both see the shared threads and act on them.

Whatever the role, a pass holder sees only the pass's threads (named `<pass id>-…`), the
cores, and the allowed harnesses; no terminal, no teleport, no updates, no quotas, no other
pass. A pass presented never falls back to the owner's access. The hub keeps the passes in
`passes.json` (each known by the hash of its token) and tells every gateway; revoking ends
the access at once, the threads stay. `gateway_ui.owner` names the owner in the banner.
