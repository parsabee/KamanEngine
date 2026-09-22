// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Application lifecycle and the engine/game boundary for KamanEngine.
//!
//! `kaman-core` owns the engine loop and defines the seam every game plugs into.
//! It sits at the top of the engine stack: it depends on `kaman-ecs`,
//! `kaman-render-api`, `kaman-perf`, `kaman-math`, and `winit`, but **never** on
//! `metal` or the concrete renderer — all rendering goes through the
//! `kaman-render-api` traits (ARCHITECTURE §2). Game concepts (vehicles, tracks,
//! tallies, …) never enter this crate; a test enforces that (see the crate's
//! no-game-symbols guard).
//!
//! # The boundary (ARCHITECTURE §3)
//!
//! The engine owns the loop and calls into the game through two types:
//!
//! - [`Game`] — the trait a game implements. Three lifecycle hooks
//!   ([`init`](Game::init), [`update`](Game::update), [`render`](Game::render))
//!   with a fixed call order; all game-specific code lives behind it.
//! - [`EngineCtx`] — the narrow handle passed into each hook. It exposes only
//!   engine services (the ECS [`World`](kaman_ecs::hecs::World), the render seam,
//!   an [`InputState`], and a [`PerfSnapshot`](kaman_perf::PerfSnapshot)) through
//!   short-lived accessors, so a game can neither reach engine internals nor
//!   alias engine state.
//!
//! # Two drivers, one loop
//!
//! The same game hooks run under two entry points, both sharing the fixed-timestep
//! [`Accumulator`] (only the clock source differs):
//!
//! - [`headless`] — no window, no GPU, against a
//!   [`NullRenderer`](kaman_render_api::NullRenderer), driven by a synthetic
//!   clock. This is what the `--smoke` oracle and tests use; it runs on headless
//!   CI.
//! - [`run`] — a `winit` window on macOS driving the identical hook sequence off
//!   a real monotonic clock, with per-frame drawing behind an isolated function
//!   that KE-0102 fills with the Metal backend.
//!
//! # Fixed timestep (KE-0201)
//!
//! Simulation is decoupled from display rate. Each frame the driver banks the
//! real elapsed time in the [`Accumulator`] and runs
//! [`Game::update`](Game::update)`(ctx, `[`FIXED_DT`]`)` a whole number of times
//! (0..N) — draining the accumulator — then [`Game::render`](Game::render) once.
//! So the `update` count per second of simulated time is framerate-independent
//! (identical at 60 and 120 Hz), a spiral-of-death clamp
//! ([`MAX_STEPS_PER_FRAME`]) keeps a stall from wedging the loop, and
//! [`EngineCtx::alpha`] carries the render interpolation factor. See
//! [`timestep`] for the full contract.
//!
//! # Example
//!
//! ```rust
//! use kaman_core::{Game, EngineCtx, headless};
//!
//! struct Demo;
//! impl Game for Demo {
//!     fn init(&mut self, _: &mut EngineCtx) {}
//!     fn update(&mut self, _: &mut EngineCtx, _dt: f32) {}
//!     fn render(&mut self, _: &mut EngineCtx) {}
//! }
//!
//! // The engine owns the loop; the game just implements `Game`.
//! let harness = headless::run(&mut Demo, 120);
//! assert_eq!(harness.frames_run(), 120);
//! ```

#![deny(missing_docs)]

pub mod app;
pub mod context;
pub mod driver;
pub mod game;
pub mod headless;
pub mod input;
pub mod timestep;

pub use app::{run, run_with_backend, BackendFactory};
pub use context::{EngineCtx, Renderer};
pub use game::Game;
pub use input::{InputState, Key, MouseButton};
pub use timestep::{Accumulator, FIXED_DT, MAX_STEPS_PER_FRAME};

#[cfg(test)]
mod boundary_tests {
    /// Engine/game boundary guard (extends the KE-0005 pattern to `kaman-core`):
    /// no game-named symbol may leak into this crate.
    ///
    /// It embeds every source file of the crate with `include_str!` and scans
    /// each, token by token (splitting on non-alphanumeric boundaries), for any
    /// game-specific identifier. If one appears anywhere the test fails, so a
    /// forbidden concept can't be added without this test seeing it.
    ///
    /// As in `kaman-ecs`, the forbidden words are assembled from ASCII byte codes
    /// rather than written as string literals, so this test's own source contains
    /// no occurrence of them — it can scan every file (including itself) with no
    /// exclusion window and never false-positive on its own body. Matching is
    /// case-insensitive and whole-word, so an incidental substring inside a larger
    /// identifier does not trip the guard; only the exact concept words do.
    #[test]
    fn no_game_specific_symbols() {
        // Every source file in the crate. Any new module must be added here so it
        // is covered by the guard.
        let sources: &[(&str, &str)] = &[
            ("lib.rs", include_str!("lib.rs")),
            ("app.rs", include_str!("app.rs")),
            ("context.rs", include_str!("context.rs")),
            ("driver.rs", include_str!("driver.rs")),
            ("game.rs", include_str!("game.rs")),
            ("headless.rs", include_str!("headless.rs")),
            ("input.rs", include_str!("input.rs")),
            ("timestep.rs", include_str!("timestep.rs")),
        ];

        // Game concepts, built from ASCII byte codes so the words never appear
        // literally in any source file of this crate. Codes spell:
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

        for (name, src) in sources {
            for token in src.split(|c: char| !c.is_ascii_alphanumeric()) {
                if token.is_empty() {
                    continue;
                }
                let lower = token.to_ascii_lowercase();
                for bad in &forbidden {
                    assert_ne!(
                        &lower, bad,
                        "game-specific identifier `{token}` found in {name}: \
                         kaman-core must stay game-agnostic (engine/game boundary)",
                    );
                }
            }
        }
    }
}
