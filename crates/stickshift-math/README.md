# kaman-math

Shared math layer for KamanEngine.

## Responsibility

`kaman-math` is the zero-coupling leaf crate that the rest of the engine
(`kaman-ecs`, `kaman-physics`, `kaman-camera`, `kaman-scene`, …) depends on for
spatial math. It does two things:

1. **Re-exports [`glam`]** so the whole workspace shares a single, version-pinned
   copy — downstream crates reach glam types through `kaman_math::glam::…`
   instead of adding their own `glam` dependency.
2. **Provides the engine [`Transform`] type** — a TRS
   (Translation-Rotation-Scale) convenience wrapper over glam's `Vec3`, `Quat`,
   and `Mat4`.

New math types (rays, bounding volumes, frustums, projection helpers) are added
only when a consumer needs them — they are intentionally out of scope until then.

## API tour

The core type is `Transform` (position + rotation + scale):

```rust
use kaman_math::Transform;
use kaman_math::glam::{Vec3, Quat};
use std::f32::consts::PI;

// Construct: identity, from a position, or a position + rotation.
let mut t = Transform::from_position(Vec3::new(5.0, 0.0, 0.0));

// Mutate in place.
t.rotate(Quat::from_rotation_y(PI / 2.0));
t.translate(Vec3::new(0.0, 1.0, 0.0));
t.scale = Vec3::new(2.0, 2.0, 2.0);

// Map local-space geometry into world space.
let world_point = t.transform_point(Vec3::new(1.0, 0.0, 0.0)); // affected by translation
let world_dir = t.transform_vector(Vec3::new(0.0, 0.0, 1.0));   // rotation/scale only

// Or bake the whole TRS into a 4×4 matrix for the GPU / camera pipeline.
let matrix = t.to_matrix();

let _ = (world_point, world_dir, matrix);
```

Constructors:

- `Transform::identity` / `Transform::default` — no translation, rotation, or scale.
- `Transform::from_position` — position only.
- `Transform::from_position_rotation` — position + rotation, unit scale.

Operations:

- `transform_point` — local → world for positions (applies S, R, then T).
- `transform_vector` — local → world for directions (S and R only, no T).
- `to_matrix` — 4×4 TRS matrix (Scale → Rotate → Translate).
- `rotate` / `translate` — in-place mutation.

[`glam`]: https://docs.rs/glam

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
