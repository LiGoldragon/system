# System — Agent Instructions

## Purpose

`system` defines the portable system boundary for Persona: window
identity, focus events, and privileged system adapters. Backend
code starts here only while there is a single target system; split a backend
into its own repository once the second backend makes the common interface
concrete.

## Local Rules

- Use Jujutsu for version control.
- Keep repositories public unless the human gives a specific reason otherwise.
- Use Nix for build and test entry points.
- Use Rust 2024 and keep verbs on data-bearing objects.
- No polling. Subscribe to system events or defer work until a producer pushes
  the next signal.
- State storage is `system.sema` through `sema-engine` when this crate owns
  durable state. This crate should usually define system contracts, not own
  router state.

## Protos estate status

Stack: correct-new destination
Status: active component, current checkout legacy-wired
This checkout is not proof of correct-new adoption.
