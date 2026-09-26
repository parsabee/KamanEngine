// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`Game`] trait — the engine/game boundary.
//!
//! A game is anything that implements [`Game`]. The engine owns the event loop
//! and drives the game through three lifecycle hooks; the game owns all
//! game-specific state (its structs, its rules) and reaches engine services only
//! through the [`EngineCtx`] it is handed. This is the seam
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
/// # Call ordering (invariant) and fixed-timestep cadence (KE-0201)
///
/// The engine runs a **fixed timestep** ([`FIXED_DT`](crate::FIXED_DT)) decoupled
/// from the display rate. Each display frame it banks the real elapsed time and
/// runs simulation in whole fixed steps, then draws once:
///
/// 1. [`init`](Self::init) is called **exactly once**, before the first
///    [`update`](Self::update).
/// 2. Then, for each display frame, [`update`](Self::update) is called
///    **0..N times** — once per fixed step drained from the accumulator, each
///    with the *constant* `dt` [`FIXED_DT`](crate::FIXED_DT) — immediately
///    followed by [`render`](Self::render) **once**.
///
/// So the full sequence is `init` → `(update × k, render)` per frame, where `k`
/// is 0 or more. `render` still runs exactly once per frame and never before
/// `init`; `init` never runs twice. A game must therefore **not** assume one
/// `update` per `render`: a fast display may render several times between
/// simulation steps (`k == 0`), and a slow frame may run several steps before one
/// render (`k > 1`, capped by [`MAX_STEPS_PER_FRAME`](crate::MAX_STEPS_PER_FRAME)
/// to avoid a spiral of death). The [`headless`](crate::headless) driver (one
/// step per frame, synthetic clock) and the windowed [`run`](crate::run) entry
/// (real clock, variable steps) share one accumulator so this contract holds
/// identically across both, and tests enforce it.
///
/// # Separation of concerns
///
/// - [`update`](Self::update) advances game state (mutate the ECS world, read
///   input). It should not record draws.
/// - [`render`](Self::render) records the frame through the render seam
///   ([`EngineCtx::renderer`](crate::EngineCtx::renderer)). It should not change
///   game state.
///
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

    /// Advance game state by one fixed step. Called **0..N times per frame**
    /// (see the cadence note on [`Game`]), always before that frame's
    /// [`render`](Self::render).
    ///
    /// `dt` is always the constant [`FIXED_DT`](crate::FIXED_DT), not a measured
    /// wall-clock delta — this is what makes simulation deterministic and
    /// framerate-independent. Do not accumulate real time yourself; the engine's
    /// accumulator already does. Read input via
    /// [`EngineCtx::input`](crate::EngineCtx::input) and mutate the ECS world via
    /// [`EngineCtx::world_mut`](crate::EngineCtx::world_mut). Do not record draws
    /// here — that is [`render`](Self::render)'s job.
    fn update(&mut self, ctx: &mut EngineCtx, dt: f32);

    /// Record the frame. Called **exactly once per frame**, immediately after
    /// that frame's fixed [`update`](Self::update) steps.
    ///
    /// Record draws through the render seam
    /// ([`EngineCtx::renderer`](crate::EngineCtx::renderer)) following the
    /// `begin_frame` → `set_pipeline`/`draw_mesh` → `submit` protocol. For smooth
    /// motion between fixed steps, a game may read
    /// [`EngineCtx::alpha`](crate::EngineCtx::alpha) — the interpolation factor in
    /// `0.0..=1.0` — and lerp between the previous and current fixed states; a
    /// Phase 2 game may ignore it. This hook may be a no-op in Phase 1. Do not
    /// mutate game state here.
    fn render(&mut self, ctx: &mut EngineCtx);
}
