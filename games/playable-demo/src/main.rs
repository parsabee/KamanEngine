// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! `playable-demo` — KamanEngine's first title: a playable endless runner, and the
//! host for the headless smoke oracle.
//!
//! This binary is the *only* place game-specific code lives: it implements
//! [`kaman_core::Game`] and is driven by the engine loop through the
//! [`EngineCtx`](kaman_core::EngineCtx) seam. The engine crates never see any of
//! the car / road / lane / obstacle / score concepts modelled here — they are
//! composed entirely from the engine-generic ECS, physics, and scene-streaming
//! primitives. See `docs/PLAYABLE_DEMO.md` at the workspace root for a guided
//! walkthrough of how this is built, and a "build your own game" how-to for
//! writing a new title on top of the same engine crates.
//!
//! # The game
//!
//! The player drives a car forward along the streaming axis (`-Z`), accelerating
//! over the run. Three discrete lanes run along `X`; the arrow keys move the car
//! between them (kinematic — the transform is set directly, the physics solver
//! never drives the car, per ARCHITECTURE §5). The road, traffic, and roadside
//! scenery are produced by [`Scene::stream`](kaman_scene::Scene::stream) with the
//! car as the focus, so the world scrolls endlessly and content behind the player
//! despawns. Traffic appears in deterministic (seeded PRNG) lanes ahead; colliding
//! with it ends the run. Score climbs with distance travelled.
//!
//! # Controls
//!
//! - **Left** — move one lane left.
//! - **Right** — move one lane right.
//! - **Space** — start the run from the title screen, and replay after a crash.
//!   It is ignored mid-run, so a stray press cannot throw away a good score.
//! - **Escape** — quit (handled by the engine's windowed entry).
//!
//! # Module map
//!
//! - [`config`] — every tuning constant, grouped by area (lanes/speed/camera,
//!   car fit, asset paths, buildings, guardrail, terrain, backdrop).
//! - [`rng`] — the seeded lane-obstacle PRNG and the independent per-slot
//!   scenery hash.
//! - [`assets`] — glTF import through `kaman-assets`, fit transforms, and the
//!   vertex-layout helpers.
//! - [`scenery`] — procedural geometry (guardrails, hill terrain) built once and
//!   streamed/positioned by transform.
//! - [`components`] — the game-side ECS components and the model-placement enum.
//! - [`game`] — [`game::CarRunner`], the [`kaman_core::Game`] implementation:
//!   lane input, scoring, streaming, collision.
//! - [`hud`] — the on-screen score and game-over overlay, drawn through the
//!   engine's 2D overlay seam.
//! - [`render`] — the render pass: how a frame is drawn from that state through
//!   the engine's render seam.
//!
//! # The smoke oracle
//!
//! Without a window the binary runs the `--smoke` oracle: it boots the
//! [`Game`](kaman_core::Game) via the engine's headless driver, runs 120 frames
//! offscreen against a
//! `NullRenderer` with **no GPU / Metal device**, and exits 0. Headless input is
//! empty, so the box just runs straight down the middle lane — a deterministic
//! run valid on headless CI (see `docs/INTEGRATION.md` §1).

#![deny(missing_docs)]

use clap::Parser;

mod assets;
mod components;
mod config;
mod game;
mod hud;
mod render;
mod rng;
mod scenery;

use game::CarRunner;

/// Number of frames the smoke oracle simulates before exiting.
const SMOKE_FRAMES: u32 = 120;

/// Command-line arguments for `playable-demo`.
#[derive(Parser, Debug)]
#[command(name = "playable-demo", about = "KamanEngine playable-demo + headless smoke oracle")]
struct Cli {
    /// Run the headless smoke oracle: boot the game and drive a fixed frame loop, then exit 0.
    ///
    /// Requires no GPU/Metal device — safe on headless CI runners.
    #[arg(long)]
    smoke: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.smoke {
        run_smoke(SMOKE_FRAMES);
        return;
    }

    // Windowed path: boots the same `Game` under the winit entry, driving a real
    // Metal backend. The backend is constructed below the render seam by
    // `kaman-render` and injected via a factory, so `kaman-core` never depends on
    // `metal` (ARCHITECTURE §2).
    let mut game = CarRunner::new();
    run_windowed(&mut game);
}

/// Launch the windowed engine with the Metal backend on macOS.
#[cfg(target_os = "macos")]
fn run_windowed(game: &mut CarRunner) {
    use kaman_core::Renderer;
    kaman_core::run_with_backend(
        game,
        Box::new(|window, width, height| {
            Box::new(kaman_render::MetalRenderer::new(window, width, height)) as Box<dyn Renderer>
        }),
    );
}

/// Non-macOS fallback: no Metal backend, run against the null seam.
#[cfg(not(target_os = "macos"))]
fn run_windowed(game: &mut CarRunner) {
    kaman_core::run(game);
}

/// Boot the `playable-demo` [`Game`](kaman_core::Game) and drive `frames` frames
/// headlessly, then report success.
fn run_smoke(frames: u32) {
    use kaman_core::headless::Headless;
    use kaman_core::input::Key;

    let mut game = CarRunner::new();
    let mut harness = Headless::new();

    // The demo opens frozen on its title prompt, so tap the start key on the first
    // frame — otherwise the oracle would replay 120 frames of a motionless title
    // screen and assert nothing about the game. Headless input is otherwise empty,
    // so the car then runs straight down the middle lane: a fully deterministic run.
    harness.input_mut().press_key(Key::Space);
    harness.run(&mut game, 1);
    harness.input_mut().release_key(Key::Space);
    harness.run(&mut game, frames - 1);

    // The engine must have driven the game for exactly the requested frame count.
    assert_eq!(harness.frames_run(), frames, "driver ran the wrong frame count");

    println!("{}", smoke_report(frames));
}

/// The exact stdout contract line the smoke oracle prints on success.
fn smoke_report(frames: u32) -> String {
    format!("smoke: {frames} frames OK")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_runs_without_panicking() {
        // The oracle must complete for the fixed frame count without requiring a GPU.
        run_smoke(SMOKE_FRAMES);
    }

    #[test]
    fn smoke_prints_the_expected_contract_line() {
        // Guards the exact `--smoke` stdout contract (`smoke: 120 frames OK`).
        assert_eq!(smoke_report(SMOKE_FRAMES), "smoke: 120 frames OK");
    }
}
