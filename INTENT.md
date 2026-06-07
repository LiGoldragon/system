# INTENT — system

`system` is Persona's portable OS, window-manager, and focus-observation boundary. It
names what Persona needs from the operating system without forcing router or harness code
to know about Niri, Wayland, macOS, or any other backend. It owns system observations as
pushed events and privileged OS actions as a separately-gated surface. It does not decide
routing policy and it does not move terminal bytes. Today's system is a realization step
on the eventually-self-hosting stack: once the OS itself lives in Sema, this OS-boundary
layer goes away.

`system` is **paused** — domain-level focus work waits on a real consumer (window-focus-
aware notifications, a multi-engine UI, multi-monitor layout). The daemon still comes up
as a supervised first-stack component so the prototype's "all six daemons ready" witness
passes, and `FocusTracker` exists today as a real data-bearing Kameo actor ready for the
Niri event-stream path that activates on unpause. The skeleton is honest: the daemon reads
its spawn envelope, binds `system.sock` at the managed socket mode, answers supervision
status/readiness, and returns a typed `SystemRequestUnimplemented` for every unbuilt domain
request rather than hanging or printing untyped text. `system-daemon`
starts from exactly one signal-encoded/rkyv
`SystemDaemonConfiguration` file and rejects inline NOTA and `.nota`
startup files.

Key constraints: producers push events; consumers subscribe — unknown system state is
explicit typed state, never a reason to poll. Read-only observations and privileged actions
are separate surfaces; force-focus and focus-drift suppression require manager-created
system authority, and a non-privileged connection may observe permitted state but cannot
request an OS-level action. `system` must not duplicate contract-owned records:
`signal-system` owns the contract types and their rkyv/NOTA derives, and `system` consumes
them directly with no local mirror types. Backend-specific details stay behind data-bearing
adapter objects. The Niri window id is the first real target key; title, app id, and pid
are evidence, not identity. Live subscription state belongs to Kameo actors, not loose
shared objects. The router receives observations and decides policy. Prompt cleanliness,
typed write leases, and programmatic write injection are NOT system facts — they are
terminal transport facts owned by `terminal` and `terminal-cell`. `system` does not open
any other component's database; any system-owned durability is limited to
subscription/backend state and emits observations only after commit. Negative action names
(`ForceFocus`) are reframed positively before any privileged-action code lands.
