# system skill

Work here when the change concerns OS/window-manager abstractions, focus
events, target identity, or backend adapters.

Rules for work here:

- Model observations as typed pushed events.
- Keep `system` and `meta-system` as thin clients over `signal-system` and
  `meta-signal-system`; local direct Niri inspection belongs to `system-focus`.
- `system-daemon` starts from exactly one signal-encoded/rkyv
  `SystemDaemonConfiguration` file. Inline NOTA and `.nota` startup files
  belong to deploy/CLI tooling and are rejected before daemon construction.
- Keep backend handles inside data-bearing adapter objects.
- Keep routing decisions in `router`.
- Keep terminal PTY byte transport in `terminal`.
- Keep prompt cleanliness, input gates, and write-injection safety in
  `terminal` / `terminal-cell`, through `signal-terminal`.
- Use `niri msg --json windows` for current-state focus probes and
  `niri msg --json event-stream` for pushed focus changes. Filter by tracked
  `NiriWindow` id before emitting Persona observations.
- Keep live subscription state in the Kameo `FocusTracker`. Do not bypass
  the actor mailbox when turning event-stream rows into observations.
- Escalate if a backend cannot push the needed event; do not add polling as a
  fallback.

## Escalation when a backend cannot push

The workspace's `skills/push-not-pull.md` is the canonical rule:
producers push state, consumers subscribe. A "poll for now" is never the
right answer — once written, it does not get removed.

When a system-observation backend lacks the subscription primitive the
component needs (an OS surface that only exposes "what is it now?"
queries, a compositor without an event stream, an adapter whose only
read shape is a snapshot), the response is one of these, in order:

1. **Build the primitive in the backend.** If the backend is in scope,
   add the push surface — an event stream, an inotify watch, a Unix
   socket subscriber pattern, a `timerfd` deadline when the contract is
   genuinely "wake me at this deadline." Niri's `event-stream` is the
   shape this component already relies on; other backends earn the same
   shape before they integrate.
2. **Replace the backend.** When the backend cannot be modified, route
   through a different producer that already pushes.
3. **Defer the dependent feature.** Real-time observation waits until a
   push primitive lands. State the deferral explicitly; do not ship a
   poll loop "until the real one is ready."
4. **Escalate.** When (1)–(3) do not resolve the case at hand, surface
   it: a designer report naming the constraint, the backend, the
   missing push primitive, and the dependent feature. The decision —
   build, replace, or defer — belongs upstream.

Escalation is the correct outcome when no push answer is found. It is
not a failure mode; it is the discipline working. The wrong outcome —
falling back to a poll — is never the answer.

The three carve-outs that look polling-shaped but are not (reachability
probes, backpressure-aware pacing, deadline-driven OS timers) live in
`~/primary/ESSENCE.md` §"Polling is forbidden" and
`~/primary/skills/push-not-pull.md` §"The named carve-outs." Anything
outside those three escalates.
