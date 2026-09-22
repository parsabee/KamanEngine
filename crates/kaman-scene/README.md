# kaman-scene

Owns the world: a hecs `World` plus a `kaman-physics` `PhysicsWorld`, with **streaming**
(spawn-ahead / despawn-behind) and **floating-origin rebase** for an endless world. This crate
is **engine-generic** — it holds no game types (no car/road/lane/obstacle/score); the game
supplies the focus point and decides *what* to spawn, while the scene owns the bookkeeping.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.

## Responsibility

- `Scene` — owns `World` + `PhysicsWorld`; `world()/world_mut()`, `physics()/physics_mut()`, and
  `step_physics()` (steps at `FIXED_DT`, then syncs each dynamic body's transform back into its
  `TransformComponent`).
- `StreamingConfig` — 1-D streaming policy along an `axis`: `spawn_interval`, `spawn_ahead`,
  `despawn_behind`, `rebase_threshold`.
- `stream(focus, spawn)` — despawns tracked entities that fell more than `despawn_behind` behind
  the focus, then invokes the game's `spawn` callback once per unfilled slot up to
  `focus + spawn_ahead`. `SpawnCtx` gives the callback `world`/`physics`/`position`/`slot` and a
  `spawned(entity)` hook to mark an entity for despawn-behind (untracked spawns are permanent).
- `maybe_rebase(focus) -> Vec3` — when the focus passes `rebase_threshold`, shifts all ECS
  transforms **and** physics bodies back toward the origin by a whole multiple of the threshold,
  transparently to gameplay; returns the applied offset.

## Invariants

- **Atomic despawn:** `despawn` removes the entity's physics body (`PhysicsWorld::remove_body`)
  *before* despawning the ECS entity, so no live `PhysicsBodyComponent` ever holds a freed handle
  (see [`kaman-physics`](../kaman-physics/README.md) and [`kaman-ecs`](../kaman-ecs/README.md)).
- **Rebase ordering:** rebase runs **between** physics steps, never mid-solve — the documented
  cadence is `stream → step_physics → maybe_rebase`, so the solver never sees a discontinuous
  position within a step. Because every position shifts by the same vector, relative positions are
  exactly preserved.
- **Bounded memory:** with a fixed streaming window, entity and rigid-body counts stay bounded as
  the focus advances forever (asserted over 2000 simulated frames).

## Dependency direction

`kaman-scene → { kaman-ecs, kaman-physics, kaman-math }` — acyclic; it does **not** depend on
`kaman-core`. `kaman-core` owns a `Scene` inside its loop and exposes it via `EngineCtx`.
