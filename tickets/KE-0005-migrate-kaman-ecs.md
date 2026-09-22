# KE-0005 — Migrate ecs → `kaman-ecs`

Phase:         0
Priority:      P0
Status:        Done
Integration:   Reuse-as-is
Size:          S · A1
Time:          S
Risk:          Low
Depends on:    KE-0003      Blocks: KE-0101 (Phase 1 boundary), Phase 2 gameplay
Serves:        KR0.3

## Problem / Motivation
Migrate the `hecs`-based component definitions and helpers. This crate becomes the stable API
that the `Game` trait (Phase 1) and KamanScript (Phase 5) bind against, so its public surface
matters more than most.

## Scope & Acceptance
- [x] **Move commit:** relocate `ecs.rs` into `crates/kaman-ecs`; depend on `kaman-math`; re-export `hecs`.
- [x] Keep components engine-generic only — **no game types** (no car/road/score). Verify none leaked in.
- [x] Public API: component types (`Transform`, `Render*`, physics-handle, static/dynamic tags),
      mesh-shape generation helpers as present.
- [x] `#[non_exhaustive]` on component enums where future variants are expected, to protect the
      A1 boundary from becoming an accidental A2 later.
- [x] Unit tests: spawn/query round-trips; component (de)composition invariants.

## Technical notes
- This is Reuse-as-is but marked A1: tightening visibility / `#[non_exhaustive]` reshapes the
  public shape slightly. Guard with this crate's own spawn/query unit tests.

## Out of scope
- Streaming spawn/despawn semantics (Phase 2, KE-0203). Physics handle removal (KE-0202).

## Test gate
`cargo test -p kaman-ecs` green; oracle green; a test asserts no game-named symbols exist.

## Doc gate
`#![deny(missing_docs)]`; crate `README` documents the component model as the KamanScript/Game
binding surface; invariants (e.g. handle ownership) stated in doc + enforced by a test.
