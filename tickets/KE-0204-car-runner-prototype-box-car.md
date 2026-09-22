# KE-0204 — `car-runner` prototype (box car)

Phase:         2
Priority:      P1
Status:        Todo
Integration:   New
Size:          M · A0
Time:          M
Risk:          Med
Depends on:    KE-0201, KE-0202, KE-0203      Blocks: "is it fun?" gate (KR2.4)
Serves:        KR2.4

## Problem / Motivation
Turn the `car-runner` stub into a **playable endless runner** on macOS, driving only the engine's
public API. The car is a box; the track scrolls; obstacles stream in and must be dodged. This is the
**"is it fun?" gate** — the whole point of reaching Phase 2 fast. All game concepts (car, road,
lane, obstacle, score) live **only in `games/car-runner`**, never in engine crates.

## Scope & Acceptance
- [ ] Player = a box entity; **kinematic lane movement** (left/right between lanes) driven by input
      from `EngineCtx`, script/game-owned (not solver-driven), per ARCHITECTURE §5.
- [ ] Forward motion + a scrolling track built from streamed segments (KE-0203 focus = the car).
- [ ] **Obstacles** stream ahead and despawn behind (KE-0203); collision with the car via
      `kaman-physics` ends/《resets》the run.
- [ ] **Score** increases with distance; shown at least via stdout/log (HUD overlay is Phase 4).
- [ ] Runs interactively: `cargo run -p car-runner` opens the Metal window and is playable; the
      `--smoke` headless path still boots the game and exits 0.
- [ ] Fixed-timestep gameplay (KE-0201) so behavior is framerate-independent.
- [ ] The `kaman-*` no-game-symbol guards still pass (car/road/score exist only in `car-runner`).

## Technical notes
- A0 for the *engine* (no engine-crate API change) — all churn is in `games/car-runner`. If you find
  yourself needing an engine change, that's a separate engine ticket, not this one.
- Lane motion is kinematic: set transforms/velocities directly; rapier is used for
  obstacle/collision queries, not to drive the car.
- Keep input minimal (KE-0101 `InputState`: left/right + restart); richer input is KE-0304.
- After it runs, **hold the "is it fun?" review** (KR2.4) before piling on more content.

## Out of scope
- Real car/obstacle meshes + textures (Phase 4, glTF). HUD/SDF text + audio (Phase 4).
- Authoring gameplay in KamanScript (Phase 5 — this logic is Rust for now and ports later).
- iOS/touch controls (Phase 3).

## Test gate
`cargo test --workspace` green (game-logic units where practical: lane clamp, scoring, collision→
reset); `--smoke` exits 0; no-game-symbol guards green; clippy clean. Interactive playability
reviewed at the "is it fun?" gate.

## Doc gate
`#![deny(missing_docs)]` (as applicable to the bin); `car-runner` README documents controls + the
game loop; a short "is it fun?" note recorded for the KR2.4 review.
