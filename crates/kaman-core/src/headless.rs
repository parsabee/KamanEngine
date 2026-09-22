// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The headless driver — run a [`Game`](crate::Game) with no window and no GPU.
//!
//! This is the engine loop with the platform layer stripped away: it holds a
//! [`Loop`](crate::driver::Loop) (the ECS world, an [`InputState`](crate::InputState),
//! a [`PerfTracker`](kaman_perf::PerfTracker), and the fixed-timestep
//! [`Accumulator`](crate::timestep::Accumulator)) plus a
//! [`NullRenderer`](kaman_render_api::NullRenderer) standing in for the render
//! seam, and drives a game for a fixed number of frames. It requires **no display
//! and no Metal device**, so it is what the `--smoke` oracle and the crate's
//! tests use, and it runs on headless CI runners.
//!
//! # The synthetic clock
//!
//! To stay deterministic, this driver does not read wall time: each frame it
//! advances a **synthetic clock** by exactly one [`FIXED_DT`], so the shared
//! [`drive_frame`](crate::driver::drive_frame) runs exactly one `update` and one
//! `render` per frame. The windowed entry ([`run`](crate::run)) runs the *same*
//! [`drive_frame`] but off a real monotonic clock, so a fast display yields 0..N
//! `update`s per frame. Only the clock source differs; the accumulator logic is
//! shared, which is what keeps simulation identical across drivers and frame
//! rates.

use std::time::Duration;

use kaman_ecs::hecs::World;
use kaman_render_api::NullRenderer;

use crate::driver::{drive_frame, Loop};
use crate::game::Game;
use crate::input::InputState;

pub use crate::timestep::FIXED_DT;

/// Drive `game` headlessly for `frames` frames and return the engine state.
///
/// Calls [`Game::init`](crate::Game::init) once, then advances a synthetic clock
/// by one [`FIXED_DT`] per frame — so each of `frames` frames runs exactly one
/// [`Game::update`](crate::Game::update)`(ctx, FIXED_DT)` followed by one
/// [`Game::render`](crate::Game::render), honoring the call-ordering contract on
/// [`Game`]. The synthetic clock keeps runs fully deterministic.
///
/// Returns the [`Headless`] harness so callers/tests can inspect the resulting
/// ECS [`World`] and the [`NullRenderer`]'s recorded draws. Input stays empty for
/// the whole run (there is no device); tests that need input drive it through a
/// [`Headless`] built with [`Headless::new`].
///
/// This performs no GPU work and needs no display.
///
/// # Example
///
/// ```rust
/// use kaman_core::{Game, EngineCtx, headless};
///
/// struct Empty;
/// impl Game for Empty {
///     fn init(&mut self, _: &mut EngineCtx) {}
///     fn update(&mut self, _: &mut EngineCtx, _dt: f32) {}
///     fn render(&mut self, _: &mut EngineCtx) {}
/// }
///
/// let harness = headless::run(&mut Empty, 120);
/// assert_eq!(harness.frames_run(), 120);
/// ```
pub fn run<G: Game>(game: &mut G, frames: u32) -> Headless {
    let mut harness = Headless::new();
    harness.run(game, frames);
    harness
}

/// A reusable headless harness owning the engine state a [`Game`](crate::Game)
/// runs against.
///
/// Wraps a shared [`Loop`] (ECS [`World`], [`InputState`], `PerfTracker`, and the
/// fixed-timestep [`Accumulator`](crate::timestep::Accumulator)) plus a
/// [`NullRenderer`] (the render seam double). Construct one with
/// [`new`](Self::new), optionally seed input, then call [`run`](Self::run) (or
/// the free [`run`](crate::headless::run) function). After a run, inspect
/// [`world`](Self::world) and [`renderer`](Self::renderer) to assert what the
/// game did.
pub struct Headless {
    lp: Loop,
    renderer: NullRenderer,
    frames_run: u32,
}

impl Headless {
    /// Create an empty harness: empty world, fresh `NullRenderer`, no input.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lp: Loop::new(),
            renderer: NullRenderer::new(),
            frames_run: 0,
        }
    }

    /// Drive `game` for `frames` frames against this harness's state.
    ///
    /// [`Game::init`](crate::Game::init) runs on the first call to `run` only;
    /// subsequent calls continue driving `update`/`render` without re-`init`, so
    /// a caller can step a game in chunks. Each frame advances the synthetic
    /// clock by one [`FIXED_DT`] and runs the shared
    /// [`drive_frame`](crate::driver::drive_frame), so it produces exactly one
    /// `update` and one `render` per frame.
    pub fn run<G: Game>(&mut self, game: &mut G, frames: u32) {
        self.lp.init_once(game, &mut self.renderer);

        let step = Duration::from_secs_f64(f64::from(FIXED_DT));
        for _ in 0..frames {
            let steps = drive_frame(&mut self.lp, game, &mut self.renderer, step);
            debug_assert_eq!(steps, 1, "headless synthetic clock is one FIXED_DT per frame");
            self.frames_run += 1;
        }
    }

    /// Mutable access to the input snapshot, so tests can drive keys/mouse
    /// before a [`run`](Self::run).
    pub fn input_mut(&mut self) -> &mut InputState {
        &mut self.lp.input
    }

    /// The ECS [`World`] after (or between) runs — inspect what the game spawned.
    #[must_use]
    pub fn world(&self) -> &World {
        &self.lp.world
    }

    /// The [`NullRenderer`] — inspect recorded draws / created resources.
    #[must_use]
    pub fn renderer(&self) -> &NullRenderer {
        &self.renderer
    }

    /// Total number of frames driven so far across all [`run`](Self::run) calls.
    #[must_use]
    pub fn frames_run(&self) -> u32 {
        self.frames_run
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::EngineCtx;

    /// A game that records the exact order of hook calls, so the driver's
    /// call-ordering contract can be asserted.
    #[derive(Default)]
    struct OrderRecorder {
        log: Vec<&'static str>,
    }

    impl Game for OrderRecorder {
        fn init(&mut self, _ctx: &mut EngineCtx) {
            self.log.push("init");
        }
        fn update(&mut self, _ctx: &mut EngineCtx, _dt: f32) {
            self.log.push("update");
        }
        fn render(&mut self, _ctx: &mut EngineCtx) {
            self.log.push("render");
        }
    }

    #[test]
    fn init_once_then_update_render_pairs() {
        let mut game = OrderRecorder::default();
        run(&mut game, 3);
        assert_eq!(
            game.log,
            vec![
                "init", "update", "render", "update", "render", "update", "render",
            ]
        );
    }

    #[test]
    fn zero_frames_still_inits() {
        let mut game = OrderRecorder::default();
        let harness = run(&mut game, 0);
        assert_eq!(game.log, vec!["init"]);
        assert_eq!(harness.frames_run(), 0);
    }

    #[test]
    fn init_runs_only_once_across_chunked_runs() {
        let mut game = OrderRecorder::default();
        let mut harness = Headless::new();
        harness.run(&mut game, 1);
        harness.run(&mut game, 1);
        assert_eq!(
            game.log,
            vec!["init", "update", "render", "update", "render"]
        );
        assert_eq!(harness.frames_run(), 2);
    }

    /// A game that spawns an entity in `init` and records one draw per frame,
    /// so we can assert engine state after a headless run.
    struct SpawnAndDraw {
        entity: Option<kaman_ecs::hecs::Entity>,
        mesh: Option<kaman_render_api::MeshHandle>,
        pipeline: Option<kaman_render_api::PipelineHandle>,
    }

    impl Game for SpawnAndDraw {
        fn init(&mut self, ctx: &mut EngineCtx) {
            use kaman_ecs::TransformComponent;
            use kaman_math::glam::Vec3;
            use kaman_render_api::{MeshData, PipelineDescriptor, VertexLayout};

            self.entity = Some(
                ctx.world_mut()
                    .spawn((TransformComponent::from_position(Vec3::ZERO),)),
            );
            let r = ctx.renderer();
            self.mesh = Some(r.create_mesh(&MeshData {
                vertices: &[],
                indices: &[],
                layout: VertexLayout::default(),
            }));
            self.pipeline = Some(r.create_pipeline(&PipelineDescriptor {
                vertex_shader: "vs".into(),
                fragment_shader: "fs".into(),
                vertex_layout: VertexLayout::default(),
            }));
        }

        fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
            use kaman_ecs::TransformComponent;
            let e = self.entity.unwrap();
            if let Ok(mut t) = ctx.world_mut().get::<&mut TransformComponent>(e) {
                t.transform.position.x += dt;
            }
        }

        fn render(&mut self, ctx: &mut EngineCtx) {
            use kaman_math::Transform;
            use kaman_render_api::MaterialParams;
            let (mesh, pipeline) = (self.mesh.unwrap(), self.pipeline.unwrap());
            let r = ctx.renderer();
            r.begin_frame();
            r.set_pipeline(pipeline);
            r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
            r.submit();
        }
    }

    #[test]
    fn game_mutates_world_and_records_draws_through_seam() {
        use kaman_ecs::TransformComponent;

        let mut game = SpawnAndDraw {
            entity: None,
            mesh: None,
            pipeline: None,
        };
        let harness = run(&mut game, 10);

        // Ten frames each advanced x by FIXED_DT.
        let e = game.entity.unwrap();
        let pos = harness
            .world()
            .get::<&TransformComponent>(e)
            .unwrap()
            .transform
            .position;
        assert!((pos.x - 10.0 * FIXED_DT).abs() < 1e-5);

        // Ten frames, each begun/submitted with one draw.
        assert_eq!(harness.renderer().frames_begun(), 10);
        assert_eq!(harness.renderer().frames_submitted(), 10);
        assert_eq!(harness.renderer().draw_count(), 10);
        assert_eq!(harness.renderer().live_mesh_count(), 1);
    }

    #[test]
    fn seeded_input_is_visible_to_the_game() {
        use crate::input::Key;

        struct ReadsInput {
            saw_w: bool,
        }
        impl Game for ReadsInput {
            fn init(&mut self, _: &mut EngineCtx) {}
            fn update(&mut self, ctx: &mut EngineCtx, _dt: f32) {
                if ctx.input().is_key_down(Key::W) {
                    self.saw_w = true;
                }
            }
            fn render(&mut self, _: &mut EngineCtx) {}
        }

        let mut game = ReadsInput { saw_w: false };
        let mut harness = Headless::new();
        harness.input_mut().press_key(Key::W);
        harness.run(&mut game, 1);
        assert!(game.saw_w);
    }
}
