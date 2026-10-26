# Mesh

## Bus

An embedded, in-process bus with NATS-style subjects: tokens separated by dots,
`*` for one token, `>` for the rest (`agent.*.state`, `teleport.>`).

- **Isolation.** A handler that raises is counted (`handler_errors`) and the others
  still run.
- **Links.** `connect(a, b)` joins two buses. Each message records the buses it
  crossed, so a ring of three buses delivers once and stops.
- **Offline-first.** While a link is down, messages on *durable* subjects
  (`teleport.>`, `approval.>`, `agent.>` by default) wait in a bounded outbox and
  replay on reconnect. Receivers keep a bounded set of seen message ids, so a
  replay is never delivered twice. Non-durable subjects (load, heartbeats) are
  dropped: the next sample supersedes them.
- **Transport.** The bus does not care what is under a link. The intended
  deployments are an authenticated socket over a private network (WireGuard,
  Tailscale) or a NATS/Zenoh leaf; both implement `Link.send` and call
  `Bus.deliver` on receipt.

## Discovery

Nodes publish `node.announce` once and `node.heartbeat` periodically with
`{node_id, role, incarnation, capabilities}`. Roles: `heavy` (runs sandboxes and
models), `control` (a desk), `mobile` (thin client).

`Registry` keeps the live nodes (default 15 s TTL), ignores announcements from an
older `incarnation` (a node that restarted), and `merge`s snapshots from peers, so
every node converges on one view without a leader.

## Coordinator

`MeshCoordinator` is a set of subscriptions:

| Subject in | Effect |
|---|---|
| `node.announce`, `node.heartbeat` | update the registry; publish `mesh.nodes` on change |
| `agent.<id>.state` | telemetry for dashboards and the mascot |
| `node.<id>.load` | GPU and traffic figures |
| `teleport.request` | validate, run the injected `teleporter`, publish `teleport.result` |

A request is refused (with a reason, never silently) when a node is not alive, when
source and target are the same, when the target is not `heavy`, or when that agent
is already moving. `sweep()` publishes `node.lost` for silent nodes.

The coordinator keeps nothing a restart cannot rebuild from the next round of
heartbeats.

## Telemetry to mascot

`mascot_state(snapshot)` is a pure function:

| Input | Output |
|---|---|
| any agent in `error` | red; otherwise highest of inference (amber), listening (magenta), success (emerald); idle is cyan |
| highest GPU load `g` over nodes | `pulse_hz = 0.25 + 1.75·g` |
| total packets per second `p` | `turbulence = min(1, p/2000)` |
| number of non-idle agents `k` | `glow = min(1, 0.2 + 0.2·k)` |

Same snapshot, same creature: the mascot cannot lie, and a renderer can be tested
without a GPU.

## Not here yet

Real transports, authentication of links, and the sandbox runtime. They plug in at
`Link` and `Endpoint`; the tests use in-memory implementations of both.
