# KE-0202 — rapier wrapper + removal API + use-after-free guard

Phase:         2
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          S · A0
Time:          M
Risk:          Med
Depends on:    KE-0005      Blocks: KE-0203
Serves:        KR2.2

## Problem / Motivation
Migrate the prototype's `physics.rs` (`PhysicsWorld` over rapier3d) into `crates/kaman-physics`,
and add the **body/collider removal API** it lacks today. An infinite runner despawns obstacles
every frame; without removal, bodies leak and stale handles dangle. Per ARCHITECTURE §5, rapier is
retained for v1 behind this wrapper so a later custom arcade-physics layer can replace it without
rippling. INTEGRATION §2.7: the **use-after-free test is written before the removal API**.

## Scope & Acceptance
- [ ] **Move commit:** relocate `physics.rs` into `crates/kaman-physics` (behavior unchanged);
      depend on `kaman-math`; re-export the rapier handle types the ECS uses (`RigidBodyHandle`,
      `ColliderHandle`) consistently with `kaman-ecs::PhysicsBodyComponent`.
- [ ] Keep the existing surface: `create_dynamic_body`, `create_static_body`, `add_box_collider`,
      `add_sphere_collider`, `step`, `get_transform`, `set_velocity`, `get_velocity`.
- [ ] **New removal API:** `remove_body(handle)` (removing its colliders too) and
      `remove_collider(handle)`, using rapier's island/set removal correctly.
- [ ] **Use-after-free guard (test-first):** write the test *before* the removal impl — after
      `remove_body`, every query on the stale handle (`get_transform`/`get_velocity`/`set_velocity`)
      returns `None`/no-ops and never panics or derefs freed storage. State the invariant in doc.
- [ ] Step at the `FIXED_DT` from KE-0201 (kinematic/lane motion stays script/game-owned, not
      solver-driven — ARCHITECTURE §5).

## Technical notes
- Handle ownership invariant (mirror KE-0005): a body handle stored in a `PhysicsBodyComponent` must
  be removed from physics atomically with clearing/removing the component — doc + test + arch note.
- Reuse-as-is coverage target for the moved code; **Refactor ≥ 80%** on the new removal API
  (INTEGRATION §2.4).

## Out of scope
- ECS despawn orchestration / streaming (KE-0203). Custom arcade-physics replacement (post-v1).
- Character-controller / kinematic lane logic (lives in `car-runner`, KE-0204).

## Test gate
`cargo test -p kaman-physics` green incl. the use-after-free test; workspace oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; removal API documents the stale-handle + atomic-removal invariants
(enforced by tests); crate `README`; ARCHITECTURE §5 updated to note the removal API landed.
