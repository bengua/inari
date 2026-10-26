# UX/UI specification

Status: specified and implemented as one responsive page (`src/inari_core/web/index.html`,
no build step, no external resource) served by `inari-core gateway`. Try it with
`inari-core demo` (a simulated mesh; every drag performs a real teleportation).
What the page does today: the node-and-agent graph with drag-to-teleport, the
journal as a vertical tree, the live orb driven by `mascot_state`, signed
approvals, a target sheet for touch (long-press, or `T` on the keyboard), and
reduced-motion support. Not yet built: fork and merge gestures, search, the
biometric gate (the gateway holds the approval key; browsers use a bearer token),
and an installable shell. The sections below remain the contract.

## Principles

1. **The graph, not the chat log.** A conversation is a tree of decisions on a
   machine, not a scroll of messages.
2. **Place means machine.** Where an agent sits says where it runs.
3. **The creature is an instrument.** Every visual parameter comes from telemetry.
4. **One gesture, one meaning.** Dragging moves; tapping opens; long-press branches.
5. **Calm by default.** Motion encodes information or it is absent.

## Tokens

| Token | Value | Use |
|---|---|---|
| `--bg-0` / `--bg-1` | `#060913` / `#0f172a` | page / panels |
| `--glass` | `rgba(255,255,255,.06)` + 24px backdrop blur + 1px inner `rgba(255,255,255,.12)` | surfaces |
| `--idle` | `#22d3ee` (cyan) | resting network |
| `--listen` | `#e879f9` (magenta) | an agent is receiving input |
| `--infer` | `#fbbf24` (amber) | a model is computing |
| `--ok` | `#34d399` (emerald) | success |
| `--err` | `#f87171` | failure (not in the brief; a failure needs a colour) |

Colour is never the only signal: every state also has a shape or label (below).

## Desktop — the spatial graph

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ◉ orb (live)    mesh: 3 nodes · 2 agents busy            ⌕  ⚙            │
├────────────┬─────────────────────────────────────────────┬───────────────┤
│ Nodes      │                                             │ Inspector     │
│ ● node-a   │        ┌─────────── node-a ───────────┐     │ thread: ship  │
│   heavy    │        │       ◍ agent-1 (amber)      │     │ holder: a @2  │
│   gpu 82%  │        │      ╱╲                      │     │ ─────────────│
│ ● node-b   │        │   ○──●──○ ← branch points    │     │ timeline      │
│   heavy    │        └───────────────┬──────────────┘     │ ▸ decision    │
│   gpu  4%  │              drag ⟶    │                    │ ▸ tool bash   │
│ ◌ phone    │        ┌─────────── node-b ───────────┐     │ ▸ result ok   │
│   mobile   │        │       ◍ agent-2 (cyan)       │     │ ─────────────│
│            │        └──────────────────────────────┘     │ [fork] [pause]│
└────────────┴─────────────────────────────────────────────┴───────────────┘
```

- **Nodes** are large translucent regions; their size follows capacity, their
  border brightness follows load.
- **Agents** are pastilles inside the node that holds them. Ring colour = phase
  (`idle/listening/inference/success/error`); a glyph repeats it (`·`, `◐`, `⟳`,
  `✓`, `!`).
- **Threads** open as a decision tree to the right of the agent: nodes are events
  of kind `decision`, `tool_call` and `user_message`; branches are forks
  (`lineage.fork_event_id`); a diamond marks a `checkpoint`. The current head
  pulses.
- **Inspector** lists the journal as a flat, searchable list *for reading*; the
  graph is for navigating.

### Interactions

| Gesture | Effect |
|---|---|
| Tap an agent | open its tree, focus the inspector |
| Drag an agent onto another node | **teleport** (below) |
| Long-press a tree node | fork from that event (new capsule, `lineage` set) |
| Drag a branch onto another | propose a merge (journals union; working memory chosen by the user) |
| Space | pause/resume the focused agent |
| `/` | search the journal and the vector index |

### Teleport interaction

1. Drag starts: valid targets (alive, `heavy`) glow; others dim. Dropping elsewhere
   cancels with no side effect.
2. Drop: the pastille becomes a **ghost** at the source (frozen, outlined) and a
   **dotted tether** draws to the target. This is `FROZEN → OFFERED`.
3. Transfer: the tether fills as bytes arrive; a caption shows `n files · m blobs
   already there`. Then `VERIFIED`, `READY`.
4. Commit: the ghost collapses into the target and the ring colour returns.
5. Failure: the pastille snaps back to the source and the tether turns red with
   the reason (`digest mismatch`, `stale epoch`…). Nothing is lost; the source
   resumed.
6. In doubt: both ends show a hatched outline and "waiting to confirm"; no action
   is offered except *Retry check*. It is never shown as success or failure until
   the epoch says which.

Every state shown comes from `teleport.result` and the machine's `history`.

## The mascot

An organic orb on a 2D canvas (WebGL optional), the same creature on desktop and
phone. Parameters come straight from `mascot_state(snapshot)`:

| Parameter | Driven by | Rendering |
|---|---|---|
| colour | highest-priority agent phase | core and halo hue |
| `pulse_hz` | peak GPU load | heartbeat scale oscillation, 0.25–2 Hz |
| `turbulence` | network packets/s | amplitude of the surface noise |
| `glow` | number of busy agents | halo radius and opacity |

`prefers-reduced-motion`: pulse and turbulence are replaced by a static ring whose
thickness encodes load, and the phase glyph stays.

## Phone — thin client

No model runs on the phone. It renders the mascot, a dashboard, approvals and a
remote.

```
┌─────────────────────────┐
│                         │
│          ◉              │   ← the orb, full-width, live
│       (amber, fast)     │
│                         │
│  2 agents busy · 3 nodes│
├─────────────────────────┤
│ ◍ agent-1    node-a  ⟳  │
│ ◍ agent-2    node-b  ·  │
├─────────────────────────┤
│ ⚠ Approve?              │
│ sudo reboot             │
│ agent-1 · node-a · 0:42 │
│ [  Deny  ] [ Approve ⌁ ]│  ← biometric gate
└─────────────────────────┘
```

- **Home** is the orb. Pulling down reveals the node list; tapping a node shows its
  agents.
- **Approval** is a notification and a card. *Approve* requires biometrics, which
  unlocks the device key that signs the response. The card counts down to the
  request's expiry; at zero it disappears and the answer is *no*.
- **Remote teleport**: long-press an agent, then tap the destination node in the
  sheet that lists valid targets (a precise alternative to dragging on a small
  screen). The request is published on `teleport.request`; if the phone is offline
  it queues and is sent on reconnect, with a visible "queued" badge.
- **Offline**: the last known state stays on screen, dimmed, with its age. Nothing
  on the phone is needed for agents to keep running.

## Data contracts

| View | Reads | Writes |
|---|---|---|
| Orb, dashboard | `agent.*.state`, `node.*.load`, `mesh.nodes` | — |
| Graph | capsule files via the node's API; schemas in `schema/` | — |
| Teleport | `teleport.result` | `teleport.request` |
| Approvals | `ApprovalRequest` | `ApprovalResponse` |

## Accessibility

Contrast of text on glass ≥ 4.5:1 (the blur is capped so it cannot lower it);
targets ≥ 44 px on the phone; full keyboard path on desktop (arrow keys move
between agents, Enter opens, `T` starts a teleport and the arrow keys choose the
target); screen-reader text for every state change ("agent-1 moved to node-b").

## Out of scope here

Visual design assets, the renderer, the graph layout algorithm (force-directed with
node regions as constraints is the intended start) and the mobile application
shell.
