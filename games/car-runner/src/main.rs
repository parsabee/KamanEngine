// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! `car-runner` — KamanEngine's first title, and the host for the headless smoke oracle.
//!
//! This binary is the *only* place game-specific code lives: it implements
//! [`kaman_core::Game`] and is driven by the engine loop through the
//! [`EngineCtx`](kaman_core::EngineCtx) seam. The engine crates never see any of
//! the car/road/score concepts modelled here.
//!
//! Phase 1 has no visible renderer yet (that is KE-0102), so the binary's real
//! job today is the `--smoke` oracle: it boots the [`Game`] via the engine's
//! headless driver, runs 120 frames offscreen against a
//! [`NullRenderer`](kaman_core) with **no GPU / Metal device**, and exits 0. That
//! keeps it valid on headless GitHub macOS CI runners (see `docs/INTEGRATION.md`
//! §1). Without `--smoke` it would open a window via [`kaman_core::run`], but the
//! window shows nothing until the Metal backend lands.

use clap::Parser;

use kaman_core::{EngineCtx, Game};
use kaman_ecs::hecs::Entity;
use kaman_ecs::{DynamicTag, RenderComponent, StaticTag, TransformComponent};
use kaman_math::glam::Vec3;

/// Number of frames the smoke oracle simulates before exiting.
const SMOKE_FRAMES: u32 = 120;

/// Command-line arguments for `car-runner`.
#[derive(Parser, Debug)]
#[command(name = "car-runner", about = "KamanEngine car-runner + headless smoke oracle")]
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

    // Windowed path: boots the same `Game` under the winit entry. In Phase 1 the
    // window is present but shows nothing until the Metal backend (KE-0102) lands.
    let mut game = CarRunner::new();
    kaman_core::run(&mut game);
}

/// Boot the `car-runner` [`Game`] and drive `frames` frames headlessly, then report success.
///
/// This is the continuous macOS oracle from `docs/INTEGRATION.md`: it exercises
/// the whole engine/game boundary — `init` once, then `update`/`render` per frame
/// — with no GPU work, so it is valid on headless CI. It prints the fixed
/// `smoke: <n> frames OK` contract line on success.
fn run_smoke(frames: u32) {
    let mut game = CarRunner::new();
    let harness = kaman_core::headless::run(&mut game, frames);

    // The engine must have driven the game for exactly the requested frame count.
    assert_eq!(harness.frames_run(), frames, "driver ran the wrong frame count");

    println!("{}", smoke_report(frames));
}

/// The exact stdout contract line the smoke oracle prints on success.
///
/// Kept as its own function so the `--smoke` output contract can be asserted in a
/// test without spawning the binary (`smoke: <n> frames OK`).
fn smoke_report(frames: u32) -> String {
    format!("smoke: {frames} frames OK")
}

/// The car-runner game state.
///
/// Phase 1 keeps this minimal but real: it spawns a small scene of
/// engine-generic ECS entities in [`init`](Game::init) and advances them in
/// [`update`](Game::update). All meaning (which entity is the vehicle, how the
/// world scrolls) lives here in the game crate, composed from the engine-generic
/// components in `kaman-ecs`.
struct CarRunner {
    /// The player-controlled vehicle entity.
    vehicle: Option<Entity>,
    /// The scrolling ground/track segments.
    track: Vec<Entity>,
    /// Distance travelled so far (the running tally).
    distance: f32,
}

impl CarRunner {
    /// Forward speed of the world, in world-units per second.
    const SPEED: f32 = 12.0;
    /// Number of track segments laid out ahead of the vehicle.
    const TRACK_SEGMENTS: usize = 8;
    /// Spacing between track segments along the travel axis.
    const SEGMENT_SPACING: f32 = 6.0;

    /// Create an unspawned game; [`init`](Game::init) populates the world.
    fn new() -> Self {
        Self {
            vehicle: None,
            track: Vec::new(),
            distance: 0.0,
        }
    }
}

impl Game for CarRunner {
    fn init(&mut self, ctx: &mut EngineCtx) {
        let world = ctx.world_mut();

        // The player vehicle: a dynamic red cube at the origin.
        self.vehicle = Some(world.spawn((
            TransformComponent::from_position(Vec3::new(0.0, 0.5, 0.0)),
            RenderComponent::cube([0.9, 0.1, 0.1]),
            DynamicTag,
        )));

        // A run of static track segments laid out ahead along -Z.
        for i in 0..Self::TRACK_SEGMENTS {
            let z = -(i as f32) * Self::SEGMENT_SPACING;
            let seg = world.spawn((
                TransformComponent::from_position(Vec3::new(0.0, 0.0, z)),
                RenderComponent::cube([0.2, 0.2, 0.25]),
                StaticTag,
            ));
            self.track.push(seg);
        }
    }

    fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
        let step = Self::SPEED * dt;
        self.distance += step;

        let world = ctx.world_mut();

        // Scroll each track segment toward the vehicle; recycle it to the back of
        // the run once it passes behind, giving an endless track from a fixed pool.
        let span = Self::TRACK_SEGMENTS as f32 * Self::SEGMENT_SPACING;
        for &seg in &self.track {
            if let Ok(mut t) = world.get::<&mut TransformComponent>(seg) {
                t.transform.position.z += step;
                if t.transform.position.z > Self::SEGMENT_SPACING {
                    t.transform.position.z -= span;
                }
            }
        }

        // Bob the vehicle slightly so its transform visibly changes each frame.
        if let Some(v) = self.vehicle {
            if let Ok(mut t) = world.get::<&mut TransformComponent>(v) {
                t.transform.position.y = 0.5 + 0.1 * (self.distance * 0.5).sin();
            }
        }
    }

    fn render(&mut self, _ctx: &mut EngineCtx) {
        // Phase 1: no draw recording yet. The scene lives in the ECS world; the
        // renderer that walks it and records draws through the seam is KE-0102.
    }
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

    #[test]
    fn init_spawns_the_expected_scene() {
        let mut game = CarRunner::new();
        let harness = kaman_core::headless::run(&mut game, 0);
        // Vehicle + one entity per track segment.
        let expected = 1 + CarRunner::TRACK_SEGMENTS;
        assert_eq!(harness.world().len() as usize, expected);
        assert!(game.vehicle.is_some());
        assert_eq!(game.track.len(), CarRunner::TRACK_SEGMENTS);
    }

    #[test]
    fn update_advances_distance_and_recycles_track() {
        let mut game = CarRunner::new();
        let harness = kaman_core::headless::run(&mut game, SMOKE_FRAMES);

        assert_eq!(harness.frames_run(), SMOKE_FRAMES);
        // Distance grew by SPEED * dt each frame.
        assert!(game.distance > 0.0);

        // Every track segment stays within the recycling window along Z.
        let span = CarRunner::TRACK_SEGMENTS as f32 * CarRunner::SEGMENT_SPACING;
        for &seg in &game.track {
            let z = harness
                .world()
                .get::<&TransformComponent>(seg)
                .unwrap()
                .transform
                .position
                .z;
            assert!(z <= CarRunner::SEGMENT_SPACING + 1e-3);
            assert!(z >= CarRunner::SEGMENT_SPACING - span - 1e-3);
        }
    }
}
