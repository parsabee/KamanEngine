// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Engine-generic audio for KamanEngine (KE-0405).
//!
//! `kaman-audio` wraps [`kira`] in the smallest surface a game actually needs:
//! **load a sound from a path**, **play it once**, **play it looping**, **stop the
//! loop**, and **set the master volume**. That is the entire API ([`Audio`]).
//!
//! It is engine-generic in the strict sense: it knows nothing about what any sound
//! *means*. "The background music" and "the impact effect" are a *game's* words
//! for two [`SoundHandle`]s; this crate only knows "play this one once" and "loop
//! this one" (ARCHITECTURE §3). A guard test scans the crate's own sources to keep
//! it that way.
//!
//! # Load once, reference by handle
//!
//! [`Audio::load`] keys sounds by path, so asking for the same file twice hands
//! back the same [`SoundHandle`] and decodes nothing the second time — the KE-0103
//! "load once, reference by handle" rule applied to audio. All decoding happens at
//! load time, so no play call touches the filesystem and nothing on the per-frame
//! path allocates.
//!
//! # It needs no audio device, and it never fails
//!
//! Creating a `kira` mixer fails on a machine with no audio output: a headless CI
//! runner, a remote shell, the demo's `--smoke` oracle. Surfacing that as a
//! `Result` would push an `if audio_available` branch into every game, so instead
//! [`Audio::with_output_device`] degrades to a **silent mode** that still accepts
//! loads and plays and still returns valid handles. [`Audio::silent`] constructs
//! that mode directly — which is what the engine's headless driver holds, so a
//! headless run opens no device at all.
//!
//! Nothing here panics, unwraps, or returns a `Result`. Game code calls
//! `ctx.audio().play_once(handle, volume)` unconditionally, and that call is either
//! audible or a no-op.
//!
//! Because silence is a *behaviour* rather than an absence, it is also
//! **assertable**: [`Audio`] records what it was asked to do — the way
//! `kaman-render-api`'s `NullRenderer` records draws — so a plain `cargo test` on a
//! machine with no output device can prove what a game asked to hear. See
//! [`Audio::loop_starts`] and [`Audio::one_shots_played`].
//!
//! # Example
//!
//! ```rust
//! use kaman_audio::{Audio, Volume};
//!
//! // No device needed — and no branch in the code below because of it.
//! let mut audio = Audio::silent();
//! let bed = audio.load("music.wav");
//! let hit = audio.load("hit.wav");
//!
//! audio.set_master_volume(Volume(-3.0));
//! audio.play_looping(bed, Volume(-14.0));
//! audio.play_once(hit, Volume(-2.0));
//!
//! // Asking again for the sound already looping starts nothing, so a game that
//! // re-triggers its music can never end up playing two copies of it.
//! audio.play_looping(bed, Volume(-14.0));
//! assert_eq!(audio.loop_starts(), 1);
//! assert_eq!(audio.one_shots_played(), 1);
//! ```

#![deny(missing_docs)]

pub mod handle;
pub mod mixer;
pub mod volume;

pub use handle::SoundHandle;
pub use mixer::Audio;
pub use volume::Volume;

#[cfg(test)]
mod guard_tests {
    /// A-boundary guard: `kaman-audio` must stay engine-generic.
    ///
    /// Mirrors the `kaman-ecs` / `kaman-assets` / `kaman-core` guards: it scans
    /// every source file in this crate, token by token, for any game-specific
    /// identifier. The forbidden words are assembled from ASCII byte codes so they
    /// never appear literally in this file (the test cannot false-positive on its
    /// own body). Matching is case-insensitive and whole-word.
    ///
    /// Any new module must be added to `sources` so it is covered.
    #[test]
    fn no_game_specific_symbols() {
        let sources: &[(&str, &str)] = &[
            ("lib.rs", include_str!("lib.rs")),
            ("handle.rs", include_str!("handle.rs")),
            ("mixer.rs", include_str!("mixer.rs")),
            ("volume.rs", include_str!("volume.rs")),
        ];

        // Game concepts, spelled from byte codes so no literal occurrence exists
        // here: [99,97,114], [114,111,97,100], [115,99,111,114,101],
        // [111,98,115,116,97,99,108,101], [108,97,110,101].
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
                         kaman-audio must stay engine-generic",
                    );
                }
            }
        }
    }
}
