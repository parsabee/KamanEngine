// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`Game`] trait — the engine/game boundary.
//!
//! A game is anything that implements [`Game`]. The engine owns the event loop
//! and drives the game through three lifecycle hooks; the game owns all
//! game-specific state (its structs, its rules) and reaches engine services only
//! through the [`EngineCtx`](crate::EngineCtx) it is handed. This is the seam
//! that keeps game concepts (vehicles, tracks, tallies, …) out of the engine crates
//! (ARCHITECTURE §3): the engine depends on the *trait*, never on a concrete
//! game.

use crate::context::EngineCtx;

/// A KamanEngine game: implement this to be driven by the engine loop.
///
/// The engine calls the three hooks in a fixed order (see below). All engine
/// access flows through the [`EngineCtx`] argument — a game never touches a
/// window, a Metal device, or any other engine internal directly.
///
/// # Call ordering (invariant)
///
/// For a driver that runs `N` frames:
///
/// 1. [`init`](Self::init) is called **exactly once**, before the first
///    [`update`](Self::update).
/// 2. Then, for each of the `N` frames, [`update`](Self::update) is called
///    (with that frame's `dt`), immediately followed by
///    [`render`](Self::render).
///
/// So the full sequence is `init` → (`update`, `render`) × `N`. `update` and
/// `render` are always paired and always in that order; `render` never runs
/// before `init`, and `init` never runs twice. The
/// [`headless`](crate::headless) driver and the windowed
/// [`run`](crate::run) entry both honor this contract, and a test in the driver
/// module enforces it.
///
/// # Separation of concerns
///
/// - [`update`](Self::update) advances game state (mutate the ECS world, read
///   input). It should not record draws.
/// - [`render`](Self::render) records the frame through the render seam
///   ([`EngineCtx::renderer`](crate::EngineCtx::renderer)). It should not change
///   game state.
///
/// The fixed-timestep split (multiple `update`s per `render`) is refined in
/// KE-0201; in Phase 1 it is one `update` per `render`.
///
/// # Example
///
/// A minimal game that spawns one entity in `init` and nudges it every frame:
///
/// ```rust
/// use kaman_core::{Game, EngineCtx, headless};
/// use kaman_ecs::TransformComponent;
/// use kaman_math::glam::Vec3;
///
/// struct MyGame {
///     entity: Option<kaman_ecs::hecs::Entity>,
/// }
///
/// impl Game for MyGame {
///     fn init(&mut self, ctx: &mut EngineCtx) {
///         // Spawn engine-generic ECS entities using the shared component vocabulary.
///         let e = ctx.world_mut().spawn((
///             TransformComponent::from_position(Vec3::ZERO),
///         ));
///         self.entity = Some(e);
///     }
///
///     fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
///         if let Some(e) = self.entity {
///             if let Ok(mut t) = ctx.world_mut().get::<&mut TransformComponent>(e) {
///                 t.transform.position.x += dt;
///             }
///         }
///     }
///
///     fn render(&mut self, _ctx: &mut EngineCtx) {
///         // Recording draws through the seam is optional in Phase 1.
///     }
/// }
///
/// // Drive two frames headlessly (no window, no GPU).
/// let mut game = MyGame { entity: None };
/// headless::run(&mut game, 2);
/// ```
pub trait Game {
    /// One-time setup, called once before the first [`update`](Self::update).
    ///
    /// Spawn the game's initial ECS entities and create any render resources
    /// (meshes/pipelines) via [`EngineCtx::renderer`](crate::EngineCtx::renderer)
    /// here. Called exactly once per run.
    fn init(&mut self, ctx: &mut EngineCtx);

    /// Advance game state by `dt` seconds. Called once per frame, before
    /// [`render`](Self::render).
    ///
    /// `dt` is the time elapsed since the previous frame, in seconds. Read input
    /// via [`EngineCtx::input`](crate::EngineCtx::input) and mutate the ECS world
    /// via [`EngineCtx::world_mut`](crate::EngineCtx::world_mut). Do not record
    /// draws here — that is [`render`](Self::render)'s job.
    fn update(&mut self, ctx: &mut EngineCtx, dt: f32);

    /// Record the frame. Called once per frame, immediately after
    /// [`update`](Self::update).
    ///
    /// Record draws through the render seam
    /// ([`EngineCtx::renderer`](crate::EngineCtx::renderer)) following the
    /// `begin_frame` → `set_pipeline`/`draw_mesh` → `submit` protocol. This hook
    /// may be a no-op in Phase 1. Do not mutate game state here.
    fn render(&mut self, ctx: &mut EngineCtx);
}
