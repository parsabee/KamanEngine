// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`Audio`] — the whole audio surface, and its device-free silent mode.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};

use crate::handle::SoundHandle;
use crate::volume::Volume;

/// One loaded sound's bookkeeping.
struct Sound {
    /// The path it was loaded from — the cache key, kept so
    /// [`Audio::path`] can report it (and so a decode failure can name the file).
    path: PathBuf,
    /// The decoded samples, ready to hand to the mixer. `None` when nothing was
    /// decoded: either the whole [`Audio`] is silent (so decoding would be pure
    /// waste) or the file could not be read, which degrades this one sound to
    /// silence rather than failing the load.
    data: Option<StaticSoundData>,
    /// How many one-shots of this sound have been requested.
    one_shots: u32,
}

/// Where playback actually goes.
enum Output {
    /// A live `kira` mixer holding an open output device.
    Device(Box<Device>),
    /// No device: every play is accepted and produces nothing.
    Silent,
}

/// The live mixer and the one loop channel it owns.
struct Device {
    /// The `kira` mixer. Dropping it stops all output, so it lives as long as the
    /// [`Audio`] that owns it.
    manager: AudioManager<DefaultBackend>,
    /// The playing loop's `kira` handle, kept so it can be stopped or replaced.
    /// Held only in device mode — [`Audio::looping`] is the device-independent
    /// view of the same fact.
    looping: Option<StaticSoundHandle>,
}

/// The engine's audio layer: load sounds, play them once or looping, set the
/// master level.
///
/// # The five operations
///
/// [`load`](Self::load) a file into a [`SoundHandle`] (once per path);
/// [`play_once`](Self::play_once) for a discrete event;
/// [`play_looping`](Self::play_looping) and [`stop_looping`](Self::stop_looping)
/// for the one continuous bed, typically background music; and
/// [`set_master_volume`](Self::set_master_volume) for everything at once. There is
/// deliberately nothing else — no mixer graph, no effects, no spatialisation
/// (KE-0405 out of scope).
///
/// # One loop channel
///
/// There is exactly **one** looping slot. [`play_looping`](Self::play_looping) on
/// the sound that already owns it is a **no-op**, and on a different sound it
/// *replaces* it (stopping the old one). So a caller that re-asks for its loop —
/// after a state change, on a restart, every frame if it likes — can never end up
/// with two copies of it playing over each other. That invariant lives here
/// rather than in each game because "I asked for my music twice" is a mistake
/// every game would otherwise make once.
///
/// # No device required, no panics, no branches at the call site
///
/// Constructing a `kira` mixer fails when the machine has no audio output —
/// headless CI, a remote shell, the `--smoke` oracle.
/// [`with_output_device`](Self::with_output_device) turns that failure into
/// **silent mode** instead of an error, and [`silent`](Self::silent) constructs it
/// directly. In silent mode every operation is accepted and does nothing audible:
/// loads still return valid handles (nothing is decoded), plays are counted and
/// dropped, volume is remembered. Nothing in this crate panics, unwraps, or
/// returns a `Result`, so game code calls `play_once` unconditionally — no `cfg`,
/// no `if audio_available`, no error to swallow.
///
/// # Assertable without a device
///
/// Because silence is a behaviour and not an absence, it has to be *provable*.
/// `Audio` therefore records what it was asked to do, the way
/// `kaman-render-api`'s `NullRenderer` records draws:
/// [`is_silent`](Self::is_silent), [`looping`](Self::looping),
/// [`loop_starts`](Self::loop_starts), [`one_shots_played`](Self::one_shots_played),
/// [`one_shots_of`](Self::one_shots_of), [`sound_count`](Self::sound_count),
/// [`decode_count`](Self::decode_count) and [`master_volume`](Self::master_volume)
/// hold the same values in both modes, so a plain `cargo test` on a machine with
/// no output device can assert exactly what a game asked to hear.
///
/// # Off the hot path
///
/// Decoding happens in [`load`](Self::load) and nowhere else, so no play call
/// touches the filesystem. The play calls do no work proportional to the sound
/// and allocate nothing per frame — they are meant to be driven by game *events*
/// (a state change, a collision), not polled every update.
pub struct Audio {
    output: Output,
    /// Loaded sounds, indexed by [`SoundHandle`]'s wrapped value.
    sounds: Vec<Sound>,
    /// Path → handle, so a second load of the same file is a lookup.
    by_path: HashMap<PathBuf, SoundHandle>,
    /// How many files were actually decoded, for dedup assertions.
    decodes: usize,
    /// Which sound owns the loop channel, if any.
    looping: Option<SoundHandle>,
    /// How many times a loop actually *started* (as opposed to a repeat request
    /// for the sound already looping, which starts nothing).
    loop_starts: u32,
    /// How many one-shots have been requested across all sounds.
    one_shots_played: u32,
    /// The current master level.
    master: Volume,
}

impl Audio {
    /// An audio layer bound to the system's default output device, falling back
    /// to [`silent`](Self::silent) if there isn't one.
    ///
    /// Never fails and never panics: a machine with no output device (or a device
    /// that refuses to open) gets the silent no-op, with one line on stderr
    /// saying so. This is what the windowed engine entry uses.
    #[must_use]
    pub fn with_output_device() -> Self {
        let output = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Output::Device(Box::new(Device {
                manager,
                looping: None,
            })),
            // The whole point of the fallback: no device is a perfectly ordinary
            // state (CI, a remote shell), not an error the game should handle.
            Err(error) => {
                eprintln!("kaman-audio: no audio output device ({error}); running silent");
                Output::Silent
            }
        };
        Self::with_output(output)
    }

    /// An audio layer that opens **no device** and produces no sound.
    ///
    /// Everything still works: loads hand back valid handles, plays are accepted
    /// and recorded, volume is remembered. This is what the engine's headless
    /// driver holds, so tests and the `--smoke` oracle never touch (or wait on)
    /// an audio device, and it is constructible directly — not merely reachable by
    /// a device failure — so a test can exercise the silent path deterministically
    /// on a machine that *does* have speakers.
    #[must_use]
    pub fn silent() -> Self {
        Self::with_output(Output::Silent)
    }

    /// Shared constructor for both modes.
    fn with_output(output: Output) -> Self {
        Self {
            output,
            sounds: Vec::new(),
            by_path: HashMap::new(),
            decodes: 0,
            looping: None,
            loop_starts: 0,
            one_shots_played: 0,
            master: Volume::UNCHANGED,
        }
    }

    /// Whether this layer is in silent mode (no device, nothing audible).
    ///
    /// Game code should not need to ask — every operation works either way — but
    /// tests and diagnostics do.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        matches!(self.output, Output::Silent)
    }

    /// Load the sound at `path` and return its handle.
    ///
    /// Decoding happens **here**, once per path: a second load of the same file
    /// returns the same [`SoundHandle`] without re-reading or re-decoding it, so
    /// N plays of one sound cost one decode (the KE-0103 "load once, reference by
    /// handle" rule).
    ///
    /// Infallible by design. If the file cannot be read or decoded, that one
    /// sound becomes silent (a warning goes to stderr) and a valid handle is still
    /// returned — a missing sound must not take a game down, and callers must not
    /// have to branch. In silent mode nothing is decoded at all, so a headless run
    /// pays neither the I/O nor the memory.
    pub fn load(&mut self, path: impl AsRef<Path>) -> SoundHandle {
        let key = path.as_ref().to_path_buf();
        if let Some(&existing) = self.by_path.get(&key) {
            return existing;
        }

        let data = if self.is_silent() {
            None
        } else {
            match StaticSoundData::from_file(&key) {
                Ok(data) => {
                    self.decodes += 1;
                    Some(data)
                }
                Err(error) => {
                    eprintln!(
                        "kaman-audio: could not load {} ({error}); this sound will be silent",
                        key.display()
                    );
                    None
                }
            }
        };

        let handle = SoundHandle(self.sounds.len() as u32);
        self.sounds.push(Sound {
            path: key.clone(),
            data,
            one_shots: 0,
        });
        self.by_path.insert(key, handle);
        handle
    }

    /// Play `sound` once at `volume`, on top of whatever else is playing.
    ///
    /// The right call for a discrete event. Overlapping one-shots of the same
    /// sound mix rather than cutting each other off, and a one-shot ends by
    /// itself. Unknown handles, undecodable sounds and silent mode are all no-ops
    /// — but the request is still counted
    /// ([`one_shots_played`](Self::one_shots_played)), because what a test needs
    /// to know is what the game *asked* for.
    pub fn play_once(&mut self, sound: SoundHandle, volume: Volume) {
        let Some(index) = self.index_of(sound) else {
            return;
        };
        self.sounds[index].one_shots += 1;
        self.one_shots_played += 1;

        let Output::Device(device) = &mut self.output else {
            return;
        };
        let Some(data) = &self.sounds[index].data else {
            return;
        };
        // `volume` returns a cheap clone (the samples live behind an `Arc`), so a
        // one-shot copies no audio.
        let shot = data.volume(Decibels(volume.0));
        if let Err(error) = device.manager.play(shot) {
            eprintln!("kaman-audio: could not play a sound ({error})");
        }
    }

    /// Start `sound` looping at `volume` on the single loop channel.
    ///
    /// The whole sound is the loop region, so it repeats seamlessly and forever
    /// until [`stop_looping`](Self::stop_looping) replaces or ends it.
    ///
    /// **Idempotent for the sound already looping:** if `sound` currently owns the
    /// loop channel this does nothing at all — it does not restart the sound and
    /// does not layer a second copy. Asking for a *different* sound stops the
    /// current loop and starts the new one. So the count of loops actually started
    /// ([`loop_starts`](Self::loop_starts)) rises only when playback really began,
    /// which is what a test asserting "the music was never doubled" pins.
    pub fn play_looping(&mut self, sound: SoundHandle, volume: Volume) {
        let Some(index) = self.index_of(sound) else {
            return;
        };
        if self.looping == Some(sound) {
            return;
        }

        // One loop channel: whatever was looping gives it up first.
        self.stop_looping();
        self.looping = Some(sound);
        self.loop_starts += 1;

        let Output::Device(device) = &mut self.output else {
            return;
        };
        let Some(data) = &self.sounds[index].data else {
            return;
        };
        // `..` is the whole sound as the loop region, so it repeats end-to-start.
        let looped = data.loop_region(..).volume(Decibels(volume.0));
        match device.manager.play(looped) {
            Ok(playing) => device.looping = Some(playing),
            Err(error) => eprintln!("kaman-audio: could not start a looping sound ({error})"),
        }
    }

    /// Stop whatever owns the loop channel, leaving it free.
    ///
    /// A no-op when nothing is looping. One-shots already playing are untouched.
    pub fn stop_looping(&mut self) {
        self.looping = None;
        if let Output::Device(device) = &mut self.output {
            if let Some(mut playing) = device.looping.take() {
                // The stop command is published to the audio thread by `stop`
                // itself, so dropping the handle right after is safe.
                playing.stop(Tween::default());
            }
        }
    }

    /// Set the level everything is mixed through.
    ///
    /// Applies to sounds already playing as well as later ones, and is remembered
    /// (see [`master_volume`](Self::master_volume)) in silent mode too.
    pub fn set_master_volume(&mut self, volume: Volume) {
        self.master = volume;
        if let Output::Device(device) = &mut self.output {
            let main = device.manager.main_track();
            main.set_volume(Decibels(volume.0), Tween::default());
        }
    }

    /// The current master level, as last set (default
    /// [`Volume::UNCHANGED`]).
    #[must_use]
    pub fn master_volume(&self) -> Volume {
        self.master
    }

    /// Which sound owns the loop channel, or `None` if nothing is looping.
    #[must_use]
    pub fn looping(&self) -> Option<SoundHandle> {
        self.looping
    }

    /// How many times a loop actually **started**.
    ///
    /// Repeat requests for the sound already looping do not count (they start
    /// nothing), so this is the number a test compares against 1 to prove a loop
    /// was never layered on top of itself.
    #[must_use]
    pub fn loop_starts(&self) -> u32 {
        self.loop_starts
    }

    /// Total one-shots requested across all sounds.
    #[must_use]
    pub fn one_shots_played(&self) -> u32 {
        self.one_shots_played
    }

    /// One-shots requested for `sound` specifically (`0` for an unknown handle).
    ///
    /// This is how a caller proves an event-driven sound fired exactly as often
    /// as the event happened — no more, no less.
    #[must_use]
    pub fn one_shots_of(&self, sound: SoundHandle) -> u32 {
        self.index_of(sound)
            .map_or(0, |index| self.sounds[index].one_shots)
    }

    /// Number of distinct sounds loaded (one per distinct path).
    #[must_use]
    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    /// Number of files actually decoded.
    ///
    /// With the load-once cache this equals the number of distinct paths that
    /// decoded successfully, so a second [`load`](Self::load) of a path leaves it
    /// unchanged. Always `0` in silent mode, where nothing is decoded at all.
    #[must_use]
    pub fn decode_count(&self) -> usize {
        self.decodes
    }

    /// The path `sound` was loaded from, or `None` for an unknown handle.
    #[must_use]
    pub fn path(&self, sound: SoundHandle) -> Option<&Path> {
        self.index_of(sound)
            .map(|index| self.sounds[index].path.as_path())
    }

    /// Validate a handle against the sound table.
    ///
    /// Every operation goes through this, which is what makes a stale or foreign
    /// handle a no-op instead of a panic.
    fn index_of(&self, sound: SoundHandle) -> Option<usize> {
        let index = sound.0 as usize;
        (index < self.sounds.len()).then_some(index)
    }
}

impl Default for Audio {
    /// [`Audio::silent`] — the mode that needs nothing from the machine.
    ///
    /// Deliberately *not* the device-opening constructor: `Default` is what gets
    /// reached for implicitly, and implicitly opening an output device (in a test,
    /// in a tool, on a build machine) is exactly the surprise this crate exists to
    /// avoid. Opening a device is always spelled out:
    /// [`with_output_device`](Self::with_output_device).
    fn default() -> Self {
        Self::silent()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that does not exist. Silent mode never opens it (nothing is
    /// decoded), which is what lets these tests run anywhere.
    const A: &str = "/kaman-audio/tests/does-not-exist-a.wav";
    /// A second such path.
    const B: &str = "/kaman-audio/tests/does-not-exist-b.wav";

    #[test]
    fn silent_mode_accepts_everything_and_decodes_nothing() {
        // The load-bearing property: with no device, the full API still works and
        // hands back usable handles, so game code needs no second path.
        let mut audio = Audio::silent();
        assert!(audio.is_silent());
        assert!(Audio::default().is_silent(), "the default never opens a device");

        let sound = audio.load(A);
        assert_eq!(audio.sound_count(), 1);
        assert_eq!(audio.decode_count(), 0, "silent mode decodes nothing at all");
        assert_eq!(audio.path(sound), Some(Path::new(A)));

        audio.play_once(sound, Volume(-3.0));
        audio.play_looping(sound, Volume(-9.0));
        audio.set_master_volume(Volume(-6.0));
        audio.stop_looping();

        // Every request was still recorded, so silence is assertable.
        assert_eq!(audio.one_shots_played(), 1);
        assert_eq!(audio.loop_starts(), 1);
        assert_eq!(audio.master_volume(), Volume(-6.0));
        assert_eq!(audio.looping(), None);
    }

    #[test]
    fn loading_the_same_path_twice_reuses_the_handle() {
        // Asset-cache discipline: one path, one handle, one decode.
        let mut audio = Audio::silent();
        let first = audio.load(A);
        let second = audio.load(A);
        assert_eq!(first, second, "the same path yields the same handle");
        assert_eq!(audio.sound_count(), 1, "and no second entry");

        // A different path is a different sound.
        let other = audio.load(B);
        assert_ne!(first, other);
        assert_eq!(audio.sound_count(), 2);
    }

    #[test]
    fn one_shots_are_counted_per_sound_and_in_total() {
        let mut audio = Audio::silent();
        let a = audio.load(A);
        let b = audio.load(B);

        audio.play_once(a, Volume::UNCHANGED);
        audio.play_once(a, Volume::UNCHANGED);
        audio.play_once(b, Volume::UNCHANGED);

        assert_eq!(audio.one_shots_of(a), 2);
        assert_eq!(audio.one_shots_of(b), 1);
        assert_eq!(audio.one_shots_played(), 3);
    }

    #[test]
    fn asking_again_for_the_sound_already_looping_starts_nothing() {
        // The engine-level guarantee against a doubled loop: a repeat request is
        // not a restart and not a second copy.
        let mut audio = Audio::silent();
        let a = audio.load(A);

        audio.play_looping(a, Volume(-10.0));
        assert_eq!(audio.loop_starts(), 1);
        assert_eq!(audio.looping(), Some(a));

        for _ in 0..100 {
            audio.play_looping(a, Volume(-10.0));
        }
        assert_eq!(audio.loop_starts(), 1, "a hundred repeat requests started nothing");
        assert_eq!(audio.looping(), Some(a));
    }

    #[test]
    fn a_different_sound_takes_over_the_one_loop_channel() {
        let mut audio = Audio::silent();
        let a = audio.load(A);
        let b = audio.load(B);

        audio.play_looping(a, Volume::UNCHANGED);
        audio.play_looping(b, Volume::UNCHANGED);
        assert_eq!(audio.looping(), Some(b), "the loop channel holds one sound");
        assert_eq!(audio.loop_starts(), 2, "the replacement really started");
    }

    #[test]
    fn stopping_frees_the_channel_so_a_later_play_restarts() {
        let mut audio = Audio::silent();
        let a = audio.load(A);

        audio.play_looping(a, Volume::UNCHANGED);
        audio.stop_looping();
        assert_eq!(audio.looping(), None);

        audio.play_looping(a, Volume::UNCHANGED);
        assert_eq!(audio.looping(), Some(a));
        assert_eq!(audio.loop_starts(), 2, "a stopped loop can be started again");

        // Stopping when nothing loops is harmless.
        audio.stop_looping();
        audio.stop_looping();
        assert_eq!(audio.looping(), None);
    }

    #[test]
    fn an_unknown_handle_is_ignored_rather_than_fatal() {
        // Handles from another `Audio` (or a stale one) must not take a game down.
        let mut audio = Audio::silent();
        let alien = SoundHandle(7);

        audio.play_once(alien, Volume::UNCHANGED);
        audio.play_looping(alien, Volume::UNCHANGED);

        assert_eq!(audio.one_shots_played(), 0);
        assert_eq!(audio.one_shots_of(alien), 0);
        assert_eq!(audio.loop_starts(), 0);
        assert_eq!(audio.looping(), None);
        assert_eq!(audio.path(alien), None);
    }
}
