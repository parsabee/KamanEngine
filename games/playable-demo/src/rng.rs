// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Deterministic randomness: the lane-obstacle PRNG and the per-slot scenery
//! hash.
//!
//! Two independent sources of "randomness" drive the demo, and they are kept
//! deliberately separate:
//!
//! - [`Rng`] (SplitMix64) is seeded once ([`crate::config::SEED`]) and advanced
//!   only when [`crate::game::CarRunner::spawn_slot`] decides an obstacle lane.
//!   This is the sequence the smoke oracle and the game-logic tests depend on
//!   being reproducible.
//! - [`hash_u64`] and the `*_for_slot` helpers below derive **scenery** choices
//!   (which traffic-car model, which building prefab, how much lateral jitter)
//!   straight from the streaming slot index, *without* touching [`Rng`]. That
//!   keeps cosmetic variety from ever perturbing the obstacle world: the same
//!   seed always produces the same lane sequence and the same obstacle
//!   positions, whether or not a building or traffic-car variant is drawn on
//!   top.
//!
//! Both live in the game crate on purpose: randomness (and how it's spent) is a
//! *game* concern, not an engine one, so it stays out of the `kaman-*` crates.

use crate::config::BUILDING_WEIGHTS;

/// A tiny deterministic PRNG (SplitMix64) so obstacle placement is reproducible
/// from a seed — no wall-clock time is ever read, which the smoke path and the
/// game-logic tests rely on.
#[derive(Debug, Clone)]
pub(crate) struct Rng {
    /// The generator's running state; advanced by [`next_u64`](Self::next_u64).
    state: u64,
}

impl Rng {
    /// Seed the generator. The same seed always yields the same sequence.
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64-bit value (SplitMix64).
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n > 0`).
    pub(crate) fn next_below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n)) as u32
    }
}

/// A SplitMix64 finalizer over `x`, used for all deterministic per-slot choices so
/// they are independent of the lane PRNG ([`Rng`]) (adding variety never perturbs
/// the obstacle world / the smoke run).
pub(crate) fn hash_u64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Pick a traffic car variant (index into
/// [`crate::config::TRAFFIC_CAR_ASSETS`]) for a streaming `slot`. Deterministic
/// and independent of the lane PRNG. Returns `0` if there are no traffic models.
pub(crate) fn variant_for_slot(slot: i64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (hash_u64(slot as u64) % n as u64) as usize
}

/// Pick a roadside building prefab (index into
/// [`crate::config::BUILDING_ASSETS`]) for a streaming `slot` and `side`
/// (0 = left, 1 = right), weighted by [`BUILDING_WEIGHTS`] so skyscrapers are
/// rare and mid/small buildings common. Deterministic and independent of the
/// lane PRNG.
pub(crate) fn building_for_slot(slot: i64, side: u64) -> usize {
    let total: u32 = BUILDING_WEIGHTS.iter().sum();
    if total == 0 {
        return 0;
    }
    let r = (hash_u64((slot as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (side + 1)) % total as u64)
        as u32;
    let mut acc = 0;
    for (i, &w) in BUILDING_WEIGHTS.iter().enumerate() {
        acc += w;
        if r < acc {
            return i;
        }
    }
    BUILDING_WEIGHTS.len() - 1
}
