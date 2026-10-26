# Running a node

`inari-core node --config node.json` starts one node: it holds agents, serves its
capsules to peers, runs the agent loop, and joins the bus. A node without `hub` is the
hub: it also runs the coordinator and, with `gateway_listen`, the web interface.

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
- **Provider kinds.** `openai` (any compatible gateway), `anthropic`, and `demo` (rules
  only, no network).
- **Sandbox.** `"sandbox": "subprocess"` (default) or `"container"` with `"engine"`.
- **Network.** The transport authenticates and signs frames but does not encrypt: bind to
  a private interface (WireGuard, Tailscale) or tunnel it.

## Interface options (hub with a gateway)

```json
"gateway_ui": {"footer": "<text shown at the bottom of the sidebar>"},
"mascot_dir": "<folder holding a FoxBot-style sprite engine: index.html, js/, sprites/>",
"usage": [
  {"kind": "cloudroom", "url": "<base URL of a Cloudroom-compatible server>", "providers": ["claude-code", "codex"]},
  {"kind": "static", "items": [{"provider_id": "local", "name": "Local models", "status": "unlimited"}]}
]
```

- **Mascot.** `mascot_dir` is served at `/fox/` and driven through the engine's `window.fox`
  contract (activity working/waiting/idle, harness persona, health, notify events). Without it
  the interface falls back to a canvas orb. No sprite is shipped in this repository.
- **Quotas.** Sources are polled every minute and normalised to `{provider_id, name, status,
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
| `agent.<id>.run` `{text}` | one user turn on the holder; publishes `agent.<id>.event` as it goes and `agent.<id>.done` |
| `agent.<id>.fork` `{new_thread, upto}` | fork on the holder |
| `agent.merge` `{target, source}` | union two journals on the same holder |
| `agent.history.request` `{agent}` | the holder replies `agent.<id>.history` |

Long work runs off the bus: a handler that blocks stalls the link reader, and an agent
waiting for an approval would be waiting for the very frame it blocks (found by a test).
