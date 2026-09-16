# kaman-ecs

The ECS component vocabulary for KamanEngine — the stable surface that the
Phase-1 `Game` trait and the Phase-5 KamanScript runtime bind against.

## Responsibility

`kaman-ecs` layers a small set of **engine-generic** components on top of the
[`hecs`] entity-component-system. Storage, entity IDs, spawning and querying all
come from `hecs` (re-exported as `kaman_ecs::hecs`); this crate only defines the
component *types* the rest of the engine and its games agree on.

It builds on [`kaman-math`] for spatial types (`Transform`, `glam`) and holds a
non-owning [`rapier3d`] rigid-body handle for the physics link.

## The component model (binding surface)

This is the vocabulary a `Game` or a `.kaman` script composes to describe a
scene. Everything here is deliberately **generic** — there are no game concepts
(no car, road, score, obstacle, or lane). Game meaning is built by *composing*
these primitives in the game crate, never by adding types here.

| Component | Kind | Purpose |
|---|---|---|
| `TransformComponent` | data | Wraps a `kaman_math::Transform` (position / rotation / scale). Read by rendering, written by physics. |
| `PhysicsBodyComponent` | data | Non-owning `RigidBodyHandle` into the physics world. |
| `RenderComponent` | data | Visual appearance: an RGB `color` plus a `RenderShape`. |
| `RenderShape` | enum (`#[non_exhaustive]`) | Geometry as a triangle `Mesh { vertices, indices }`. Mesh generators for cube and UV-sphere. |
| `StaticTag` | tag | Marks fixed geometry — physics does not drive its transform. |
| `DynamicTag` | tag | Marks physics-controlled entities — transform is updated from the simulation. |

### Mesh-shape helpers

`RenderShape::cube(color)` and `RenderShape::sphere(color, segments, rings)`
generate flat/smooth triangle meshes; each vertex is nine floats
`[pos_xyz, normal_xyz, color_rgb]`. `RenderComponent::{cube, sphere, mesh}` are
the convenience constructors most callers use.

### Invariants

- **Handle ownership.** `PhysicsBodyComponent` stores a *non-owning* copy of a
  `RigidBodyHandle`. The rigid body itself is owned by the physics world; the
  component is only a reference. Keeping the handle valid (not dereferencing a
  handle whose body was removed) is the caller's responsibility. Safe removal is
  tracked in KE-0202.
- **Static vs dynamic.** `StaticTag` and `DynamicTag` are mutually-exclusive
  intent markers: a static entity's transform is authored once and never touched
  by physics; a dynamic entity's transform is overwritten from the simulation
  each step.
- **`RenderShape` is `#[non_exhaustive]`.** New built-in shape representations
  may be added later, so external `match`es must include a wildcard arm. This
  keeps the A1 boundary from silently hardening into an A2 break.

## Example

```rust
use kaman_ecs::{TransformComponent, RenderComponent, PhysicsBodyComponent, DynamicTag};
use kaman_ecs::hecs::World;
use kaman_math::glam::Vec3;

let mut world = World::new();
let entity = world.spawn((
    TransformComponent::from_position(Vec3::new(0.0, 5.0, 0.0)),
    RenderComponent::cube([1.0, 0.0, 0.0]),
    DynamicTag,
));

for (_e, (t, r)) in world.query::<(&TransformComponent, &RenderComponent)>().iter() {
    let _ = (t.transform.position, &r.shape);
}
```

## Guarding the boundary

A unit test (`test_no_game_specific_symbols`) greps this crate's own source via
`include_str!` and fails if any game-specific identifier
(`car`/`road`/`score`/`obstacle`/`lane`) appears in a definition — the A1 public
surface stays engine-generic by construction.

[`hecs`]: https://docs.rs/hecs
[`kaman-math`]: ../kaman-math
[`rapier3d`]: https://docs.rs/rapier3d

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
