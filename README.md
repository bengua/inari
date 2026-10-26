# inari-core

A distributed agent core built around one idea: **an agent's state is a portable
file, not a row in someone's database.** Everything else follows from it — hot
provider switching, moving a running agent between machines, a phone that
supervises without computing, nodes that keep working when the network does not.

It is a clean-slate design (see [ADR 0001](docs/adr/0001-clean-break.md)), not a
patch of an earlier engine. This repository holds the foundation: the capsule
format, the handover engine, the teleportation protocol, the mesh, and the
specification of the interfaces that sit on top.

## What is here

| Piece | Where | State |
|---|---|---|
| Session State Capsule (SSC) format, schemas, verifier | `src/inari_core/capsule`, `schema/`, [docs/ssc.md](docs/ssc.md) | implemented, tested |
| Provider handover (neutral journal → Anthropic / OpenAI shapes, compaction) | `src/inari_core/handover` | implemented, tested |
| Teleportation: two-phase commit with a fencing epoch, rollback, in-doubt resolution | `src/inari_core/teleport`, [docs/teleport.md](docs/teleport.md) | implemented, tested with injected faults |
| Mesh: embedded bus, durable links, discovery, coordinator, telemetry → mascot | `src/inari_core/mesh`, [docs/mesh.md](docs/mesh.md) | implemented, tested |
| BYOK secrets, signed one-gesture approvals | `secrets.py`, `approvals.py`, [docs/security.md](docs/security.md) | implemented, tested |
| Desktop spatial graph and mobile client | [docs/ux.md](docs/ux.md) | specification only, on purpose |
| Authenticated TCP transport (bus links, remote endpoints), sandbox runtimes | `src/inari_core/transport`, `sandbox.py` | implemented, tested on loopback |
| Capsule signatures (Ed25519), approvals over the bus | `signature.py`, `approval_bus.py` | implemented, tested |
| NATS / Zenoh adapters | — | not written: same two seams (`Link`, `Endpoint`) |

Read [docs/architecture.md](docs/architecture.md) first.

## Try it

    uv sync --all-extras
    uv run pytest
    uv run inari-core schema schema      # regenerate the JSON Schemas
    uv run inari-core verify  some.ssc   # check any capsule, directory or archive
    uv run inari-core inspect some.ssc

A capsule is a plain tar of JSON and Markdown files: `tar xf some.ssc` and read it
with any editor.

## Design rules

1. **Crash-only.** Every write is atomic (temp file then rename); every operation
   can be repeated; recovery is "start again and read the disk".
2. **One holder.** An agent runs on exactly one node. A fencing epoch, not a
   timeout, decides who — so a partition can freeze an agent but never run it twice.
3. **Append-only history.** The journal only grows and merges by set union, so
   replicas that diverged offline reconcile without conflicts.
4. **No secrets at rest.** A capsule names the secrets it needs; the host's
   keychain supplies them for one run.
5. **Nothing site-specific in the code.** No hostnames, addresses or accounts:
   everything is configuration.

## License

MIT. See [LICENSE](LICENSE).
