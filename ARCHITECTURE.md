# system — architecture

*Portable OS, window-manager, and focus-observation boundary.*

`system` names what Persona needs from the operating system without
forcing router or harness code to know about Niri, Wayland, macOS, or any other
backend.

> **Scope.** System has no durable store today. If it later owns durable
> subscription registrations, backend cursors, or adapter state, that state
> lives in `system.sema` opened through `sema-engine`; the daemon does not
> touch raw redb or another component's Sema store.

## 0 · TL;DR

This repo owns system observations as pushed events and privileged OS actions
as a separately-gated surface. It does not decide routing policy and it does
not move terminal bytes.

```mermaid
flowchart LR
    "Niri backend" -->|"focus events"| "FocusTracker"
    "FocusTracker" -->|"observation frame"| "signal-system"
    "signal-system" -->|"pushed observation"| "router"
```

## 1 · Component Surface

`system` exposes:

- typed target identity for windows, panes, and harness surfaces;
- focus-state observations;
- a `system` CLI for one-shot focus probes and focus subscriptions;
- a `system-daemon` socket skeleton for the first Persona stack;
- a Niri focus source backed by `niri msg --json windows` and
  `niri msg --json event-stream`;
- a Kameo `FocusTracker` that owns subscription focus state while the event
  stream is running;
- privileged action records such as force-focus and focus-drift suppression;
- event subscription surfaces for consumers;
- backend adapter traits or data-bearing adapter objects.

## 1.5 · Paused-state skeleton

system is **paused** — domain-level focus work waits on a real
consumer (window-focus-aware notifications, multi-engine UI, multi-monitor
layout). The daemon still comes up as a supervised first-stack component so
the prototype's "all six daemons ready" witness passes, and the FocusTracker
exists today as a real Kameo actor (state-bearing, message-driven, not a
marker) ready for the Niri event-stream path that activates on unpause.

The daemon runs on the **schema-emitted daemon shell** (`schema-rust-next`
`NexusDaemonShape` + `GenerationDriver`, mirrored on `orchestrate`). The
emitter owns argv parsing, async task-backed multi-listener binding, request
gating, peer credentials, lifecycle, and the `ExitReport` entry in
`src/schema/daemon.rs`; system hand-writes only the `ComponentDaemon` hook
surface in `src/daemon.rs`. system adopts the **`component_decoded` working
tier**: the ordinary `system.sock` keeps speaking the `signal-system` contract
`SystemFrame` (the component owns the per-connection decode) wrapped in the
emitted length-prefixed envelope, and the owner-only **meta** listener carries
the engine-management supervision protocol. The existing Kameo actors stay the
engine, shared as `&Self::Engine` and lazily started inside the daemon's tokio
runtime; their mailboxes serialise every exchange with no component-internal
lock.

The component skeleton is honest:

1. The daemon reads exactly one signal-encoded/rkyv
   `SystemDaemonConfiguration` file at startup, rejects inline NOTA and
   `.nota` startup files, and binds `system.sock` at the managed socket mode
   carried by that record — all through the emitted `DaemonCommand` /
   `DaemonBinder` and system's `Configuration: triad_runtime::DaemonConfiguration`.
2. The daemon answers engine-management supervision requests
   (`signal-persona::Operation`, the consolidated supervision contract) from a
   `SupervisionPhase` actor over the owner-only meta socket — `ComponentReady {
   component_started_at }` once the socket is bound;
   `ComponentHealthReport { health: Running }`.
3. The daemon returns `SystemReply::SystemRequestUnimplemented` for every
   unbuilt domain request (`WatchFocus`, `UnwatchFocus`, `QueryFocus`). The
   contract decodes each variant; the reply is typed and closed, never a hang
   or untyped text error.
4. `FocusTracker` is a real Kameo actor with `target`, `id`, `last`,
   `generations`, `workspace_id`, `synthetic_generation`,
   `applied_event_count`, and `emitted_observation_count` state. Niri
   event application goes through its message handler, not direct method
   calls. It is exercised in tests today; it is not wired into a
   supervised long-lived runtime yet.

**Deferred until a real consumer surfaces:**

- The Niri event-stream push path activates and routes observations into
  consumer subscriptions.
- The `SystemPrivilegedRequest` surface — `ForceFocus`, `SuppressDrift` —
  lands. Today it is named in design but has no code and no consumer
  asking for it.
- The privileged-vs-observation authorization boundary (which connection
  class can request which actions) is settled when the first privileged
  consumer concretizes the requirement.

**Subscription lifecycle applies when unpausing.** Focus subscription close
follows the canonical five-state FSM named in
`~/primary/skills/subscription-lifecycle.md`: typed
`WatchFocus(FocusSubscription)` request returns a typed
`SubscriptionAccepted` snapshot reply carrying the per-stream
`FocusSubscriptionToken`; the producer pushes typed `FocusObservation` /
`WindowClosed` events on the `FocusEventStream`; the consumer sends a typed
`UnwatchFocus(FocusSubscriptionToken)` request to
close; the producer emits a final `SubscriptionRetracted` reply carrying
the same token; the stream ends. Both the request-side close operation and
the reply-side acknowledgement are first-class. The `signal_channel!`
macro at `signal-system/src/lib.rs:303-330` enforces the pairing:
the `close UnwatchFocus` line in the stream block emits
the typed `closed_stream()` discriminant. Raw socket close is not
semantic protocol.

**Naming reopens on unpause.** `ForceFocus` is a negative name (states what
the action overrides, not what it is). Before any privileged-action code
lands, the verb is reframed positively per the workspace naming discipline —
the rename happens when the first consumer concretizes the authority and
effect the action carries.

## 2 · State and Ownership

The component owns observations and subscriptions. Backend adapters may keep
backend-specific handles, sockets, or registration state. A live Niri
subscription keeps `FocusTracker` as the data-bearing actor; compositor events
enter through that mailbox before any Persona observation is emitted.

Read-only observations and privileged actions are separate surfaces. Focus
state is an observation that consumers subscribe to. Force-focus and
focus-drift suppression are privileged actions; they require manager-created
system authority. A non-privileged connection may observe permitted state but
cannot request an OS-level action.

Prompt cleanliness, typed write leases, and programmatic write injection are not
system observations in the current stack. They are terminal transport facts
owned by `terminal` and `terminal-cell` through the
`signal-terminal` contract.

Durable consumer history is not owned here; consumers that need history persist
it through their own Sema database. If `system` later needs durable
subscription registrations, backend cursors, or adapter state, it owns a
system-scoped Sema database through `sema-engine` for that state rather than
writing into another component's database.

## 3 · Boundaries

This repo owns:

- system runtime behavior for portable targets defined by
  `signal-system`;
- pushed focus event surfaces;
- backend abstraction for Niri and later OS ports.

This repo does not own:

- delivery decisions (`router`);
- harness lifecycle (`harness`);
- terminal PTY transport (`terminal`);
- system frame definitions (`signal-system`);
- terminal prompt and input-gate contracts (`signal-terminal`);
- durable transaction ordering for consumers.
- any other component's Sema database.

`signal-system` owns the contract types, their rkyv wire derives, and
their NOTA text derives. `system` consumes those records directly; it
does not define local mirror records for `SystemTarget`, `FocusObservation`,
or `SystemRequest`.

## 4 · Invariants

- Producers push events; consumers subscribe.
- The daemon accepts the `signal-system` frame boundary.
- The daemon applies the managed spawn-envelope socket mode to `system.sock`
  before accepting client traffic.
- The daemon answers `QueryStatus(SystemStatusQuery)` with typed health and readiness.
- A recognized but unbuilt daemon request returns
  `SystemRequestUnimplemented`; it must not hang or print an untyped text
  error.
- `system` must not duplicate contract-owned records.
- Backend-specific details stay behind data-bearing adapter objects.
- Privileged actions are not observations; they require the persona daemon's
  system connection class.
- Live subscription state belongs to Kameo actors, not loose shared objects.
- Niri window id is the first real target key; title, app id, and pid are
  evidence, not identity.
- The router receives observations and decides policy.
- Unknown system state is explicit typed state, not a reason to poll.
- System-owned durability, when present, is limited to subscription/backend
  state and emits observations only after commit.
- Prompt cleanliness is terminal-owned, not system-owned.

## Code Map

```text
src/command.rs       NOTA CLI command surface over `SystemRequest`
src/configuration.rs `Configuration` over `signal-system::SystemDaemonConfiguration`,
                     `impl triad_runtime::DaemonConfiguration`
src/daemon.rs        the hand-written `ComponentDaemon` hook surface — engine
                     wiring + the component_decoded working/meta connection hooks
src/schema/daemon.rs schema-emitted daemon shell (argv, listeners, lifecycle)
src/supervisor.rs    `SystemSupervisor` Kameo actor + `SystemRequestHandler`
src/supervision.rs   `SupervisionPhase` engine-management actor + frame decode
src/event.rs         local focus-state helpers only
src/niri.rs          Niri focus snapshot and event-stream adapter (serde_json IPC)
src/niri_focus.rs    Kameo mailbox implementation for `FocusTracker`
src/target.rs        local harness-to-system-target helper
schema/nexus.schema  no-stream nexus schema the daemon emitter reads
tests/               smoke, daemon, and actor-runtime constraint tests
```

## Constraint Witnesses

| Constraint | Nix-visible witness |
|---|---|
| The daemon applies the managed spawn-envelope socket mode. | `checks.<system>.system-daemon-applies-spawn-envelope-socket-mode` |
| The daemon answers typed health/readiness. | `checks.<system>.system-daemon-answers-status-readiness` |
| The daemon returns typed unimplemented for unfinished recognized requests. | `checks.<system>.system-daemon-returns-typed-unimplemented` |
| Niri subscriptions pass events through the Kameo mailbox. | `tests/actor_runtime_truth.rs` |
| `system` does not own terminal prompt-gate vocabulary. | `tests/smoke.rs` |

## See Also

- `../router/ARCHITECTURE.md`
- `../harness/ARCHITECTURE.md`
- `../orchestrate/ARCHITECTURE.md` — the daemon-shell port template
- `../signal-persona/ARCHITECTURE.md` — the engine-management supervision contract
- `../signal-system/ARCHITECTURE.md`
