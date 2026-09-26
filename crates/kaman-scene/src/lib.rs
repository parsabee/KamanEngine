// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`Scene`] — owner of the ECS [`World`] and the [`PhysicsWorld`], plus the
//! engine-generic **world-streaming** layer for KamanEngine.
//!
//! A [`Scene`] bundles the two simulation stores a running world needs — a hecs
//! [`World`] of entities/components and a
//! [`PhysicsWorld`] of rigid bodies — behind one
//! type, and drives them in lockstep at the engine's fixed timestep
//! ([`step_physics`](Scene::step_physics), stepping at
//! [`kaman_physics::FIXED_DT`]).
//!
//! On top of that it provides three things an endless-world game needs, all
//! **engine-generic** (there are deliberately no game concepts here — no vehicle,
//! track, tally, hazard, or gameplay-lattice types; a test scans this crate's
//! source to keep it that way):
//!
//! 1. **Spawn-ahead / despawn-behind streaming** (`stream`(Scene::stream)). The
//!    *game* supplies a **focus point** (typically the player's position) and a
//!    **spawn callback** that says *what* to spawn; the *scene* owns the
//!    bookkeeping — which slots ahead of the focus are already filled and which
//!    streamed entities have fallen behind the despawn threshold.
//! 2. **Atomic despawn** ([`despawn`](Scene::despawn), used by streaming). Removing
//!    a streamed entity removes its ECS entity **and** its physics rigid body (via
//!    [`PhysicsWorld::remove_body`]) together, so no live
//!    [`PhysicsBodyComponent`] is ever left
//!    holding a freed handle (the KE-0202 / KE-0005 handle-ownership invariant).
//! 3. **Floating-origin rebase** ([`maybe_rebase`](Scene::maybe_rebase)). When the
//!    focus drifts past a threshold from the origin, every position — ECS
//!    [`TransformComponent`] *and* every physics
//!    body translation — is shifted by a fixed offset back toward the origin, so
//!    world coordinates never grow into float-precision error. Relative positions
//!    are preserved, so gameplay is unaffected.
//!
//! # Streaming is deterministic
//!
//! Every streaming decision is a pure function of the focus position and the
//! [`StreamingConfig`]: the same focus trajectory always spawns the same slots and
//! despawns the same entities, and rebase fires at the same crossings. This keeps
//! a run reproducible (a prerequisite for any future golden test).
//!
//! # Rebase-ordering invariant
//!
//! A rebase teleports rigid bodies ([`PhysicsWorld::set_translation`]). To avoid
//! solver artifacts it must run **between** physics steps, never inside one:
//! [`step_physics`](Scene::step_physics) completes a full solve, then
//! [`maybe_rebase`](Scene::maybe_rebase) shifts everything, then the next
//! [`step_physics`](Scene::step_physics) begins from the shifted (consistent)
//! state. The recommended per-fixed-update order is therefore
//! `stream` → `step_physics` → `maybe_rebase`. Driving the shift mid-solve would
//! feed the constraint solver a discontinuous position within a single step; the
//! documented order prevents that.

#![deny(missing_docs)]

use kaman_ecs::hecs::{Entity, World};
use kaman_ecs::{DynamicTag, PhysicsBodyComponent, TransformComponent};
use kaman_math::glam::Vec3;
use kaman_physics::{PhysicsWorld, FIXED_DT};

/// Configuration for the [`Scene`]'s spawn-ahead / despawn-behind streaming and
/// its floating-origin rebase, along a single travel axis.
///
/// Streaming is one-dimensional: content is generated *ahead* of a focus point
/// along [`axis`](Self::axis) and discarded once it falls *behind*. All distances
/// are measured as the signed projection of a position onto that axis. The
/// defaults ([`StreamingConfig::default`]) stream forward along `-Z` — the
/// convention an endless runner heading "into the screen" uses — but any unit
/// axis works.
///
/// # Example
///
/// ```rust
/// use kaman_scene::StreamingConfig;
/// use kaman_math::glam::Vec3;
///
/// let cfg = StreamingConfig {
///     axis: Vec3::new(0.0, 0.0, -1.0),
///     spawn_interval: 6.0,
///     spawn_ahead: 60.0,
///     despawn_behind: 12.0,
///     rebase_threshold: 1000.0,
/// };
/// assert!(cfg.spawn_ahead > cfg.spawn_interval);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamingConfig {
    /// Unit direction of travel. Positions are projected onto this axis to get a
    /// scalar "distance along the world". Should be normalized; a zero axis
    /// disables spawning/despawning (every projection collapses to `0`).
    pub axis: Vec3,
    /// Spacing between spawn slots along [`axis`](Self::axis), in world units.
    /// Streaming fills one slot per multiple of this interval ahead of the focus.
    pub spawn_interval: f32,
    /// How far ahead of the focus (along the axis) to keep filled, in world units.
    pub spawn_ahead: f32,
    /// How far behind the focus (along the axis) a streamed entity may fall before
    /// it is despawned, in world units.
    pub despawn_behind: f32,
    /// Distance the focus may travel from the origin (along the axis) before a
    /// floating-origin rebase shifts everything back by that much.
    pub rebase_threshold: f32,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            axis: Vec3::new(0.0, 0.0, -1.0),
            spawn_interval: 6.0,
            spawn_ahead: 60.0,
            despawn_behind: 12.0,
            rebase_threshold: 1000.0,
        }
    }
}

/// The engine-generic simulation scene: an ECS [`World`] plus a
/// [`PhysicsWorld`], with spawn/despawn streaming and floating-origin rebase.
///
/// Construct with [`new`](Self::new) (default [`StreamingConfig`]) or
/// [`with_config`](Self::with_config). Access the stores through
/// [`world`](Self::world) / [`world_mut`](Self::world_mut) and
/// [`physics`](Self::physics) / [`physics_mut`](Self::physics_mut), advance
/// physics with [`step_physics`](Self::step_physics), and drive streaming each
/// fixed update with `stream`(Self::stream) and [`maybe_rebase`](Self::maybe_rebase).
///
/// # Example
///
/// ```rust
/// use kaman_scene::Scene;
/// use kaman_ecs::TransformComponent;
/// use kaman_math::glam::Vec3;
///
/// let mut scene = Scene::new();
/// scene.world_mut().spawn((TransformComponent::from_position(Vec3::ZERO),));
/// scene.step_physics();
/// assert_eq!(scene.world().len(), 1);
/// ```
pub struct Scene {
    world: World,
    physics: PhysicsWorld,
    config: StreamingConfig,
    /// Entities the scene created via `stream`; candidates for despawn-behind.
    /// Ownership: the scene tracks these so it can remove them atomically.
    streamed: Vec<Entity>,
    /// The furthest slot index (along the axis) that has already been spawned.
    /// `None` until the first `stream` call seeds the frontier from the focus.
    spawn_frontier: Option<i64>,
    /// Total offset (along the axis, in slot-distance) that rebases have shifted
    /// the world by, so slot indexing stays consistent across a rebase.
    rebase_shift_units: f32,
}

impl Scene {
    /// Creates an empty scene with the default [`StreamingConfig`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(StreamingConfig::default())
    }

    /// Creates an empty scene with an explicit streaming configuration.
    #[must_use]
    pub fn with_config(config: StreamingConfig) -> Self {
        Self {
            world: World::new(),
            physics: PhysicsWorld::new(),
            config,
            streamed: Vec::new(),
            spawn_frontier: None,
            rebase_shift_units: 0.0,
        }
    }

    /// Shared access to the ECS [`World`].
    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Mutable access to the ECS [`World`] — spawn, despawn, mutate components.
    #[must_use]
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Shared access to the [`PhysicsWorld`].
    #[must_use]
    pub fn physics(&self) -> &PhysicsWorld {
        &self.physics
    }

    /// Mutable access to the [`PhysicsWorld`] — create bodies, add colliders.
    #[must_use]
    pub fn physics_mut(&mut self) -> &mut PhysicsWorld {
        &mut self.physics
    }

    /// The active [`StreamingConfig`].
    #[must_use]
    pub fn config(&self) -> StreamingConfig {
        self.config
    }

    /// Number of entities the scene is currently tracking for despawn-behind.
    ///
    /// This is the count of entities spawned through `stream` that have not yet
    /// fallen behind the despawn threshold. Bounded streaming keeps this flat.
    #[must_use]
    pub fn streamed_count(&self) -> usize {
        self.streamed.len()
    }

    /// Advances physics by one fixed timestep ([`FIXED_DT`]) and syncs transforms.
    ///
    /// Steps the [`PhysicsWorld`] once, then copies each dynamic body's simulated
    /// transform back into its entity's [`TransformComponent`] (only entities
    /// carrying a [`PhysicsBodyComponent`] and a
    /// [`DynamicTag`] are synced; static geometry keeps its
    /// authored transform). Call this exactly once per fixed engine update so the
    /// solver stays in lockstep with `kaman_core`'s fixed-timestep driver.
    ///
    /// A stale handle on a component (should not occur under the atomic-despawn
    /// invariant) is skipped safely — [`PhysicsWorld::get_transform`] returns
    /// `None` — rather than panicking.
    pub fn step_physics(&mut self) {
        self.physics.step();

        for (_e, (transform, body, _dynamic)) in self
            .world
            .query_mut::<(&mut TransformComponent, &PhysicsBodyComponent, &DynamicTag)>()
        {
            if let Some(t) = self.physics.get_transform(body.handle) {
                transform.transform = t;
            }
        }
    }

    /// The fixed timestep [`step_physics`](Self::step_physics) advances by.
    ///
    /// Re-exported through the scene for callers that drive the loop; equal to
    /// [`kaman_physics::FIXED_DT`] (and thus `kaman_core::FIXED_DT`).
    #[must_use]
    pub const fn fixed_dt() -> f32 {
        FIXED_DT
    }

    /// Despawns an entity **atomically**: removes it from the ECS world and, if it
    /// carries a [`PhysicsBodyComponent`], removes that rigid body from the
    /// physics world in the same call.
    ///
    /// This upholds the handle-ownership invariant (KE-0202 / KE-0005): the ECS
    /// entity and its physics body are torn down together, so no live component is
    /// ever left holding a freed [`RigidBodyHandle`](kaman_physics::RigidBodyHandle),
    /// and no rigid body outlives the entity that owned it. The body is removed
    /// **before** the entity, so even a mid-teardown observer never sees the
    /// component without its body.
    ///
    /// Returns `true` if the entity existed and was removed, `false` if the handle
    /// was already gone (a safe no-op).
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.world.contains(entity) {
            return false;
        }
        // Remove the physics body first (if any), then the entity — the two
        // freed together, no stale handle in between.
        if let Ok(body) = self.world.get::<&PhysicsBodyComponent>(entity) {
            let handle = body.handle;
            drop(body);
            self.physics.remove_body(handle);
        }
        self.world.despawn(entity).is_ok()
    }

    /// Runs one streaming pass for the current `focus` position.
    ///
    /// Two phases, both pure functions of `focus` and the [`StreamingConfig`]:
    ///
    /// 1. **Despawn-behind.** Every streamed entity whose distance along the axis
    ///    has fallen more than [`despawn_behind`](StreamingConfig::despawn_behind)
    ///    behind the focus is removed atomically via [`despawn`](Self::despawn).
    /// 2. **Spawn-ahead.** For each not-yet-filled slot between the current
    ///    frontier and `focus + spawn_ahead` (slots spaced
    ///    [`spawn_interval`](StreamingConfig::spawn_interval) apart along the
    ///    axis), `spawn` is invoked with a [`SpawnCtx`] positioned at that slot.
    ///    Whatever entities the callback reports via
    ///    [`SpawnCtx::spawned`](SpawnCtx::spawned) become tracked streamed
    ///    entities (candidates for a later despawn).
    ///
    /// The `spawn` closure receives mutable access to the ECS world and physics
    /// world (through [`SpawnCtx`]) plus the world-space slot position, and decides
    /// *what* to spawn — the scene stays free of game content.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_scene::{Scene, StreamingConfig};
    /// use kaman_ecs::TransformComponent;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut scene = Scene::with_config(StreamingConfig {
    ///     spawn_interval: 10.0,
    ///     spawn_ahead: 30.0,
    ///     despawn_behind: 10.0,
    ///     ..Default::default()
    /// });
    ///
    /// scene.stream(Vec3::ZERO, |cx| {
    ///     let e = cx.world.spawn((TransformComponent::from_position(cx.position),));
    ///     cx.spawned(e);
    /// });
    /// assert!(scene.streamed_count() > 0);
    /// ```
    pub fn stream<F>(&mut self, focus: Vec3, mut spawn: F)
    where
        F: FnMut(&mut SpawnCtx<'_>),
    {
        let axis = self.config.axis;
        let focus_d = focus.dot(axis);

        // --- Phase 1: despawn-behind ---------------------------------------
        let cutoff = focus_d - self.config.despawn_behind;
        // Collect victims first so we don't mutate the world while borrowing it.
        let mut kept = Vec::with_capacity(self.streamed.len());
        let mut victims = Vec::new();
        for &e in &self.streamed {
            let behind = match self.world.get::<&TransformComponent>(e) {
                Ok(t) => t.transform.position.dot(axis) < cutoff,
                // Entity already gone (removed by game): drop from tracking.
                Err(_) => true,
            };
            if behind {
                victims.push(e);
            } else {
                kept.push(e);
            }
        }
        self.streamed = kept;
        for e in victims {
            self.despawn(e);
        }

        // --- Phase 2: spawn-ahead ------------------------------------------
        let interval = self.config.spawn_interval;
        if interval <= 0.0 || self.config.spawn_ahead <= 0.0 {
            return;
        }

        // Slot indices are measured in "rebased" world space: the running
        // rebase_shift keeps the frontier consistent across an origin rebase.
        let target = focus_d + self.config.spawn_ahead + self.rebase_shift_units;
        let target_slot = (target / interval).floor() as i64;

        // Seed the frontier just behind the focus on the first pass so we don't
        // retroactively fill the whole world behind the player.
        let start = match self.spawn_frontier {
            Some(f) => f + 1,
            None => ((focus_d + self.rebase_shift_units) / interval).floor() as i64,
        };

        for slot in start..=target_slot {
            let along = (slot as f32) * interval - self.rebase_shift_units;
            let position = axis * along;
            let mut cx = SpawnCtx {
                world: &mut self.world,
                physics: &mut self.physics,
                position,
                slot,
                out: Vec::new(),
            };
            spawn(&mut cx);
            let spawned = cx.out;
            self.streamed.extend(spawned);
            self.spawn_frontier = Some(slot);
        }
    }

    /// Applies a floating-origin rebase if the focus has drifted past the
    /// threshold, shifting **all** positions — ECS transforms *and* physics body
    /// translations — back toward the origin by a fixed offset.
    ///
    /// The shift is a whole multiple of the focus's distance along the axis,
    /// quantized to [`rebase_threshold`](StreamingConfig::rebase_threshold), so
    /// the focus is pulled back to within one threshold of the origin. Relative
    /// positions between any two entities are preserved exactly (every position
    /// moves by the same vector), so gameplay never notices.
    ///
    /// Must be called **between** physics steps, not inside one (see the crate-
    /// level rebase-ordering invariant): it teleports rigid bodies via
    /// [`PhysicsWorld::set_translation`], which is only safe outside a solve.
    ///
    /// Returns the offset vector actually applied (`Vec3::ZERO` if no rebase was
    /// due this call).
    pub fn maybe_rebase(&mut self, focus: Vec3) -> Vec3 {
        let axis = self.config.axis;
        let threshold = self.config.rebase_threshold;
        if threshold <= 0.0 {
            return Vec3::ZERO;
        }

        let focus_d = focus.dot(axis);
        if focus_d.abs() < threshold {
            return Vec3::ZERO;
        }

        // Quantize the shift to whole thresholds so the operation is
        // deterministic and slot indexing stays aligned.
        let steps = (focus_d / threshold).trunc();
        let shift_along = steps * threshold;
        if shift_along == 0.0 {
            return Vec3::ZERO;
        }
        // Move everything back by `shift_along` along the axis.
        let offset = axis * (-shift_along);

        // Shift ECS transforms.
        for (_e, transform) in self.world.query_mut::<&mut TransformComponent>() {
            transform.transform.position += offset;
        }

        // Shift physics bodies to match, between steps.
        let handles: Vec<_> = self
            .world
            .query::<&PhysicsBodyComponent>()
            .iter()
            .map(|(_e, b)| b.handle)
            .collect();
        for handle in handles {
            if let Some(t) = self.physics.get_transform(handle) {
                self.physics.set_translation(handle, t.position + offset);
            }
        }

        // Track the shift so slot indexing stays consistent with pre-rebase space.
        self.rebase_shift_units += shift_along;

        offset
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

/// The handle a `stream`(Scene::stream) spawn callback uses to populate one slot.
///
/// It grants mutable access to the scene's ECS [`World`] and [`PhysicsWorld`] and
/// carries the world-space [`position`](Self::position) of the slot being filled.
/// The callback creates whatever entities/bodies it wants and reports each one it
/// wants the scene to *track* (so it participates in despawn-behind) via
/// [`spawned`](Self::spawned). Entities the callback creates but does not report
/// are left untracked (e.g. permanent scenery).
pub struct SpawnCtx<'a> {
    /// Mutable access to the ECS world for spawning entities.
    pub world: &'a mut World,
    /// Mutable access to the physics world for creating bodies/colliders.
    pub physics: &'a mut PhysicsWorld,
    /// World-space position of the slot being filled.
    pub position: Vec3,
    /// The integer slot index being filled (monotonic along the axis).
    pub slot: i64,
    out: Vec<Entity>,
}

impl SpawnCtx<'_> {
    /// Registers `entity` as a streamed entity so the scene despawns it (with its
    /// physics body) once it falls behind the despawn threshold.
    pub fn spawned(&mut self, entity: Entity) {
        self.out.push(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaman_ecs::{DynamicTag, PhysicsBodyComponent, TransformComponent};
    use kaman_math::glam::Vec3;

    /// Helper: spawn a dynamic entity with a physics body at `position`.
    fn spawn_body(scene: &mut Scene, position: Vec3) -> Entity {
        use kaman_math::Transform;
        let handle = scene
            .physics_mut()
            .create_dynamic_body(Transform::from_position(position));
        scene
            .physics_mut()
            .add_box_collider(handle, Vec3::new(0.5, 0.5, 0.5));
        scene.world_mut().spawn((
            TransformComponent::from_position(position),
            PhysicsBodyComponent::new(handle),
            DynamicTag,
        ))
    }

    #[test]
    fn step_physics_syncs_dynamic_transform_from_body() {
        let mut scene = Scene::new();
        let pos = Vec3::new(0.0, 10.0, 0.0);
        let e = spawn_body(&mut scene, pos);

        for _ in 0..30 {
            scene.step_physics();
        }
        let y = scene
            .world()
            .get::<&TransformComponent>(e)
            .unwrap()
            .transform
            .position
            .y;
        assert!(y < 10.0, "body fell under gravity and synced to ECS: y={y}");
    }

    /// Despawn atomicity: after a streamed entity crosses the despawn threshold,
    /// (a) the ECS entity is gone, (b) its physics body handle is stale, and
    /// (c) no live `PhysicsBodyComponent` still references that handle.
    #[test]
    fn despawn_removes_entity_and_physics_body_atomically() {
        let mut scene = Scene::new();
        let pos = Vec3::new(0.0, 0.0, 0.0);
        let e = spawn_body(&mut scene, pos);
        let handle = scene
            .world()
            .get::<&PhysicsBodyComponent>(e)
            .unwrap()
            .handle;

        assert!(scene.physics().get_transform(handle).is_some());

        assert!(scene.despawn(e));

        // (a) entity gone
        assert!(!scene.world().contains(e));
        // (b) physics handle stale
        assert!(scene.physics().get_transform(handle).is_none());
        // (c) no live component references it
        let alive = scene
            .world()
            .query::<&PhysicsBodyComponent>()
            .iter()
            .any(|(_e, b)| b.handle == handle);
        assert!(!alive, "no live PhysicsBodyComponent may hold a freed handle");
        // double-despawn is a safe no-op
        assert!(!scene.despawn(e));
    }

    /// Streaming spawns ahead of the focus and despawns behind it, atomically.
    #[test]
    fn stream_spawns_ahead_and_despawns_behind() {
        let mut scene = Scene::with_config(StreamingConfig {
            axis: Vec3::new(0.0, 0.0, -1.0),
            spawn_interval: 10.0,
            spawn_ahead: 30.0,
            despawn_behind: 10.0,
            rebase_threshold: 100_000.0,
        });

        // First pass at origin: fills slots ahead (along -Z).
        scene.stream(Vec3::ZERO, |cx| {
            let e = cx.world.spawn((TransformComponent::from_position(cx.position),));
            cx.spawned(e);
        });
        let after_first = scene.streamed_count();
        assert!(after_first > 0, "spawned ahead on the first pass");

        // All streamed entities are ahead (negative Z, matching -Z axis).
        for &e in &scene.streamed {
            let z = scene.world().get::<&TransformComponent>(e).unwrap().transform.position.z;
            assert!(z <= 0.0, "streamed entity is ahead along -Z, z={z}");
        }

        // Advance the focus one interval per pass (realistic streaming), so the
        // frontier never has to close a large gap in one call.
        let mut last_focus = Vec3::ZERO;
        for step in 1..=20 {
            last_focus = Vec3::new(0.0, 0.0, -(step as f32) * 10.0);
            scene.stream(last_focus, |cx| {
                let e = cx.world.spawn((TransformComponent::from_position(cx.position),));
                cx.spawned(e);
            });
        }

        // Under gradual advance, every surviving entity sits within the streaming
        // window: no further behind than `despawn_behind` and no further ahead
        // than `spawn_ahead` of the focus.
        let axis = scene.config().axis;
        let focus_along = last_focus.dot(axis);
        for &e in &scene.streamed {
            let along = scene
                .world()
                .get::<&TransformComponent>(e)
                .unwrap()
                .transform
                .position
                .dot(axis);
            assert!(
                along >= focus_along - scene.config().despawn_behind - 1e-3,
                "survivor {along} must not be behind the despawn cutoff {}",
                focus_along - scene.config().despawn_behind
            );
            assert!(
                along <= focus_along + scene.config().spawn_ahead + 1e-3,
                "survivor {along} must not be beyond the spawn-ahead horizon"
            );
        }
    }

    /// No unbounded growth: driving many frames of advancing focus keeps both the
    /// ECS entity count and the physics body count bounded.
    #[test]
    fn bounded_counts_over_many_frames() {
        let cfg = StreamingConfig {
            axis: Vec3::new(0.0, 0.0, -1.0),
            spawn_interval: 5.0,
            spawn_ahead: 50.0,
            despawn_behind: 10.0,
            rebase_threshold: 100_000.0,
        };
        let mut scene = Scene::with_config(cfg);

        // Theoretical bound: slots in [focus - despawn_behind, focus + spawn_ahead]
        // = (despawn_behind + spawn_ahead) / spawn_interval, plus a small margin.
        let window = cfg.despawn_behind + cfg.spawn_ahead;
        let bound = (window / cfg.spawn_interval).ceil() as usize + 3;

        let mut max_entities = 0usize;
        let mut max_bodies = 0usize;

        // Advance the focus one interval per "frame" for many frames.
        for frame in 0..2000i64 {
            let focus = Vec3::new(0.0, 0.0, -(frame as f32) * cfg.spawn_interval);
            scene.stream(focus, |cx| {
                use kaman_math::Transform;
                let h = cx.physics.create_dynamic_body(Transform::from_position(cx.position));
                let e = cx.world.spawn((
                    TransformComponent::from_position(cx.position),
                    PhysicsBodyComponent::new(h),
                ));
                cx.spawned(e);
            });
            max_entities = max_entities.max(scene.world().len() as usize);
            max_bodies = max_bodies.max(scene.physics().rigid_body_set.len());
        }

        assert!(
            max_entities <= bound,
            "entity count stayed bounded: max={max_entities}, bound={bound}"
        );
        assert!(
            max_bodies <= bound,
            "physics body count stayed bounded: max={max_bodies}, bound={bound}"
        );
        // And no leaked bodies: live bodies == tracked entities at the end.
        assert_eq!(scene.physics().rigid_body_set.len(), scene.streamed_count());
    }

    /// Origin rebase preserves relative positions of two entities across the shift,
    /// for both ECS transforms and physics body translations.
    #[test]
    fn rebase_preserves_relative_positions() {
        use kaman_math::Transform;
        let cfg = StreamingConfig {
            axis: Vec3::new(0.0, 0.0, -1.0),
            spawn_interval: 10.0,
            spawn_ahead: 10.0,
            despawn_behind: 1e9, // never despawn during this test
            rebase_threshold: 1000.0,
        };
        let mut scene = Scene::with_config(cfg);

        // Two entities with physics bodies, far out along -Z.
        let a_pos = Vec3::new(1.0, 2.0, -1500.0);
        let b_pos = Vec3::new(4.0, 2.0, -1490.0);
        let ha = scene.physics_mut().create_dynamic_body(Transform::from_position(a_pos));
        let hb = scene.physics_mut().create_dynamic_body(Transform::from_position(b_pos));
        let a = scene.world_mut().spawn((
            TransformComponent::from_position(a_pos),
            PhysicsBodyComponent::new(ha),
        ));
        let b = scene.world_mut().spawn((
            TransformComponent::from_position(b_pos),
            PhysicsBodyComponent::new(hb),
        ));

        let rel_ecs_before = b_pos - a_pos;

        // Focus is out past the threshold -> rebase fires.
        let offset = scene.maybe_rebase(Vec3::new(0.0, 0.0, -1500.0));
        assert_ne!(offset, Vec3::ZERO, "a rebase should have occurred");

        // ECS relative position preserved.
        let a_after = scene.world().get::<&TransformComponent>(a).unwrap().transform.position;
        let b_after = scene.world().get::<&TransformComponent>(b).unwrap().transform.position;
        assert!((b_after - a_after - rel_ecs_before).length() < 1e-3);

        // Both moved back toward the origin (smaller |z|).
        assert!(a_after.z.abs() < a_pos.z.abs());

        // Physics body relative position preserved and matches ECS.
        let pa = scene.physics().get_transform(ha).unwrap().position;
        let pb = scene.physics().get_transform(hb).unwrap().position;
        assert!((pb - pa - rel_ecs_before).length() < 1e-3);
        assert!((pa - a_after).length() < 1e-3, "physics body tracks ECS after rebase");
    }

    /// No rebase fires while the focus stays within the threshold.
    #[test]
    fn no_rebase_within_threshold() {
        let mut scene = Scene::new();
        let offset = scene.maybe_rebase(Vec3::new(0.0, 0.0, -10.0));
        assert_eq!(offset, Vec3::ZERO);
    }

    /// A2 boundary guard: `kaman-scene` must stay engine-generic — no game-named
    /// symbol may appear anywhere in its source. Mirrors the `kaman-ecs` /
    /// `kaman-core` guards: forbidden words are built from ASCII byte codes so
    /// this test's own body contains no literal occurrence, letting it scan the
    /// whole file (including itself) with no exclusion window.
    #[test]
    fn no_game_specific_symbols() {
        let src = include_str!("lib.rs");

        // Codes spell the forbidden game concepts:
        //   [99,97,114], [114,111,97,100], [115,99,111,114,101],
        //   [111,98,115,116,97,99,108,101], [108,97,110,101]
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
                     kaman-scene must stay engine-generic (A2 boundary)",
                );
            }
        }
    }
}
