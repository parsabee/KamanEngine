# KE-0003 — Migrate math → `kaman-math`

Phase:         0
Priority:      P0
Status:        Todo
Integration:   Reuse-as-is
Size:          XS · A0
Time:          S
Risk:          Low
Depends on:    KE-0001, KE-0002      Blocks: KE-0005
Serves:        KR0.3

## Problem / Motivation
`math` is a pure, zero-coupling leaf. Migrating it first proves the per-module pipeline
(INTEGRATION §2.3) at the lowest possible risk.

## Scope & Acceptance
- [ ] **Move commit:** relocate the prototype's `math.rs` into `crates/kaman-math` unchanged;
      wire the `glam` dep; fix visibility. No logic change.
- [ ] Oracle + KE-0002 math goldens still green after the move.
- [ ] Public surface re-exports `glam` and exposes `Transform`, `Ray`, `AABB` (as present).
- [ ] Unit tests cover the public API (extend the known-value table as needed) to ≥ baseline.

## Technical notes
- This crate is a dependency of `kaman-ecs`, `kaman-physics`, `kaman-camera`, `kaman-scene`.

## Out of scope
- Adding new math types (do them when a consumer needs them).

## Test gate
`cargo test -p kaman-math` green; workspace oracle green.

## Doc gate
`#![deny(missing_docs)]`; every public item has rustdoc; crate `README` (responsibility +
API tour); at least one runnable doc example.
