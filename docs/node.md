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
