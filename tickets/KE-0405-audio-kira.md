# KE-0405 — Audio (kira)

Phase:         4
Priority:      P1
Status:        Done
Integration:   New
Size:          M · A2
Time:          M
Risk:          Med
Depends on:    KE-0101      Blocks: —
Serves:        KR4.3

## Problem / Motivation
The game is silent. Add an audio layer (`kira`) for sound effects (crash, lane change, pickup) and
background music, driven from the game through a small engine-generic API. Audio is largely
platform-agnostic Rust, so it's low-risk and mostly CLT-friendly.

## Scope & Acceptance
- [x] A `kaman-audio` (or `kaman-core` submodule) wrapping `kira`: load a sound, play one-shot SFX,
      play/stop looping music, master volume. Engine-generic (no game types).
- [x] Exposed to the game via `EngineCtx` (e.g. `ctx.audio()`); `car-runner` plays a crash SFX on
      collision and looping music.
- [x] Headless/`--smoke` path stays silent and does not require an audio device (guard/no-op).
- [x] Sounds loaded once and referenced by handle (asset-cache discipline).

## Outcome
`crates/kaman-audio` (a crate, not a `kaman-core` submodule — it is a leaf that owns the
`kira`/`cpal` dependency, matching how every other engine capability is a crate). Its whole API is
`Audio::{load, play_once, play_looping, stop_looping, set_master_volume}` over an opaque
`SoundHandle`, with levels in `Volume` (decibels). Loads are keyed by path, so a repeat load is a
lookup and decoding happens once, at load time, never on the update path.

Nothing in it can fail: `AudioManager::new` failing (no output device) degrades the whole layer to a
**silent mode** that still accepts loads and plays and still returns valid handles, and that mode is
also constructible directly (`Audio::silent`). So game code has no `cfg`, no availability check and
no `Result` to handle. Silence is *assertable* the way `NullRenderer`'s draws are —
`is_silent / looping / loop_starts / one_shots_played / one_shots_of / decode_count` — so audio
behaviour is unit-tested with no device at all.

`Loop` owns one `Audio` and exposes it as `ctx.audio()`. `Loop::new` leaves it **silent**, so
`Headless` (and `--smoke`) open no device; the windowed entry alone binds it to the default output.
There is exactly **one loop channel**, and re-asking for the sound already looping starts nothing —
so a game cannot layer two copies of its music over each other.

`playable-demo` loads the two committed WAVs in `init`, starts the music on the
`Ready → Playing` edge (not at launch) and keeps that one loop running across crash and replay, and
plays the impact one-shot from `game_over` — the run's single live→ended edge. Mix levels are three
named constants in the demo's `config`.

## Technical notes
- Keep audio off the fixed-update hot path; trigger from game events. Don't block the loop on I/O
  (load at init / async).
- iOS audio session specifics are handled when Phase 3 lands; keep the API platform-agnostic.

## Out of scope
- Spatial/3D audio, DSP graphs, dynamic music systems. iOS audio-session config (Phase 3).

## Test gate
`cargo test --workspace` green (handle/registry + volume logic unit-testable; audio device mocked or
guarded); `--smoke` exits 0 silently; macOS oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; document the audio API + the headless no-op behavior.
