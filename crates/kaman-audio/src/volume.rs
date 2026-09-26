// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`Volume`] — the one unit the audio layer measures loudness in.

/// A volume level in **decibels**, relative to the level a sound was recorded at.
///
/// `0 dB` ([`UNCHANGED`](Self::UNCHANGED)) plays a sound exactly as loud as the
/// file is; negative values make it quieter, positive values louder (and risk
/// clipping the mix). At or below [`SILENT`](Self::SILENT) a sound is inaudible.
///
/// # Why decibels and not a `0.0..=1.0` gain
///
/// Loudness is perceived logarithmically, so a linear gain is the wrong knob to
/// hand a designer: `0.5` does *not* sound half as loud (that is about `-6 dB`,
/// i.e. an amplitude of `0.5`, but two sounds `6 dB` apart read as one clearly
/// under the other rather than "half"). Mixing decisions — "the music sits under
/// the effects" — are naturally expressed as a difference in dB and stay correct
/// when the master level moves, which is exactly how a game's config wants to
/// spell them. [`as_amplitude`](Self::as_amplitude) is the escape hatch for code
/// that needs the linear multiplier.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Volume(pub f32);

impl Volume {
    /// `0 dB` — play at exactly the level the sound was recorded at.
    pub const UNCHANGED: Self = Self(0.0);

    /// `-60 dB` — the level at (and below) which a sound is treated as inaudible.
    ///
    /// Useful as a "mute" that keeps the rest of the mix wiring intact.
    pub const SILENT: Self = Self(-60.0);

    /// The linear amplitude multiplier this level corresponds to: `1.0` at
    /// [`UNCHANGED`](Self::UNCHANGED), `0.0` at or below
    /// [`SILENT`](Self::SILENT), and `10^(dB/20)` in between.
    ///
    /// A pure function, so the volume policy of a game is testable with no audio
    /// device involved at all.
    #[must_use]
    pub fn as_amplitude(self) -> f32 {
        if self <= Self::SILENT {
            return 0.0;
        }
        10.0f32.powf(self.0 / 20.0)
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self::UNCHANGED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_decibels_is_unity_gain() {
        assert!((Volume::UNCHANGED.as_amplitude() - 1.0).abs() < 1e-6);
        assert_eq!(Volume::default(), Volume::UNCHANGED);
    }

    #[test]
    fn minus_six_decibels_is_about_half_the_amplitude() {
        // The rule of thumb the doc comment leans on, pinned.
        assert!((Volume(-6.0).as_amplitude() - 0.5).abs() < 0.01);
        assert!((Volume(-12.0).as_amplitude() - 0.25).abs() < 0.01);
    }

    #[test]
    fn silence_and_below_is_exactly_zero() {
        assert_eq!(Volume::SILENT.as_amplitude(), 0.0);
        assert_eq!(Volume(-120.0).as_amplitude(), 0.0);
    }

    #[test]
    fn quieter_levels_compare_as_quieter() {
        // `PartialOrd` must order by loudness, since the mix policy is expressed
        // as "this sits under that".
        assert!(Volume(-14.0) < Volume(-2.0));
        assert!(Volume(-2.0) < Volume::UNCHANGED);
    }
}
