# ADR 0001 — A clean break from the previous core

Status: accepted.

## Context

The previous engine kept thread state in a central database, shared mutable state
between a server and its host daemons, and let background work run on timers that
read that state. Under load or after a restart its failures were hard to explain:
a scheduler that re-armed itself at a delay of zero, tasks killed halfway through
a tool call, threads that could no longer be opened. Fixing symptoms repeatedly
cost more than the engine was worth.

## Decision

Start a new repository with a different foundation instead of patching:

- State is a **self-contained capsule**, not rows. Moving, forking, backing up and
  inspecting an agent are file operations.
- There is **no shared mutable state** between nodes. Nodes exchange immutable
  events and capsules; the only mutable cross-node fact is the fencing epoch.
- **Crash-only** components: no shutdown path to get wrong, no in-memory state a
  restart cannot rebuild.
- Providers are **adapters over a neutral journal**, so a model switch is a
  re-rendering, not a migration.

## Consequences

- The earlier engine keeps running until this one can replace it; nothing here
  imports from it.
- Costs: file-level (not block-level) deduplication of workspaces; a journal that
  must be compacted for small context windows; a protocol (teleportation) that
  has to be correct under partitions, which is why it ships with fault injection.
