# KE-0405 — Audio (kira)

Phase:         4
Priority:      P1
Status:        Todo
Integration:   New
Size:          M · A0
Time:          M
Risk:          Low
Depends on:    KE-0101      Blocks: —
Serves:        KR4.3

## Problem / Motivation
The game is silent. Add an audio layer (`kira`) for sound effects (crash, lane change, pickup) and
background music, driven from the game through a small engine-generic API. Audio is largely
platform-agnostic Rust, so it's low-risk and mostly CLT-friendly.

## Scope & Acceptance
- [ ] A `kaman-audio` (or `kaman-core` submodule) wrapping `kira`: load a sound, play one-shot SFX,
      play/stop looping music, master volume. Engine-generic (no game types).
- [ ] Exposed to the game via `EngineCtx` (e.g. `ctx.audio()`); `car-runner` plays a crash SFX on
      collision and looping music.
- [ ] Headless/`--smoke` path stays silent and does not require an audio device (guard/no-op).
- [ ] Sounds loaded once and referenced by handle (asset-cache discipline).

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
