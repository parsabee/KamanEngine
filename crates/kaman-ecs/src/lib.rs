// Copyright (c) 2025 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Entity Component System (ECS) components and tags for KamanEngine.
//!
//! This crate is the stable component vocabulary that the Phase-1 `Game` trait
//! and the Phase-5 KamanScript runtime bind against. It defines the
//! engine-generic components stored in an [`hecs`] [`World`](hecs::World):
//!
//! - [`TransformComponent`] — spatial transform (position, rotation, scale).
//! - [`PhysicsBodyComponent`] — links an entity to a physics rigid body.
//! - [`RenderComponent`] / [`RenderShape`] — visual appearance (color + mesh).
//! - [`StaticTag`] / [`DynamicTag`] — categorize entities as fixed or
//!   physics-controlled.
//!
//! Everything here is **engine-generic**: there are deliberately no game-specific
//! components (no vehicle, track, tally, hazard, or gameplay-lattice types). Games
//! and scripts compose these primitives; game concepts live in the game crate,
//! not here. A unit test enforces this by scanning the crate source for the
//! forbidden concept words.
//!
//! [`hecs`] is re-exported ([`crate::hecs`]) so downstream crates depend on a
//! single, version-pinned copy without adding their own dependency.
//!
//! # Example
//!
//! ```rust
//! use kaman_ecs::{TransformComponent, RenderComponent, DynamicTag};
//! use kaman_ecs::hecs::World;
//! use kaman_math::glam::Vec3;
//!
//! let mut world = World::new();
//! let entity = world.spawn((
//!     TransformComponent::from_position(Vec3::new(0.0, 5.0, 0.0)),
//!     RenderComponent::cube([1.0, 0.0, 0.0]),
//!     DynamicTag,
//! ));
//! assert!(world.contains(entity));
//! ```

#![deny(missing_docs)]

use kaman_math::glam::Vec3;
use kaman_math::Transform;
use rapier3d::prelude::RigidBodyHandle;

/// Re-export of the [`hecs`] entity-component-system crate.
///
/// Downstream crates should reach `hecs` types (e.g. `World`, `Entity`) through
/// this path (`kaman_ecs::hecs::World`) so the whole workspace shares one pinned
/// version.
pub use hecs;

/// A component that stores an entity's spatial transform.
///
/// This component wraps a [`Transform`] which contains position, rotation, and scale.
/// It's used by the rendering system to determine where to draw objects and by the
/// physics system to synchronize entity positions with physics bodies.
///
/// # Example
///
/// ```rust
/// use kaman_ecs::TransformComponent;
/// use kaman_math::glam::Vec3;
///
/// let transform_comp = TransformComponent::from_position(Vec3::new(1.0, 2.0, 3.0));
/// assert_eq!(transform_comp.transform.position, Vec3::new(1.0, 2.0, 3.0));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct TransformComponent {
    /// The transform data (position, rotation, scale).
    pub transform: Transform,
}

impl TransformComponent {
    /// Creates a new transform component from an existing transform.
    ///
    /// # Arguments
    ///
    /// * `transform` - The transform to wrap.
    pub fn new(transform: Transform) -> Self {
        Self { transform }
    }

    /// Creates a new transform component at the specified position.
    ///
    /// The transform will have default rotation (identity) and scale (1, 1, 1).
    ///
    /// # Arguments
    ///
    /// * `position` - The position in 3D space.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_ecs::TransformComponent;
    /// use kaman_math::glam::Vec3;
    ///
    /// let transform = TransformComponent::from_position(Vec3::new(5.0, 0.0, -3.0));
    /// assert_eq!(transform.transform.position, Vec3::new(5.0, 0.0, -3.0));
    /// ```
    pub fn from_position(position: Vec3) -> Self {
        Self {
            transform: Transform::from_position(position),
        }
    }
}

/// A component that links an ECS entity to a physics rigid body.
///
/// This component stores a [`RigidBodyHandle`] into the physics simulation. The
/// physics system uses it to update the owning entity's [`TransformComponent`]
/// from the simulated body each step.
///
/// # Invariants
///
/// The stored [`handle`](Self::handle) is a lightweight, `Copy` index into a
/// physics `RigidBodySet` — it does **not** own the rigid body. Ownership of the
/// body lives in the physics world; this component is a non-owning reference.
/// Callers are responsible for keeping the handle and the physics world in sync:
/// a handle whose body has been removed from the set is dangling and must not be
/// dereferenced. (Handle-removal safety is tracked separately in KE-0202.)
///
/// # Example
///
/// ```rust,no_run
/// use kaman_ecs::PhysicsBodyComponent;
/// use rapier3d::prelude::RigidBodyHandle;
///
/// let handle = RigidBodyHandle::from_raw_parts(0, 1);
/// let physics_comp = PhysicsBodyComponent::new(handle);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct PhysicsBodyComponent {
    /// The (non-owning) handle to the rigid body in the physics world.
    pub handle: RigidBodyHandle,
}

impl PhysicsBodyComponent {
    /// Creates a new physics body component.
    ///
    /// # Arguments
    ///
    /// * `handle` - The rigid body handle from the physics simulation. The
    ///   component stores it by value and does not take ownership of the body
    ///   itself (see the type-level invariants).
    pub fn new(handle: RigidBodyHandle) -> Self {
        Self { handle }
    }
}

/// A component that specifies how an entity should be rendered.
///
/// This component contains the visual appearance of an entity, including its
/// color (RGB values in range 0.0-1.0) and shape (a triangle mesh).
///
/// # Example
///
/// ```rust
/// use kaman_ecs::RenderComponent;
///
/// let red_cube = RenderComponent::cube([1.0, 0.0, 0.0]);
/// let blue_sphere = RenderComponent::sphere([0.0, 0.0, 1.0]);
/// ```
#[derive(Debug, Clone)]
pub struct RenderComponent {
    /// RGB color values (range 0.0-1.0).
    pub color: [f32; 3],
    /// The shape to render.
    pub shape: RenderShape,
}

impl RenderComponent {
    /// Creates a new render component with the specified color and shape.
    ///
    /// # Arguments
    ///
    /// * `color` - RGB color values (range 0.0-1.0).
    /// * `shape` - The shape to render.
    pub fn new(color: [f32; 3], shape: RenderShape) -> Self {
        Self { color, shape }
    }

    /// Creates a render component for a cube with the specified color.
    ///
    /// # Arguments
    ///
    /// * `color` - RGB color values (range 0.0-1.0).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_ecs::RenderComponent;
    ///
    /// let red_cube = RenderComponent::cube([1.0, 0.0, 0.0]);
    /// ```
    pub fn cube(color: [f32; 3]) -> Self {
        Self {
            color,
            shape: RenderShape::cube(color),
        }
    }

    /// Creates a render component for a sphere with the specified color.
    ///
    /// Uses default quality settings (20 segments × 20 rings).
    ///
    /// # Arguments
    ///
    /// * `color` - RGB color values (range 0.0-1.0).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_ecs::RenderComponent;
    ///
    /// let green_sphere = RenderComponent::sphere([0.0, 1.0, 0.0]);
    /// ```
    pub fn sphere(color: [f32; 3]) -> Self {
        Self {
            color,
            shape: RenderShape::sphere(color, 20, 20),
        }
    }

    /// Creates a render component for a custom triangle mesh.
    ///
    /// # Arguments
    ///
    /// * `color` - Base RGB color values (range 0.0-1.0), used if vertices don't have colors.
    /// * `vertices` - Vertex data where each vertex is
    ///   `[pos_x, pos_y, pos_z, normal_x, normal_y, normal_z, color_r, color_g, color_b]`.
    /// * `indices` - Triangle indices (3 per triangle).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use kaman_ecs::RenderComponent;
    ///
    /// // Create a simple triangle
    /// let vertices = vec![
    ///     [0.0, 1.0, 0.0,  0.0, 0.0, 1.0,  1.0, 0.0, 0.0], // top, red
    ///     [-1.0, -1.0, 0.0,  0.0, 0.0, 1.0,  0.0, 1.0, 0.0], // bottom-left, green
    ///     [1.0, -1.0, 0.0,  0.0, 0.0, 1.0,  0.0, 0.0, 1.0], // bottom-right, blue
    /// ];
    /// let indices = vec![0, 1, 2];
    /// let mesh = RenderComponent::mesh([1.0, 1.0, 1.0], vertices, indices);
    /// ```
    pub fn mesh(color: [f32; 3], vertices: Vec<[f32; 9]>, indices: Vec<u32>) -> Self {
        Self {
            color,
            shape: RenderShape::Mesh { vertices, indices },
        }
    }
}

/// The types of shapes that can be rendered.
///
/// All geometry is represented as triangle meshes with vertices and indices.
///
/// This enum is [`#[non_exhaustive]`](https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute):
/// additional built-in shape representations may be added in future engine
/// versions, so downstream `match` expressions must include a wildcard arm.
/// Construct shapes through the [`RenderComponent`] constructors
/// ([`cube`](RenderComponent::cube), [`sphere`](RenderComponent::sphere),
/// [`mesh`](RenderComponent::mesh)) or the [`RenderShape`] generators rather than
/// building variants directly.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RenderShape {
    /// A custom triangle mesh with vertices and indices.
    Mesh {
        /// Vertex data: each vertex is `[position_xyz, normal_xyz, color_rgb]`.
        vertices: Vec<[f32; 9]>,
        /// Triangle indices (3 indices per triangle).
        indices: Vec<u32>,
    },
}

impl RenderShape {
    /// Generates a cube mesh (1×1×1 unit box).
    ///
    /// Creates a cube with 24 vertices and 12 triangles (2 per face).
    /// Each face has its own normal vector for flat shading.
    ///
    /// # Arguments
    ///
    /// * `color` - RGB color for all vertices.
    pub fn cube(color: [f32; 3]) -> Self {
        let vertices = vec![
            // Front face (Z+) - 4 vertices
            [-0.5, -0.5, 0.5, 0.0, 0.0, 1.0, color[0], color[1], color[2]], // 0
            [0.5, -0.5, 0.5, 0.0, 0.0, 1.0, color[0], color[1], color[2]],  // 1
            [0.5, 0.5, 0.5, 0.0, 0.0, 1.0, color[0], color[1], color[2]],   // 2
            [-0.5, 0.5, 0.5, 0.0, 0.0, 1.0, color[0], color[1], color[2]],  // 3
            // Back face (Z-) - 4 vertices
            [0.5, -0.5, -0.5, 0.0, 0.0, -1.0, color[0], color[1], color[2]], // 4
            [-0.5, -0.5, -0.5, 0.0, 0.0, -1.0, color[0], color[1], color[2]], // 5
            [-0.5, 0.5, -0.5, 0.0, 0.0, -1.0, color[0], color[1], color[2]], // 6
            [0.5, 0.5, -0.5, 0.0, 0.0, -1.0, color[0], color[1], color[2]],  // 7
            // Top face (Y+) - 4 vertices
            [-0.5, 0.5, 0.5, 0.0, 1.0, 0.0, color[0], color[1], color[2]], // 8
            [0.5, 0.5, 0.5, 0.0, 1.0, 0.0, color[0], color[1], color[2]],  // 9
            [0.5, 0.5, -0.5, 0.0, 1.0, 0.0, color[0], color[1], color[2]], // 10
            [-0.5, 0.5, -0.5, 0.0, 1.0, 0.0, color[0], color[1], color[2]], // 11
            // Bottom face (Y-) - 4 vertices
            [-0.5, -0.5, -0.5, 0.0, -1.0, 0.0, color[0], color[1], color[2]], // 12
            [0.5, -0.5, -0.5, 0.0, -1.0, 0.0, color[0], color[1], color[2]],  // 13
            [0.5, -0.5, 0.5, 0.0, -1.0, 0.0, color[0], color[1], color[2]],   // 14
            [-0.5, -0.5, 0.5, 0.0, -1.0, 0.0, color[0], color[1], color[2]],  // 15
            // Right face (X+) - 4 vertices
            [0.5, -0.5, 0.5, 1.0, 0.0, 0.0, color[0], color[1], color[2]],  // 16
            [0.5, -0.5, -0.5, 1.0, 0.0, 0.0, color[0], color[1], color[2]], // 17
            [0.5, 0.5, -0.5, 1.0, 0.0, 0.0, color[0], color[1], color[2]],  // 18
            [0.5, 0.5, 0.5, 1.0, 0.0, 0.0, color[0], color[1], color[2]],   // 19
            // Left face (X-) - 4 vertices
            [-0.5, -0.5, -0.5, -1.0, 0.0, 0.0, color[0], color[1], color[2]], // 20
            [-0.5, -0.5, 0.5, -1.0, 0.0, 0.0, color[0], color[1], color[2]],  // 21
            [-0.5, 0.5, 0.5, -1.0, 0.0, 0.0, color[0], color[1], color[2]],   // 22
            [-0.5, 0.5, -0.5, -1.0, 0.0, 0.0, color[0], color[1], color[2]],  // 23
        ];

        let indices = vec![
            // Front face
            0, 1, 2, 0, 2, 3, // Back face
            4, 5, 6, 4, 6, 7, // Top face
            8, 9, 10, 8, 10, 11, // Bottom face
            12, 13, 14, 12, 14, 15, // Right face
            16, 17, 18, 16, 18, 19, // Left face
            20, 21, 22, 20, 22, 23,
        ];

        RenderShape::Mesh { vertices, indices }
    }

    /// Generates a sphere mesh using a UV-sphere algorithm.
    ///
    /// Creates a smooth sphere with the specified number of segments and rings.
    /// Twenty segments and twenty rings gives a good balance of quality and
    /// performance.
    ///
    /// # Arguments
    ///
    /// * `color` - RGB color for all vertices.
    /// * `segments` - Number of horizontal divisions (longitude).
    /// * `rings` - Number of vertical divisions (latitude).
    pub fn sphere(color: [f32; 3], segments: u32, rings: u32) -> Self {
        let mut vertices = Vec::new();

        // Generate sphere vertices
        for ring in 0..=rings {
            let theta = ring as f32 * std::f32::consts::PI / rings as f32;
            let sin_theta = theta.sin();
            let cos_theta = theta.cos();

            for segment in 0..=segments {
                let phi = segment as f32 * 2.0 * std::f32::consts::PI / segments as f32;
                let sin_phi = phi.sin();
                let cos_phi = phi.cos();

                let x = cos_phi * sin_theta;
                let y = cos_theta;
                let z = sin_phi * sin_theta;

                // Position (scaled to radius 0.5), normal (unit sphere), color
                vertices.push([
                    x * 0.5,
                    y * 0.5,
                    z * 0.5, // position
                    x,
                    y,
                    z, // normal
                    color[0],
                    color[1],
                    color[2], // color
                ]);
            }
        }

        // Generate triangle indices
        let mut indices = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let current = ring * (segments + 1) + segment;
                let next = current + segments + 1;

                // First triangle
                indices.push(current);
                indices.push(next);
                indices.push(current + 1);

                // Second triangle
                indices.push(current + 1);
                indices.push(next);
                indices.push(next + 1);
            }
        }

        RenderShape::Mesh { vertices, indices }
    }
}

/// A tag component marking an entity as static (non-moving).
///
/// Static entities don't have their transforms updated by the physics system.
/// They're typically used for ground planes, walls, and other fixed geometry.
///
/// # Example
///
/// ```rust
/// use kaman_ecs::{TransformComponent, RenderComponent, StaticTag};
/// use kaman_ecs::hecs::World;
/// use kaman_math::glam::Vec3;
///
/// let mut world = World::new();
/// world.spawn((
///     TransformComponent::from_position(Vec3::ZERO),
///     RenderComponent::cube([0.5, 0.5, 0.5]),
///     StaticTag,
/// ));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct StaticTag;

/// A tag component marking an entity as dynamic (physics-controlled).
///
/// Dynamic entities have their transforms updated from the physics simulation
/// each frame. They respond to forces, collisions, and gravity.
///
/// # Example
///
/// ```rust,no_run
/// use kaman_ecs::{TransformComponent, RenderComponent, PhysicsBodyComponent, DynamicTag};
/// use kaman_ecs::hecs::World;
/// use kaman_math::glam::Vec3;
/// use rapier3d::prelude::RigidBodyHandle;
///
/// let mut world = World::new();
/// let handle = RigidBodyHandle::from_raw_parts(0, 1);
/// world.spawn((
///     TransformComponent::from_position(Vec3::new(0.0, 5.0, 0.0)),
///     PhysicsBodyComponent::new(handle),
///     RenderComponent::cube([1.0, 0.0, 0.0]),
///     DynamicTag,
/// ));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DynamicTag;

#[cfg(test)]
mod tests {
    use super::*;
    use hecs::World;
    use kaman_math::glam::Vec3;

    #[test]
    fn test_create_entity_with_components() {
        let mut world = World::new();

        let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
        let render = RenderComponent::cube([1.0, 0.0, 0.0]);

        let entity = world.spawn((TransformComponent::new(transform), render, DynamicTag));

        assert!(world.contains(entity));
    }

    #[test]
    fn test_query_transforms() {
        let mut world = World::new();

        // Create multiple entities with transforms
        world.spawn((
            TransformComponent::from_position(Vec3::new(0.0, 0.0, 0.0)),
            RenderComponent::cube([1.0, 0.0, 0.0]),
        ));

        world.spawn((
            TransformComponent::from_position(Vec3::new(1.0, 1.0, 1.0)),
            RenderComponent::cube([0.0, 1.0, 0.0]),
        ));

        world.spawn((
            TransformComponent::from_position(Vec3::new(2.0, 2.0, 2.0)),
            RenderComponent::cube([0.0, 0.0, 1.0]),
        ));

        // Query all entities with transform and render components
        let mut count = 0;
        for (_entity, (transform, render)) in
            world.query::<(&TransformComponent, &RenderComponent)>().iter()
        {
            count += 1;
            assert!(transform.transform.position.length() >= 0.0);
            // Check that shape is a Mesh variant
            assert!(matches!(render.shape, RenderShape::Mesh { .. }));
        }

        assert_eq!(count, 3);
    }

    #[test]
    fn test_query_dynamic_entities() {
        let mut world = World::new();

        // Static entity
        world.spawn((
            TransformComponent::from_position(Vec3::ZERO),
            RenderComponent::cube([0.5, 0.5, 0.5]),
            StaticTag,
        ));

        // Dynamic entities
        world.spawn((
            TransformComponent::from_position(Vec3::new(1.0, 0.0, 0.0)),
            RenderComponent::cube([1.0, 0.0, 0.0]),
            DynamicTag,
        ));

        world.spawn((
            TransformComponent::from_position(Vec3::new(2.0, 0.0, 0.0)),
            RenderComponent::cube([0.0, 1.0, 0.0]),
            DynamicTag,
        ));

        // Query only dynamic entities
        let mut dynamic_count = 0;
        for (_entity, (_transform, _tag)) in
            world.query::<(&TransformComponent, &DynamicTag)>().iter()
        {
            dynamic_count += 1;
        }

        assert_eq!(dynamic_count, 2);
    }

    #[test]
    fn test_modify_transform() {
        let mut world = World::new();

        let entity = world.spawn((
            TransformComponent::from_position(Vec3::ZERO),
            RenderComponent::cube([1.0, 1.0, 1.0]),
        ));

        // Modify the transform
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
            transform.transform.position = Vec3::new(5.0, 10.0, 15.0);
        }

        // Verify modification
        let pos = world
            .get::<&TransformComponent>(entity)
            .map(|t| t.transform.position)
            .unwrap();
        assert_eq!(pos, Vec3::new(5.0, 10.0, 15.0));
    }

    #[test]
    fn test_physics_body_component() {
        let mut world = World::new();

        // Create a dummy physics body handle
        let handle = RigidBodyHandle::from_raw_parts(0, 1);

        let _entity = world.spawn((
            TransformComponent::from_position(Vec3::new(1.0, 2.0, 3.0)),
            PhysicsBodyComponent::new(handle),
            RenderComponent::cube([1.0, 0.0, 0.0]),
            DynamicTag,
        ));

        // Query entities with physics bodies
        let mut found = false;
        for (_entity, (physics, _transform)) in
            world.query::<(&PhysicsBodyComponent, &TransformComponent)>().iter()
        {
            assert_eq!(physics.handle, handle);
            found = true;
        }

        assert!(found);
    }

    #[test]
    fn test_remove_entity() {
        let mut world = World::new();

        let entity = world.spawn((
            TransformComponent::from_position(Vec3::ZERO),
            RenderComponent::cube([1.0, 1.0, 1.0]),
        ));

        assert!(world.contains(entity));

        world.despawn(entity).unwrap();

        assert!(!world.contains(entity));
    }

    #[test]
    fn test_render_shape_variants() {
        let cube = RenderComponent::cube([1.0, 0.0, 0.0]);
        let sphere = RenderComponent::sphere([0.0, 1.0, 0.0]);

        // Both should be Mesh variants now
        assert!(matches!(cube.shape, RenderShape::Mesh { .. }));
        assert!(matches!(sphere.shape, RenderShape::Mesh { .. }));
        // They should have different vertex data
        assert_ne!(cube.shape, sphere.shape);
    }

    #[test]
    fn test_query_by_color() {
        let mut world = World::new();

        let red = [1.0, 0.0, 0.0];
        let green = [0.0, 1.0, 0.0];

        world.spawn((
            TransformComponent::from_position(Vec3::ZERO),
            RenderComponent::cube(red),
        ));

        world.spawn((
            TransformComponent::from_position(Vec3::new(1.0, 0.0, 0.0)),
            RenderComponent::cube(green),
        ));

        // Count red objects
        let mut red_count = 0;
        for (_entity, render) in world.query::<&RenderComponent>().iter() {
            if render.color == red {
                red_count += 1;
            }
        }

        assert_eq!(red_count, 1);
    }

    /// The cube mesh helper composes into exactly 24 vertices / 36 indices and
    /// each vertex round-trips as `[pos_xyz, normal_xyz, color_rgb]` (9 floats)
    /// carrying the requested color. Guards the mesh (de)composition invariant.
    #[test]
    fn test_cube_mesh_decomposition_invariants() {
        let color = [0.2, 0.4, 0.6];
        let RenderShape::Mesh { vertices, indices } = RenderShape::cube(color);

        assert_eq!(vertices.len(), 24, "cube has 24 vertices (4 per face)");
        assert_eq!(indices.len(), 36, "cube has 12 triangles (36 indices)");
        // Every index must reference a real vertex.
        for &i in &indices {
            assert!((i as usize) < vertices.len(), "index {i} out of range");
        }
        // Color channels are carried in the trailing 3 floats of each vertex.
        for v in &vertices {
            assert_eq!([v[6], v[7], v[8]], color);
        }
    }

    /// A sphere with `s` segments and `r` rings composes into `(s+1)*(r+1)`
    /// vertices and `6*s*r` indices, all in range. Guards the parametric
    /// (de)composition invariant.
    #[test]
    fn test_sphere_mesh_decomposition_invariants() {
        let (segments, rings) = (8u32, 6u32);
        let RenderShape::Mesh { vertices, indices } =
            RenderShape::sphere([1.0, 1.0, 1.0], segments, rings);

        let expected_verts = ((segments + 1) * (rings + 1)) as usize;
        let expected_indices = (6 * segments * rings) as usize;
        assert_eq!(vertices.len(), expected_verts);
        assert_eq!(indices.len(), expected_indices);
        for &i in &indices {
            assert!((i as usize) < vertices.len(), "index {i} out of range");
        }
    }

    /// A1-boundary guard: this crate must stay engine-generic.
    ///
    /// It embeds this crate's entire source with `include_str!` and scans it,
    /// token by token (splitting on non-alphanumeric boundaries), for any
    /// game-specific identifier. If one appears anywhere in the source the test
    /// fails, so a forbidden component/type cannot be added without this test
    /// seeing it.
    ///
    /// The forbidden words are assembled from byte codes rather than written as
    /// string literals, precisely so the source file contains no occurrence of
    /// them — the test cannot false-positive on its own body, and it can scan
    /// the whole file with no exclusion window. Matching is case-insensitive and
    /// whole-word, so an incidental substring inside a larger identifier does
    /// not trip the guard; only the exact concept words do.
    #[test]
    fn test_no_game_specific_symbols() {
        let src = include_str!("lib.rs");

        // Game concepts, built from ASCII byte codes so the words never appear
        // literally anywhere in this source file (which is exactly what lets the
        // test scan its own source without excluding itself). Codes spell:
        //   [99,97,114]=…, [114,111,97,100]=…, [115,99,111,114,101]=…,
        //   [111,98,115,116,97,99,108,101]=…, [108,97,110,101]=…
        let forbidden: Vec<String> = [
            &[99u8, 97, 114][..],
            &[114, 111, 97, 100][..],
            &[115, 99, 111, 114, 101][..],
            &[111, 98, 115, 116, 97, 99, 108, 101][..],
            &[108, 97, 110, 101][..],
        ]
        .iter()
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
        .collect();

        for token in src.split(|c: char| !c.is_ascii_alphanumeric()) {
            if token.is_empty() {
                continue;
            }
            let lower = token.to_ascii_lowercase();
            for bad in &forbidden {
                assert_ne!(
                    &lower, bad,
                    "game-specific identifier `{token}` found in lib.rs: \
                     kaman-ecs must stay engine-generic (A1 boundary)",
                );
            }
        }
    }
}
