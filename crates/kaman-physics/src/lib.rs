// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! rapier3d wrapper with a body/collider removal API for KamanEngine.
//!
//! This crate wraps the [rapier3d](https://rapier.rs) physics engine behind a
//! small, engine-owned surface so that a later custom arcade-physics layer can
//! replace it without rippling through callers (ARCHITECTURE §5). It offers a
//! simplified interface for creating rigid bodies, adding colliders, stepping
//! the simulation, and — new in v1 — **removing** bodies and colliders.
//!
//! # Features
//!
//! - Dynamic and static rigid bodies
//! - Box and sphere colliders
//! - Gravity simulation (default: -9.81 m/s² on Y axis)
//! - Transform synchronization with the ECS
//! - Velocity control
//! - **Body/collider removal** with stale-handle safety
//!
//! # Handle types
//!
//! Rigid-body and collider handles are rapier's own [`RigidBodyHandle`] and
//! [`ColliderHandle`], re-exported here so downstream crates (notably
//! `kaman-ecs`, whose `PhysicsBodyComponent` stores a [`RigidBodyHandle`])
//! depend on a single, version-pinned copy of the handle type.
//!
//! # Invariants
//!
//! - **Stale-handle safety.** After [`PhysicsWorld::remove_body`] (or
//!   [`PhysicsWorld::remove_collider`]), the handle is invalid. Every query
//!   against a stale handle — [`get_transform`](PhysicsWorld::get_transform),
//!   [`get_velocity`](PhysicsWorld::get_velocity),
//!   [`set_velocity`](PhysicsWorld::set_velocity) — returns `None` / is a no-op
//!   and **never panics or dereferences freed storage**. This is guaranteed by
//!   rapier's generational handles (a removed slot's generation is bumped) and
//!   is enforced by the `use_after_free_*` tests.
//! - **Handle ownership.** A [`RigidBodyHandle`] stored in a
//!   `kaman_ecs::PhysicsBodyComponent` is owned jointly by the ECS and this
//!   world. It must be removed from physics (via [`remove_body`]) *atomically*
//!   with clearing / despawning the component, so no live component ever holds
//!   a handle that has already been freed here. This crate cannot enforce that
//!   coupling on its own (it does not know about the ECS); the despawn
//!   orchestration that upholds it lands in KE-0203. Stale-handle safety above
//!   is the backstop that keeps a transient mismatch memory-safe.
//!
//! [`remove_body`]: PhysicsWorld::remove_body
//!
//! # Timestep
//!
//! The solver advances in a fixed timestep. [`PhysicsWorld::step`] defaults to
//! [`FIXED_DT`], which **must equal** `kaman_core::FIXED_DT` (`1/60 s`, the
//! engine-wide single source of truth per ARCHITECTURE §4). It is redefined
//! locally rather than imported to avoid a dependency cycle: `kaman-core`
//! depends on the ECS/scene layers that will depend on `kaman-physics`, so
//! `kaman-physics` must not depend on `kaman-core`. A test asserts the two
//! constants match once both crates are in the same build.
//!
//! # Example
//!
//! ```rust
//! use kaman_physics::PhysicsWorld;
//! use kaman_math::Transform;
//! use kaman_math::glam::Vec3;
//!
//! let mut physics = PhysicsWorld::new();
//!
//! // Create a dynamic cube.
//! let transform = Transform::from_position(Vec3::new(0.0, 5.0, 0.0));
//! let body = physics.create_dynamic_body(transform);
//! physics.add_box_collider(body, Vec3::new(0.5, 0.5, 0.5));
//!
//! // Simulate physics at the fixed timestep.
//! for _ in 0..60 {
//!     physics.step();
//! }
//!
//! // Get updated position.
//! if let Some(new_transform) = physics.get_transform(body) {
//!     println!("Object fell to: {:?}", new_transform.position);
//! }
//!
//! // Despawn it — the handle is now stale and safe to query.
//! physics.remove_body(body);
//! assert!(physics.get_transform(body).is_none());
//! ```

#![deny(missing_docs)]

use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use rapier3d::na::{Quaternion, UnitQuaternion};
use rapier3d::prelude::*;

/// Re-export of rapier's [`RigidBodyHandle`].
///
/// Downstream crates reach the handle type through this path so the whole
/// workspace shares one pinned rapier version. `kaman_ecs::PhysicsBodyComponent`
/// stores a value of this exact type.
pub use rapier3d::prelude::RigidBodyHandle;

/// Re-export of rapier's [`ColliderHandle`].
///
/// Reached through this path so the workspace shares one pinned rapier version.
pub use rapier3d::prelude::ColliderHandle;

/// Fixed physics timestep, in seconds (`1/60 s`).
///
/// This is the default `dt` for [`PhysicsWorld::step`]. It **must** equal
/// `kaman_core::FIXED_DT`, the engine-wide fixed-update rate (ARCHITECTURE §4).
/// It is defined here — rather than imported from `kaman-core` — to avoid a
/// dependency cycle (`kaman-core` sits above the physics/scene layers). The
/// equality is asserted by a test in any build that includes both crates.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// A physics simulation world managing rigid bodies and colliders.
///
/// The `PhysicsWorld` wraps rapier3d and provides a simplified interface for
/// common physics operations:
/// - Rigid body creation (dynamic and static)
/// - Collider attachment (boxes and spheres)
/// - Physics simulation stepping at a fixed timestep
/// - Transform / velocity queries and updates
/// - Body / collider removal
///
/// # Example
///
/// ```rust
/// use kaman_physics::PhysicsWorld;
/// use kaman_math::Transform;
/// use kaman_math::glam::Vec3;
///
/// let mut physics = PhysicsWorld::new();
///
/// // Create ground.
/// let ground_transform = Transform::from_position(Vec3::new(0.0, -1.0, 0.0));
/// let ground = physics.create_static_body(ground_transform);
/// physics.add_box_collider(ground, Vec3::new(10.0, 0.1, 10.0));
/// ```
pub struct PhysicsWorld {
    /// Gravity vector (default: -9.81 on Y axis).
    pub gravity: Vec3,
    /// Physics integration parameters (timestep, iterations, etc.).
    pub integration_parameters: IntegrationParameters,
    /// The main physics pipeline.
    pub physics_pipeline: PhysicsPipeline,
    /// Manages simulation islands for optimization.
    pub island_manager: IslandManager,
    /// Broad-phase collision detection.
    pub broad_phase: DefaultBroadPhase,
    /// Narrow-phase collision detection.
    pub narrow_phase: NarrowPhase,
    /// Set of all rigid bodies.
    pub rigid_body_set: RigidBodySet,
    /// Set of all colliders.
    pub collider_set: ColliderSet,
    /// Impulse-based joints.
    pub impulse_joint_set: ImpulseJointSet,
    /// Articulation/multibody joints.
    pub multibody_joint_set: MultibodyJointSet,
    /// Continuous collision detection solver.
    pub ccd_solver: CCDSolver,
    /// Query pipeline for raycasts and spatial queries.
    pub query_pipeline: QueryPipeline,
}

impl PhysicsWorld {
    /// Creates a new physics world with default gravity (-9.81 on Y axis).
    ///
    /// Equivalent to `PhysicsWorld::with_gravity(Vec3::new(0.0, -9.81, 0.0))`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    ///
    /// let physics = PhysicsWorld::new();
    /// ```
    pub fn new() -> Self {
        Self::with_gravity(Vec3::new(0.0, -9.81, 0.0))
    }

    /// Creates a physics world with custom gravity.
    ///
    /// # Arguments
    ///
    /// * `gravity` - The gravity vector in m/s² (e.g. `Vec3::new(0.0, -9.81, 0.0)` for Earth gravity).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::glam::Vec3;
    ///
    /// // Moon gravity (approximately 1/6 of Earth).
    /// let physics = PhysicsWorld::with_gravity(Vec3::new(0.0, -1.62, 0.0));
    /// ```
    pub fn with_gravity(gravity: Vec3) -> Self {
        Self {
            gravity,
            integration_parameters: IntegrationParameters::default(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
        }
    }

    /// Advances the physics simulation by one fixed timestep ([`FIXED_DT`]).
    ///
    /// Callers step the world exactly once per fixed engine update, so the
    /// solver stays in lockstep with `kaman_core`'s fixed-timestep driver.
    /// This is a convenience wrapper over [`step_dt`](Self::step_dt) with
    /// `dt = FIXED_DT`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// physics.step(); // Advance by FIXED_DT (~16.67ms).
    /// ```
    pub fn step(&mut self) {
        self.step_dt(FIXED_DT);
    }

    /// Advances the physics simulation by an explicit timestep `dt` (seconds).
    ///
    /// Prefer [`step`](Self::step), which uses the engine-wide [`FIXED_DT`].
    /// This variant exists for tests and specialized fixed-substep loops; the
    /// engine's shipping path always steps at the fixed rate (ARCHITECTURE §4).
    ///
    /// # Arguments
    ///
    /// * `dt` - The timestep in seconds.
    pub fn step_dt(&mut self, dt: f32) {
        self.integration_parameters.dt = dt;

        let gravity_vector = vector![self.gravity.x, self.gravity.y, self.gravity.z];

        self.physics_pipeline.step(
            &gravity_vector,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &(),
        );
    }

    /// Creates a dynamic rigid body at the specified transform.
    ///
    /// Dynamic bodies are affected by forces, gravity, and collisions.
    ///
    /// # Arguments
    ///
    /// * `transform` - Initial position, rotation, and scale.
    ///
    /// # Returns
    ///
    /// A handle to the created rigid body. Use it to add colliders or query the body.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let transform = Transform::from_position(Vec3::new(0.0, 10.0, 0.0));
    /// let body = physics.create_dynamic_body(transform);
    /// ```
    pub fn create_dynamic_body(&mut self, transform: Transform) -> RigidBodyHandle {
        let rotation = UnitQuaternion::from_quaternion(Quaternion::new(
            transform.rotation.w,
            transform.rotation.x,
            transform.rotation.y,
            transform.rotation.z,
        ));

        let rigid_body = RigidBodyBuilder::dynamic()
            .translation(vector![
                transform.position.x,
                transform.position.y,
                transform.position.z
            ])
            .rotation(rotation.scaled_axis())
            .build();

        self.rigid_body_set.insert(rigid_body)
    }

    /// Creates a static rigid body at the specified transform.
    ///
    /// Static bodies are immovable and unaffected by forces or gravity. Used for
    /// ground planes, walls, and other fixed geometry.
    ///
    /// # Arguments
    ///
    /// * `transform` - Initial position, rotation, and scale.
    ///
    /// # Returns
    ///
    /// A handle to the created rigid body.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let transform = Transform::from_position(Vec3::new(0.0, -1.0, 0.0));
    /// let ground = physics.create_static_body(transform);
    /// physics.add_box_collider(ground, Vec3::new(10.0, 0.1, 10.0));
    /// ```
    pub fn create_static_body(&mut self, transform: Transform) -> RigidBodyHandle {
        let rotation = UnitQuaternion::from_quaternion(Quaternion::new(
            transform.rotation.w,
            transform.rotation.x,
            transform.rotation.y,
            transform.rotation.z,
        ));

        let rigid_body = RigidBodyBuilder::fixed()
            .translation(vector![
                transform.position.x,
                transform.position.y,
                transform.position.z
            ])
            .rotation(rotation.scaled_axis())
            .build();

        self.rigid_body_set.insert(rigid_body)
    }

    /// Adds a box-shaped collider to a rigid body.
    ///
    /// The size is specified as half-extents (distance from center to each face).
    ///
    /// # Arguments
    ///
    /// * `body_handle` - The rigid body to attach the collider to.
    /// * `half_extents` - Half-width, half-height, half-depth (e.g. `Vec3::new(0.5, 0.5, 0.5)` for a 1×1×1 box).
    ///
    /// # Returns
    ///
    /// A handle to the created collider.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::default());
    /// physics.add_box_collider(body, Vec3::new(1.0, 1.0, 1.0));
    /// ```
    pub fn add_box_collider(
        &mut self,
        body_handle: RigidBodyHandle,
        half_extents: Vec3,
    ) -> ColliderHandle {
        let collider =
            ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z).build();

        self.collider_set
            .insert_with_parent(collider, body_handle, &mut self.rigid_body_set)
    }

    /// Adds a sphere-shaped collider to a rigid body.
    ///
    /// # Arguments
    ///
    /// * `body_handle` - The rigid body to attach the collider to.
    /// * `radius` - The radius of the sphere.
    ///
    /// # Returns
    ///
    /// A handle to the created collider.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::from_position(Vec3::new(0.0, 5.0, 0.0)));
    /// physics.add_sphere_collider(body, 0.5);
    /// ```
    pub fn add_sphere_collider(
        &mut self,
        body_handle: RigidBodyHandle,
        radius: f32,
    ) -> ColliderHandle {
        let collider = ColliderBuilder::ball(radius).build();

        self.collider_set
            .insert_with_parent(collider, body_handle, &mut self.rigid_body_set)
    }

    /// Removes a rigid body and all colliders attached to it.
    ///
    /// This despawns the body from the simulation: it is unlinked from the
    /// island manager and its joints, and every collider parented to it is
    /// removed too. After this call the `handle` is **stale** — [`get_transform`],
    /// [`get_velocity`], and [`set_velocity`] on it return `None` / are no-ops
    /// (rapier bumps the freed slot's generation, so the handle can no longer
    /// resolve to live storage). See the crate-level *stale-handle safety* and
    /// *handle-ownership* invariants.
    ///
    /// [`get_transform`]: Self::get_transform
    /// [`get_velocity`]: Self::get_velocity
    /// [`set_velocity`]: Self::set_velocity
    ///
    /// # Arguments
    ///
    /// * `handle` - The rigid body to remove. Removing an already-invalid handle
    ///   is a safe no-op and returns `None`.
    ///
    /// # Returns
    ///
    /// - `Some(RigidBody)`: the removed body, if the handle was valid.
    /// - `None`: if the handle was already invalid / removed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::default());
    /// physics.add_box_collider(body, Vec3::new(0.5, 0.5, 0.5));
    ///
    /// physics.remove_body(body);
    /// assert!(physics.get_transform(body).is_none());
    /// ```
    pub fn remove_body(&mut self, handle: RigidBodyHandle) -> Option<RigidBody> {
        // `remove_attached_colliders = true` removes the body's colliders from
        // the collider set as part of the same call, so no collider is left
        // dangling with a freed parent.
        self.rigid_body_set.remove(
            handle,
            &mut self.island_manager,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            true,
        )
    }

    /// Removes a single collider from the simulation.
    ///
    /// The collider is detached from its parent body (if any) and removed from
    /// the collider set; attached bodies are woken so contacts re-evaluate.
    /// After this call the `handle` is stale and no longer resolves.
    ///
    /// # Arguments
    ///
    /// * `handle` - The collider to remove. An already-invalid handle is a safe
    ///   no-op and returns `None`.
    ///
    /// # Returns
    ///
    /// - `Some(Collider)`: the removed collider, if the handle was valid.
    /// - `None`: if the handle was already invalid / removed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::default());
    /// let collider = physics.add_box_collider(body, Vec3::new(0.5, 0.5, 0.5));
    ///
    /// physics.remove_collider(collider);
    /// // The body still exists; only its collider was removed.
    /// assert!(physics.get_transform(body).is_some());
    /// ```
    pub fn remove_collider(&mut self, handle: ColliderHandle) -> Option<Collider> {
        self.collider_set.remove(
            handle,
            &mut self.island_manager,
            &mut self.rigid_body_set,
            true,
        )
    }

    /// Retrieves the current transform of a rigid body.
    ///
    /// Returns the body's current world-space position and rotation. Scale is
    /// always `(1, 1, 1)` because physics bodies have no scale.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the rigid body to query.
    ///
    /// # Returns
    ///
    /// - `Some(Transform)`: the current transform if the body exists.
    /// - `None`: if the handle is invalid or the body was removed (stale-handle safe).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::from_position(Vec3::new(0.0, 5.0, 0.0)));
    ///
    /// if let Some(transform) = physics.get_transform(body) {
    ///     println!("Body position: {:?}", transform.position);
    /// }
    /// ```
    pub fn get_transform(&self, handle: RigidBodyHandle) -> Option<Transform> {
        self.rigid_body_set.get(handle).map(|body| {
            let translation = body.translation();
            let rotation = body.rotation();

            Transform {
                position: Vec3::new(translation.x, translation.y, translation.z),
                rotation: Quat::from_xyzw(rotation.i, rotation.j, rotation.k, rotation.w),
                scale: Vec3::ONE,
            }
        })
    }

    /// Sets the linear velocity of a rigid body.
    ///
    /// Immediately overrides the body's velocity. A stale / invalid handle is a
    /// safe no-op (stale-handle safe).
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the rigid body to modify.
    /// * `velocity` - The new linear velocity in m/s.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::default());
    ///
    /// physics.set_velocity(body, Vec3::new(0.0, 10.0, 0.0));
    /// ```
    pub fn set_velocity(&mut self, handle: RigidBodyHandle, velocity: Vec3) {
        if let Some(body) = self.rigid_body_set.get_mut(handle) {
            body.set_linvel(vector![velocity.x, velocity.y, velocity.z], true);
        }
    }

    /// Teleports a rigid body to a new world-space translation.
    ///
    /// Unlike integrating a velocity, this **sets** the body's position directly
    /// (`wake_up = true`, so contacts re-evaluate). It is the primitive a
    /// floating-origin rebase uses to shift every body by a fixed offset: the
    /// scene layer calls it once per body **between** physics steps, never
    /// mid-solve, so the solver never sees a discontinuous position within a
    /// step (see the rebase-ordering invariant in `kaman-scene`).
    ///
    /// A stale / invalid handle is a safe no-op (stale-handle safe).
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the rigid body to move.
    /// * `translation` - The new world-space position.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::from_position(Vec3::new(0.0, 5.0, 0.0)));
    ///
    /// physics.set_translation(body, Vec3::new(0.0, 2.0, 0.0));
    /// assert_eq!(physics.get_transform(body).unwrap().position, Vec3::new(0.0, 2.0, 0.0));
    /// ```
    pub fn set_translation(&mut self, handle: RigidBodyHandle, translation: Vec3) {
        if let Some(body) = self.rigid_body_set.get_mut(handle) {
            body.set_translation(vector![translation.x, translation.y, translation.z], true);
        }
    }

    /// Gets the current linear velocity of a rigid body.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the rigid body to query.
    ///
    /// # Returns
    ///
    /// - `Some(Vec3)`: the current linear velocity in m/s.
    /// - `None`: if the handle is invalid or the body was removed (stale-handle safe).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_physics::PhysicsWorld;
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut physics = PhysicsWorld::new();
    /// let body = physics.create_dynamic_body(Transform::from_position(Vec3::new(0.0, 5.0, 0.0)));
    ///
    /// physics.step();
    /// if let Some(velocity) = physics.get_velocity(body) {
    ///     println!("Body velocity: {:?}", velocity);
    /// }
    /// ```
    pub fn get_velocity(&self, handle: RigidBodyHandle) -> Option<Vec3> {
        self.rigid_body_set.get(handle).map(|body| {
            let vel = body.linvel();
            Vec3::new(vel.x, vel.y, vel.z)
        })
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Use-after-free / stale-handle guard.
    //
    // Written BEFORE the removal implementation (INTEGRATION §2.7): it defines
    // the contract that `remove_body` / `remove_collider` must satisfy — after
    // removal, every query on the now-stale handle returns `None` / is a no-op
    // and never panics or dereferences freed storage.
    // ---------------------------------------------------------------------

    #[test]
    fn use_after_free_body_queries_return_none() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::from_position(Vec3::new(0.0, 5.0, 0.0)));
        world.add_box_collider(handle, Vec3::new(0.5, 0.5, 0.5));

        // Sanity: the handle is live before removal.
        assert!(world.get_transform(handle).is_some());
        assert!(world.get_velocity(handle).is_some());

        // Free the body (and its colliders).
        let removed = world.remove_body(handle);
        assert!(removed.is_some(), "removing a live body returns it");

        // The handle is now stale. None of these may panic or deref freed data.
        assert!(
            world.get_transform(handle).is_none(),
            "get_transform on a freed handle must be None"
        );
        assert!(
            world.get_velocity(handle).is_none(),
            "get_velocity on a freed handle must be None"
        );
        // set_velocity must be a silent no-op (not a panic).
        world.set_velocity(handle, Vec3::new(1.0, 2.0, 3.0));
        assert!(
            world.get_velocity(handle).is_none(),
            "set_velocity on a freed handle stays a no-op"
        );

        // Stepping after a removal must not touch the freed slot.
        world.step();
        assert!(world.get_transform(handle).is_none());
    }

    #[test]
    fn use_after_free_double_remove_is_safe() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::default());

        assert!(world.remove_body(handle).is_some());
        // Removing an already-freed handle is a safe no-op returning None.
        assert!(world.remove_body(handle).is_none());
    }

    #[test]
    fn remove_body_frees_attached_colliders() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::default());
        let collider = world.add_box_collider(handle, Vec3::new(0.5, 0.5, 0.5));
        assert_eq!(world.collider_set.len(), 1);

        world.remove_body(handle);

        // The attached collider is gone, and its handle is stale.
        assert_eq!(world.collider_set.len(), 0);
        assert!(world.collider_set.get(collider).is_none());
    }

    #[test]
    fn remove_collider_keeps_body_alive() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::default());
        let collider = world.add_sphere_collider(handle, 0.5);
        assert_eq!(world.collider_set.len(), 1);

        let removed = world.remove_collider(collider);
        assert!(removed.is_some());
        assert_eq!(world.collider_set.len(), 0);
        assert!(world.collider_set.get(collider).is_none());

        // The body itself is untouched.
        assert!(world.get_transform(handle).is_some());
    }

    #[test]
    fn remove_collider_double_remove_is_safe() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::default());
        let collider = world.add_box_collider(handle, Vec3::new(0.5, 0.5, 0.5));

        assert!(world.remove_collider(collider).is_some());
        assert!(world.remove_collider(collider).is_none());
    }

    #[test]
    fn removed_slot_reuse_does_not_alias_old_handle() {
        // rapier reuses freed slots but bumps their generation, so an old
        // handle must not resolve to the freshly-inserted body.
        let mut world = PhysicsWorld::new();
        let old = world.create_dynamic_body(Transform::from_position(Vec3::new(9.0, 9.0, 9.0)));
        world.remove_body(old);

        let new = world.create_dynamic_body(Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
        assert!(world.get_transform(old).is_none(), "old handle stays stale");
        assert_eq!(
            world.get_transform(new).unwrap().position,
            Vec3::new(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn set_translation_moves_body_and_is_stale_safe() {
        let mut world = PhysicsWorld::new();
        let handle = world.create_dynamic_body(Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));

        world.set_translation(handle, Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(
            world.get_transform(handle).unwrap().position,
            Vec3::new(4.0, 5.0, 6.0)
        );

        // A freed handle is a safe no-op, never a panic.
        world.remove_body(handle);
        world.set_translation(handle, Vec3::new(9.0, 9.0, 9.0));
        assert!(world.get_transform(handle).is_none());
    }

    #[test]
    fn fixed_dt_is_sixtieth_of_a_second() {
        // Guards the local constant against drift from kaman_core::FIXED_DT.
        // (kaman-physics does not depend on kaman-core to avoid a cycle, so the
        // value is mirrored here and pinned by this test.)
        assert_eq!(FIXED_DT, 1.0 / 60.0);
    }

    // ---------------------------------------------------------------------
    // Migrated behavior tests (unchanged from the prototype).
    // ---------------------------------------------------------------------

    #[test]
    fn test_create_physics_world() {
        let world = PhysicsWorld::new();
        assert_eq!(world.gravity, Vec3::new(0.0, -9.81, 0.0));
        assert_eq!(world.rigid_body_set.len(), 0);
    }

    #[test]
    fn test_create_dynamic_body() {
        let mut world = PhysicsWorld::new();
        let transform = Transform::from_position(Vec3::new(0.0, 10.0, 0.0));
        let handle = world.create_dynamic_body(transform);

        assert_eq!(world.rigid_body_set.len(), 1);

        let body_transform = world.get_transform(handle).unwrap();
        assert_eq!(body_transform.position, Vec3::new(0.0, 10.0, 0.0));
    }

    #[test]
    fn test_gravity_affects_body() {
        let mut world = PhysicsWorld::new();
        let transform = Transform::from_position(Vec3::new(0.0, 10.0, 0.0));
        let handle = world.create_dynamic_body(transform);
        world.add_box_collider(handle, Vec3::new(0.5, 0.5, 0.5));

        let initial_pos = world.get_transform(handle).unwrap().position;

        for _ in 0..60 {
            world.step();
        }

        let final_pos = world.get_transform(handle).unwrap().position;
        assert!(final_pos.y < initial_pos.y, "Body should fall due to gravity");
    }

    #[test]
    fn test_static_body_doesnt_move() {
        let mut world = PhysicsWorld::new();
        let transform = Transform::from_position(Vec3::new(0.0, 0.0, 0.0));
        let handle = world.create_static_body(transform);
        world.add_box_collider(handle, Vec3::new(10.0, 0.5, 10.0));

        let initial_pos = world.get_transform(handle).unwrap().position;

        for _ in 0..60 {
            world.step();
        }

        let final_pos = world.get_transform(handle).unwrap().position;
        assert_eq!(initial_pos, final_pos, "Static body should not move");
    }

    #[test]
    fn test_velocity() {
        let mut world = PhysicsWorld::new();
        let transform = Transform::from_position(Vec3::new(0.0, 10.0, 0.0));
        let handle = world.create_dynamic_body(transform);
        world.add_sphere_collider(handle, 1.0);

        world.set_velocity(handle, Vec3::new(5.0, 0.0, 0.0));

        let velocity = world.get_velocity(handle).unwrap();
        assert_eq!(velocity, Vec3::new(5.0, 0.0, 0.0));

        world.step();
        let new_velocity = world.get_velocity(handle).unwrap();
        assert!(
            new_velocity.y < 0.0,
            "Y velocity should be negative due to gravity"
        );
    }

    #[test]
    fn test_collision_detection() {
        let mut world = PhysicsWorld::new();

        let ground_transform = Transform::from_position(Vec3::new(0.0, -0.5, 0.0));
        let ground_handle = world.create_static_body(ground_transform);
        world.add_box_collider(ground_handle, Vec3::new(10.0, 0.5, 10.0));

        let box_transform = Transform::from_position(Vec3::new(0.0, 5.0, 0.0));
        let box_handle = world.create_dynamic_body(box_transform);
        world.add_box_collider(box_handle, Vec3::new(0.5, 0.5, 0.5));

        for _ in 0..120 {
            world.step();
        }

        let final_pos = world.get_transform(box_handle).unwrap().position;
        assert!(
            final_pos.y > 0.3 && final_pos.y < 1.0,
            "Box should rest on ground, y={}",
            final_pos.y
        );
    }
}
