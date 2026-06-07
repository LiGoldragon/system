# system

Portable system boundary for Persona.

This crate defines typed contracts for:

- harness window identity;
- focused-window state;
- pushed focus observations.

The first implementation target is the current Niri-based Persona OS stack.

Prompt cleanliness and programmatic write safety are terminal responsibilities,
owned by `terminal` / `terminal-cell` through `signal-terminal` input gates.

The `system` CLI accepts one NOTA command:

```sh
system '(QueryFocus ((NiriWindow 198)))'
system '(WatchFocus ((NiriWindow 198)))'
```

`QueryFocus` reads `niri msg --json windows` once. `WatchFocus` emits
an initial `FocusObservation` and then follows `niri msg --json event-stream`,
filtering noisy compositor events through the Kameo `FocusTracker` actor that
owns the tracked window state.
