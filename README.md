# system

Portable system boundary for Persona.

This crate defines typed contracts for:

- harness window identity;
- focused-window state;
- pushed focus observations.

The first implementation target is the current Niri-based Persona OS stack.

Prompt cleanliness and programmatic write safety are terminal responsibilities,
owned by `terminal` / `terminal-cell` through `signal-terminal` input gates.

`system` is the ordinary thin Signal client. It accepts one
`signal-system::SystemRequest` as inline NOTA or a NOTA/rkyv file path, sends it
to `SYSTEM_SOCKET` (default `/tmp/system.sock`), and prints the typed
`SystemReply` as NOTA.

```sh
system '(QueryStatus Niri)'
```

`meta-system` is the meta policy client. It accepts one
`meta-signal-system::MetaSystemRequest`, sends it to `SYSTEM_META_SOCKET`
(default `/tmp/meta-system.sock`), and prints the typed meta reply as NOTA.

`system-focus` is the local Niri helper retained from the paused focus skeleton.
It reads `niri msg --json windows` for `QueryFocus`; `WatchFocus` emits an
initial `FocusObservation` and then follows `niri msg --json event-stream`,
filtering noisy compositor events through the Kameo `FocusTracker` actor that
owns the tracked window state.
