# KE-0203 — World streaming: spawn/despawn + origin rebase

Phase:         2
Priority:      P0
Status:        Done
Integration:   New
Size:          L · A2
Time:          L
Risk:          High
Depends on:    KE-0201, KE-0202      Blocks: KE-0204
Serves:        KR2.3

## Problem / Motivation
An endless runner generates world ahead of the player and discards it behind, forever. This ticket
builds `crates/kaman-scene` as the owner of the hecs `World` + `kaman-physics::PhysicsWorld`, plus
**streaming**: spawn-ahead / despawn-behind and a **floating-origin rebase** so world coordinates
never drift into float-precision error. Memory must be flat over long play (KR2.3).

## Scope & Acceptance
- [x] `kaman-scene` owns the `World` + `PhysicsWorld` and exposes an engine-generic streaming API:
      register spawn/despawn *policies* by distance from a focus point (the game supplies the focus
      and the spawn callback; scene owns the bookkeeping). **No game types in `kaman-scene`.**
- [x] **Spawn-ahead / despawn-behind:** entities beyond the despawn threshold are removed —
      ECS entity + its physics body via `remove_body` — **atomically** (KE-0202 invariant), leaving
      no stale handles.
- [x] **Origin rebase:** when the focus passes a threshold, shift all positions (ECS transforms +
      physics bodies) by a fixed offset back toward the origin, transparently to gameplay. A test
      asserts relative positions are preserved across a rebase.
- [x] **No unbounded growth:** a test spawns/despawns over many simulated frames and asserts entity
      count, physics body count, and a capacity proxy stay bounded (stands in for the "10 min, no
      memory growth" KR; the wall-clock soak is validated manually).
- [x] Steps physics at `FIXED_DT` inside the fixed-timestep loop (KE-0201).

## Technical notes
- A2: `kaman-scene` becomes the API the game drives and that KE-0204 builds on; design the
  spawn/despawn/rebase surface deliberately.
- Rebase must move rapier bodies too (set translations) — do it between physics steps to avoid
  solver artifacts; document the ordering invariant.
- Determinism: streaming decisions are a pure function of focus position + policy, so the same run is
  reproducible (needed for any future golden).

## Out of scope
- Game-specific spawn content — lanes/obstacles/pickups live in `car-runner` (KE-0204).
- Asset/mesh streaming from disk (Phase 4). Custom spatial-query layer (post-v1).

## Test gate
`cargo test -p kaman-scene` green: spawn/despawn atomicity (no stale handles), origin-rebase
position preservation, bounded counts over many frames; workspace oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; streaming + rebase invariants (atomic despawn, rebase ordering) in doc +
tests; crate `README` documents the streaming API as engine-generic; ARCHITECTURE notes the model.
