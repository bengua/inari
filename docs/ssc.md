# Session State Capsule (SSC) — format `ssc/1`

A capsule is a directory (or the tar of one) that fully describes an agent thread.
It is meant to be read by people: every file is JSON, JSONL or Markdown.

```
manifest.json                      identity, lineage, access, holder, digests
memory/working_memory.md           the agent's own synthesized scratchpad
memory/timeline_events.jsonl       immutable journal, one event per line
memory/vector_index/index.json     local semantic index (embedder recorded)
artifacts/snapshot.json            workspace tree: path -> content hash
artifacts/blobs/<sha256>           content-addressed file bodies
environment/spec.json              reproducible sandbox definition
```

Authoritative schemas: `schema/*.schema.json`, generated from the Pydantic models
in `src/inari_core/capsule/models.py`. A test fails if they drift.

## manifest.json

| Field | Meaning |
|---|---|
| `format` | always `"ssc/1"`; a reader refuses anything else |
| `capsule_id` | UUID of this capsule |
| `thread_id` | the agent's identity; stays the same across moves |
| `lineage` | `parent_capsule`, `fork_event_id`, `merged_from[]`: the genealogy of forks and merges |
| `access` | `owners[]`, `readers[]`, `approval_required[]` (glob patterns over commands) |
| `holder` | `{node_id, epoch}`: who may run it, with the fencing epoch |
| `timeline_head` | fingerprint of the whole journal (order independent) |
| `files` | every other file's `{sha256, size}` — the manifest does not digest itself |

`verify` reports: missing, undeclared and altered files; a journal whose hashes,
chains or fingerprint do not match; snapshot entries whose blob is absent.

## Journal events

```json
{"id":"node-a:3","node_id":"node-a","node_seq":3,"lamport":3,"ts":3.0,
 "kind":"tool_result","data":{"call_id":"c1","output":"a.txt","is_error":false},
 "prev":"<hash of node-a:2>","hash":"<sha256 of this event without 'hash'>"}
```

- `kind`: `system`, `user_message`, `assistant_message`, `thought`, `decision`,
  `tool_call`, `tool_result`, `provider_switch`, `checkpoint`.
- Integrity is **per node**: `prev` chains the events one node wrote. Order across
  nodes comes from `lamport`, ties broken by `(node_id, node_seq)`.
- Two events with the same `id` and different `hash` are an equivocation and are
  refused (`ConflictError`).
- Merging is set union: commutative, associative, idempotent.

## Workspace snapshots

`snapshot.json` maps relative paths to blob hashes. A move transfers only the
blobs the target's store lacks. Restoring refuses any path that escapes the target
directory.

## environment/spec.json

Image, working directory, mounts (a logical volume name, never a host path),
non-secret `env`, `secret_refs` (names only) and resource hints. The model
**rejects** an `env` key that looks like a secret (`KEY`, `TOKEN`, `SECRET`,
`PASSWORD`, `CREDENTIAL`): such values go through `secret_refs`.

## Archive

`inari-core pack` writes a deterministic tar (sorted entries, zero mtime and
owner): the same capsule gives the same bytes, so archives can be compared and
deduplicated. `unpack` refuses links, absolute paths and `..`.

## Compatibility

`ssc/1` readers must ignore unknown JSON fields and refuse a different `format`.
Additive changes keep the version; anything that changes a digest rule bumps it.
