// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The fixed-timestep accumulator shared by both engine drivers.
//!
//! Both the [`headless`](crate::headless) driver (deterministic, synthetic
//! clock) and the winit [windowed](crate::run) entry (real monotonic clock) run
//! the *same* loop: only the clock source differs. That loop lives here as
//! [`Accumulator`], so there is exactly one implementation of the fixed-timestep
//! cadence to reason about.
//!
//! # The cadence
//!
//! Each frame, the driver reads how much real time has elapsed since the previous
//! frame and hands it to [`Accumulator::advance`]. The accumulator adds it to a
//! running total and returns the whole number of fixed [`FIXED_DT`] steps that
//! should be simulated this frame, draining that time from the accumulator. The
//! driver then:
//!
//! 1. calls [`Game::update`](crate::Game::update)`(ctx, FIXED_DT)` exactly that
//!    many times (0..N), then
//! 2. calls [`Game::render`](crate::Game::render)`(ctx)` **once**, passing the
//!    current [interpolation alpha](Accumulator::alpha).
//!
//! Because the step size is a constant [`FIXED_DT`] independent of frame rate,
//! the number of `update` calls per second of *simulated* time is the same at any
//! display rate (± the sub-step remainder left in the accumulator) — the property
//! the 60/120 Hz test asserts.
//!
//! # Spiral-of-death guard
//!
//! If a frame takes a very long time (a stall, a debugger breakpoint, the process
//! being suspended), a naive accumulator would demand a huge catch-up burst of
//! `update` calls, each of which is itself slow, so the accumulator only grows —
//! the loop wedges. [`Accumulator::advance`] guards against this by capping the
//! catch-up at [`MAX_STEPS_PER_FRAME`] steps per frame and discarding any
//! accumulated time beyond that. Simulated time then falls behind wall-clock time
//! (the game slows down) rather than the loop locking up. The clamp is tested in
//! [`tests::a_huge_dt_is_clamped_to_the_step_cap`].

use std::time::Duration;

/// The fixed simulation timestep: **1/60 second**, the single source of truth.
///
/// [`Game::update`](crate::Game::update) is always called with exactly this `dt`,
/// regardless of display rate. Physics (KE-0202) steps at the same rate so its
/// behavior is deterministic, so this constant must stay the one place the value
/// is defined.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// `FIXED_DT` as a [`Duration`], for accumulating against clock deltas without
/// repeated float↔duration conversions.
const FIXED_DT_DURATION: Duration = Duration::from_nanos((1_000_000_000 / 60) as u64);

/// Spiral-of-death guard: the maximum number of fixed `update` steps run in a
/// single frame.
///
/// If accumulated time would demand more than this, the surplus is dropped (see
/// [`Accumulator::advance`]). At [`FIXED_DT`] this caps catch-up at
/// `MAX_STEPS_PER_FRAME / 60` seconds of simulation per frame, so one long stall
/// cannot cascade into an unbounded burst of slow `update` calls.
pub const MAX_STEPS_PER_FRAME: u32 = 5;

/// A fixed-timestep accumulator: the shared heart of both engine drivers.
///
/// Feed it the real elapsed time each frame with [`advance`](Self::advance); it
/// returns how many [`FIXED_DT`] `update` steps to run and retains the sub-step
/// remainder for the next frame. Read [`alpha`](Self::alpha) on the render path
/// to interpolate between fixed states.
#[derive(Debug, Clone)]
pub struct Accumulator {
    /// Real time banked but not yet consumed by a fixed step.
    remainder: Duration,
}

impl Accumulator {
    /// Create an empty accumulator (no banked time).
    #[must_use]
    pub fn new() -> Self {
        Self {
            remainder: Duration::ZERO,
        }
    }

    /// Bank `elapsed` real time and return how many fixed steps to run now.
    ///
    /// Adds `elapsed` to the running total, then computes the whole number of
    /// [`FIXED_DT`] steps that fit, draining that time. The count is clamped to
    /// [`MAX_STEPS_PER_FRAME`]; if the clamp fires, the surplus accumulated time
    /// is **discarded** (not banked) so the next frame does not immediately owe
    /// another maxed-out burst — this is the spiral-of-death guard.
    ///
    /// After this call, [`alpha`](Self::alpha) reflects the leftover remainder.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.remainder += elapsed;

        let mut steps: u32 = 0;
        while self.remainder >= FIXED_DT_DURATION {
            self.remainder -= FIXED_DT_DURATION;
            steps += 1;
            if steps == MAX_STEPS_PER_FRAME {
                // Spiral-of-death guard: drop the surplus so we don't re-owe it.
                self.remainder = Duration::ZERO;
                break;
            }
        }
        steps
    }

    /// The render interpolation factor in `0.0..=1.0`.
    ///
    /// Equal to `remainder / FIXED_DT`: the fraction of a fixed step that has
    /// accumulated since the last simulated step. Rendering can lerp between the
    /// previous and current fixed states by this amount for smooth motion at
    /// display rates that don't divide evenly into the fixed rate. Because
    /// [`advance`](Self::advance) always drains full steps, the remainder is
    /// strictly less than one step, so this is always `< 1.0` (and `>= 0.0`).
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.remainder.as_secs_f32() / FIXED_DT
    }
}

impl Default for Accumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One exact fixed step of real time yields exactly one update, no leftover.
    #[test]
    fn one_fixed_dt_is_one_step() {
        let mut acc = Accumulator::new();
        assert_eq!(acc.advance(FIXED_DT_DURATION), 1);
        assert!(acc.alpha() < 1e-3);
    }

    /// Half a fixed step yields no update but banks the time as interpolation
    /// alpha; a second half then completes one step.
    #[test]
    fn sub_step_time_is_banked_as_alpha() {
        let mut acc = Accumulator::new();
        let half = FIXED_DT_DURATION / 2;
        assert_eq!(acc.advance(half), 0);
        assert!((acc.alpha() - 0.5).abs() < 1e-2);
        assert_eq!(acc.advance(half), 1);
        assert!(acc.alpha() < 1e-2);
    }

    /// **Framerate independence:** feeding one simulated second at 60 Hz (60
    /// frames of 1/60 s) and at 120 Hz (120 frames of 1/120 s) yields the *same*
    /// total update count (± the sub-step remainder). This drives the accumulator
    /// directly with a synthetic time delta — no wall clock is read.
    #[test]
    fn same_update_count_at_60hz_and_120hz() {
        // 60 Hz: 60 frames, each 1/60 s of real time.
        let mut acc60 = Accumulator::new();
        let mut steps60 = 0u32;
        for _ in 0..60 {
            steps60 += acc60.advance(Duration::from_secs_f64(1.0 / 60.0));
        }

        // 120 Hz: 120 frames, each 1/120 s of real time — same total wall time.
        let mut acc120 = Accumulator::new();
        let mut steps120 = 0u32;
        for _ in 0..120 {
            steps120 += acc120.advance(Duration::from_secs_f64(1.0 / 120.0));
        }

        // Both simulate ~one second of fixed steps: ~60 updates.
        // The remainder banked in the accumulator accounts for any ±1 slop.
        let remainder60 = acc60.alpha();
        let remainder120 = acc120.alpha();
        assert_eq!(
            steps60, steps120,
            "update count must be framerate-independent (60Hz={steps60}, 120Hz={steps120})"
        );
        assert_eq!(steps60, 60, "one simulated second is ~60 fixed steps");
        // Both leftover remainders are sub-step.
        assert!(remainder60 < 1.0 && remainder120 < 1.0);
    }

    /// The spiral-of-death guard: a single huge dt runs at most
    /// [`MAX_STEPS_PER_FRAME`] steps and the surplus is discarded (the next
    /// frame does not owe a second maxed-out burst).
    #[test]
    fn a_huge_dt_is_clamped_to_the_step_cap() {
        let mut acc = Accumulator::new();
        // Ten seconds in one frame would be 600 steps unclamped.
        let steps = acc.advance(Duration::from_secs(10));
        assert_eq!(steps, MAX_STEPS_PER_FRAME, "catch-up is capped");
        // Surplus dropped: an immediate zero-time frame owes nothing.
        assert_eq!(acc.advance(Duration::ZERO), 0);
        assert!(acc.alpha() < 1e-3, "no surplus banked after the clamp");
    }
}
