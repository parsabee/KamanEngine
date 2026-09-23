// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The shared engine loop both drivers run.
//!
//! The [`headless`](crate::headless) driver and the winit [windowed](crate::run)
//! entry differ only in where they get their input, their render backend, and
//! their clock. The *cadence* — init-once, then fixed-timestep `update`s draining
//! the [`Accumulator`](crate::timestep::Accumulator) followed by one `render` — is
//! identical, so it lives here as [`drive_frame`], operating on the shared
//! engine state in [`Loop`]. This is the single loop implementation the ticket
//! calls for: neither driver reimplements the accumulator.

use kaman_camera::Camera;
use kaman_perf::PerfTracker;
use kaman_scene::Scene;
use std::time::Duration;

use crate::context::{EngineCtx, Renderer};
use crate::game::Game;
use crate::input::InputState;
use crate::timestep::{Accumulator, FIXED_DT};

/// The engine state both drivers own and hand to [`drive_frame`].
///
/// Holds everything platform-neutral: the [`Scene`] (which owns the ECS world and
/// the physics world), the [`InputState`] snapshot, the [`PerfTracker`], the
/// fixed-timestep [`Accumulator`], and the once-only `init` latch. The render
/// backend is deliberately *not* in here — each driver owns its renderer
/// differently (a `NullRenderer` by value in headless, a `Box<dyn Renderer>` in
/// the windowed app) and passes it into [`drive_frame`] by `&mut dyn Renderer`.
pub struct Loop {
    /// The simulation scene (ECS world + physics world + streaming/rebase state)
    /// the game mutates. Replaces the bare `World` the loop used to own (KE-0203).
    pub scene: Scene,
    /// The current input snapshot the game reads.
    pub input: InputState,
    /// The engine-owned [`Camera`] (KE-0205). The game drives it through
    /// [`EngineCtx::camera_mut`](crate::EngineCtx::camera_mut) (e.g. via a chase
    /// controller); each frame the driver pushes its view-projection across the
    /// render seam before [`Game::render`](crate::Game::render), so the game owns
    /// the view while the render backend stays camera-free.
    pub camera: Camera,
    /// Frame-timing tracker.
    pub perf: PerfTracker,
    /// The fixed-timestep accumulator shared with the windowed driver.
    pub accumulator: Accumulator,
    /// Whether [`Game::init`](crate::Game::init) has already run.
    pub initialized: bool,
}

impl Loop {
    /// A fresh loop: empty scene, no input, zeroed accumulator, un-initialized,
    /// and a default [`Camera`] at `DEFAULT_ASPECT`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scene: Scene::new(),
            input: InputState::new(),
            camera: Camera::new(Self::DEFAULT_ASPECT),
            perf: PerfTracker::new(),
            accumulator: Accumulator::new(),
            initialized: false,
        }
    }

    /// Default camera aspect ratio (4:3) until a driver reports a real viewport
    /// size. The windowed driver updates it on resize; the headless driver never
    /// renders pixels, so the value is inconsequential there.
    const DEFAULT_ASPECT: f32 = 4.0 / 3.0;

    /// Run [`Game::init`](crate::Game::init) once, against `renderer`.
    ///
    /// Idempotent: the first call inits and latches; later calls are no-ops. Both
    /// drivers call this before their first frame (the windowed driver on first
    /// `resumed`, headless on the first `run`).
    pub fn init_once<G: Game>(&mut self, game: &mut G, renderer: &mut dyn Renderer) {
        if !self.initialized {
            let mut ctx = EngineCtx::new(
                &mut self.scene,
                renderer,
                &mut self.camera,
                &self.input,
                self.perf.snapshot(),
                0.0,
            );
            game.init(&mut ctx);
            self.initialized = true;
        }
    }
}

impl Default for Loop {
    fn default() -> Self {
        Self::new()
    }
}

/// Advance one display frame: bank `elapsed`, run the fixed `update` steps, then
/// `render` once. Returns the number of `update` steps run this frame.
///
/// This is the whole shared cadence. `elapsed` is the real time since the
/// previous frame — a synthetic delta in headless, a measured monotonic delta in
/// the windowed driver. The accumulator turns it into a whole number of
/// [`FIXED_DT`] steps (clamped against the spiral of death); each step calls
/// [`Game::update`](crate::Game::update)`(ctx, FIXED_DT)` and then advances the
/// [`Scene`]'s physics by one fixed step ([`Scene::step_physics`]), so physics is
/// integrated exactly once per fixed update in lockstep with the simulation.
/// After the update steps, a single [`Game::render`](crate::Game::render) runs with
/// the leftover interpolation [`alpha`](crate::EngineCtx::alpha).
///
/// Ordering within a fixed step is `update` → `step_physics`: the game applies
/// intents (forces, spawns, streaming) first, then the solver integrates and syncs
/// dynamic transforms back into the ECS. A game that also rebases should call
/// [`Scene::maybe_rebase`] from its `update` *after* driving streaming — the
/// documented `stream` → `step_physics` → `maybe_rebase` order lives in
/// `kaman-scene`; since the loop steps physics right after `update`, a game rebases
/// at the top of the *next* `update` (i.e. between this step and the next).
///
/// [`init_once`](Loop::init_once) must have run first.
pub fn drive_frame<G: Game>(
    lp: &mut Loop,
    game: &mut G,
    renderer: &mut dyn Renderer,
    elapsed: Duration,
) -> u32 {
    lp.perf.begin_frame();
    let snapshot = lp.perf.snapshot();

    let steps = lp.accumulator.advance(elapsed);

    // Fixed-timestep updates: 0..N, each with the constant FIXED_DT, each
    // followed by exactly one physics step so the solver stays in lockstep.
    for _ in 0..steps {
        {
            let mut ctx = EngineCtx::new(
                &mut lp.scene,
                renderer,
                &mut lp.camera,
                &lp.input,
                snapshot,
                0.0,
            );
            game.update(&mut ctx, FIXED_DT);
        }
        lp.scene.step_physics();
    }

    // Push the engine camera's view-projection across the render seam BEFORE the
    // game records its frame (KE-0205). The seam value is sticky, so the game's
    // own `begin_frame` inside `render` preserves it and every draw this frame is
    // drawn from the camera the game last positioned (e.g. a chase controller in
    // `update`). The backend below the seam owns no camera — only this matrix.
    renderer.set_view_projection(lp.camera.view_projection_matrix());

    // Exactly one render per frame, carrying the interpolation alpha.
    {
        let alpha = lp.accumulator.alpha();
        let mut ctx = EngineCtx::new(
            &mut lp.scene,
            renderer,
            &mut lp.camera,
            &lp.input,
            snapshot,
            alpha,
        );
        game.render(&mut ctx);
    }

    lp.perf.end_frame();
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timestep::MAX_STEPS_PER_FRAME;
    use kaman_render_api::NullRenderer;

    /// A game that counts how many times each hook fired, so the loop's cadence
    /// can be asserted directly.
    #[derive(Default)]
    struct Counter {
        updates: u32,
        renders: u32,
        /// The interpolation alpha seen on the most recent `render`.
        last_alpha: f32,
    }

    impl Game for Counter {
        fn init(&mut self, _ctx: &mut EngineCtx) {}
        fn update(&mut self, _ctx: &mut EngineCtx, _dt: f32) {
            self.updates += 1;
        }
        fn render(&mut self, ctx: &mut EngineCtx) {
            self.renders += 1;
            self.last_alpha = ctx.alpha();
        }
    }

    /// Drive `frames` frames feeding `per_frame` of **synthetic** elapsed time
    /// each frame (no wall clock), returning the game's hook counts.
    fn drive_synthetic(per_frame: Duration, frames: u32) -> Counter {
        let mut lp = Loop::new();
        let mut renderer = NullRenderer::new();
        let mut game = Counter::default();
        lp.init_once(&mut game, &mut renderer);
        for _ in 0..frames {
            drive_frame(&mut lp, &mut game, &mut renderer, per_frame);
        }
        game
    }

    /// **Framerate independence through the shared loop:** feeding one simulated
    /// second at 60 Hz (60 frames × 1/60 s) and at 120 Hz (120 frames × 1/120 s)
    /// runs the *same* number of fixed `update`s, while `render` runs exactly once
    /// per frame at each rate. Uses a synthetic per-frame delta, never wall time.
    #[test]
    fn same_update_count_at_60hz_and_120hz_through_the_loop() {
        let at_60 = drive_synthetic(Duration::from_secs_f64(1.0 / 60.0), 60);
        let at_120 = drive_synthetic(Duration::from_secs_f64(1.0 / 120.0), 120);

        // Same simulated-update count regardless of display rate (± remainder,
        // which is exactly zero here for one clean second).
        assert_eq!(at_60.updates, at_120.updates);
        assert_eq!(at_60.updates, 60);

        // One render per frame at each rate.
        assert_eq!(at_60.renders, 60);
        assert_eq!(at_120.renders, 120);
    }

    /// The render path sees a valid interpolation alpha in `0.0..1.0`. At 120 Hz
    /// half the frames land mid-step, so a non-zero alpha must appear.
    #[test]
    fn render_receives_interpolation_alpha() {
        // Feed a steady half-step per frame: alpha should oscillate 0.5, 0.0, ...
        let g = drive_synthetic(Duration::from_secs_f64(1.0 / 120.0), 3);
        assert!(g.last_alpha >= 0.0 && g.last_alpha < 1.0);
    }

    /// The spiral-of-death clamp, exercised through the whole loop: a single frame
    /// with a huge synthetic delta runs at most `MAX_STEPS_PER_FRAME` `update`s
    /// (still exactly one `render`), and the surplus is dropped so the next frame
    /// does not owe a second burst.
    #[test]
    fn loop_clamps_catch_up_on_a_long_stall() {
        let mut lp = Loop::new();
        let mut renderer = NullRenderer::new();
        let mut game = Counter::default();
        lp.init_once(&mut game, &mut renderer);

        // One giant frame (10 s) — unclamped this would be 600 updates.
        let steps = drive_frame(&mut lp, &mut game, &mut renderer, Duration::from_secs(10));
        assert_eq!(steps, MAX_STEPS_PER_FRAME);
        assert_eq!(game.updates, MAX_STEPS_PER_FRAME);
        assert_eq!(game.renders, 1);

        // Surplus discarded: an immediate zero-time frame runs no update.
        let steps2 = drive_frame(&mut lp, &mut game, &mut renderer, Duration::ZERO);
        assert_eq!(steps2, 0);
        assert_eq!(game.updates, MAX_STEPS_PER_FRAME);
        assert_eq!(game.renders, 2);
    }

    /// The engine owns the camera and pushes its view-projection across the render
    /// seam each frame **before** `Game::render` (KE-0205). A game that moves the
    /// camera in `update` sees that pose reflected in the recorder before it draws.
    #[test]
    fn driver_pushes_the_engine_camera_view_projection_before_render() {
        use kaman_math::glam::Vec3;

        /// A game that moves the engine camera in `update` and records a one-draw
        /// frame in `render` (whose `begin_frame` must not clear the seam camera).
        #[derive(Default)]
        struct MovesCamera;
        impl Game for MovesCamera {
            fn init(&mut self, _ctx: &mut EngineCtx) {}
            fn update(&mut self, ctx: &mut EngineCtx, _dt: f32) {
                ctx.camera_mut().set_position(Vec3::new(1.0, 2.0, 3.0));
                ctx.camera_mut().set_target(Vec3::ZERO);
            }
            fn render(&mut self, ctx: &mut EngineCtx) {
                ctx.renderer().begin_frame();
                ctx.renderer().submit();
            }
        }

        let mut lp = Loop::new();
        let mut renderer = NullRenderer::new();
        let mut game = MovesCamera;
        lp.init_once(&mut game, &mut renderer);
        drive_frame(
            &mut lp,
            &mut game,
            &mut renderer,
            Duration::from_secs_f64(f64::from(FIXED_DT)),
        );

        // The engine pushed the moved camera's view-projection through the seam,
        // and the game's begin_frame did not clear it (sticky), so the recorder
        // still holds exactly the engine camera's view-projection.
        let expected = lp.camera.view_projection_matrix();
        assert_eq!(renderer.view_projection(), Some(expected));
    }
}
