# kaman-physics

rapier3d wrapper for KamanEngine, with the **body/collider removal API** rapier's
prototype integration lacked.

`PhysicsWorld` creates dynamic/static rigid bodies, attaches box/sphere colliders,
steps the solver at the fixed engine timestep (`FIXED_DT = 1/60 s`, mirrored from
`kaman-core` to avoid a dependency cycle), and queries transforms/velocities. New
in v1: `remove_body` (removes the body and its attached colliders) and
`remove_collider`.

The rapier handle types (`RigidBodyHandle`, `ColliderHandle`) are re-exported so
consumers — notably `kaman-ecs`, whose `PhysicsBodyComponent` stores a
`RigidBodyHandle` — share one pinned copy.

## Invariants

- **Stale-handle safety.** After removal, every query on the freed handle
  (`get_transform`, `get_velocity`, `set_velocity`) returns `None` / is a no-op
  and never panics or dereferences freed storage (enforced by the
  `use_after_free_*` tests).
- **Handle ownership.** A handle stored in a `PhysicsBodyComponent` must be
  removed from physics atomically with clearing the component (despawn
  orchestration lands in KE-0203); stale-handle safety is the memory-safety
  backstop.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
