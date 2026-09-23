# KE-0702 — Discrete lane control + game states + score/replay

Phase:         7
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          M · A2
Time:          M
Risk:          Med
Depends on:    KE-0701, KE-0707      Blocks: KE-0708
Serves:        KR7.1

## Problem / Motivation
The demo must feel like a game: **Left/Right are the only gameplay keys**, each press snaps the car
**one lane** (not the current hold-to-glide), the run tracks a score, and a crash ends the run with a
**game-over** screen reporting the score and a **replay** option. This needs edge-triggered input in
the engine and a small game state machine.

## Scope & Acceptance
- [ ] **Engine: edge-triggered input.** Add `just_pressed`/`just_released` to `kaman-core::InputState`
      (updated once per frame by the drivers). This is the reusable piece KE-0304 also needs; document
      the overlap. (A2: `InputState` public API.)
- [ ] **Discrete lane control:** `Left`/`Right` each move the car exactly one lane per press (clamped
      at the edges); no other keys affect gameplay (a distinct replay key is allowed on game-over).
- [ ] **Game states:** `Playing → GameOver → (replay) → Playing`. On crash, enter `GameOver`; the sim
      pauses/park; show the score + a replay prompt (via the HUD, KE-0707).
- [ ] **Score:** tracked over the run; shown live (HUD) and on the game-over screen; resets on replay.
- [ ] **Replay:** a key (e.g. `Space`/`Enter`) from `GameOver` starts a fresh, deterministic run.
- [ ] `--smoke` still exits 0 (headless input is empty → the car runs straight until it crashes, then
      the smoke harness may auto-replay or just complete its frame budget — keep it deterministic).

## Technical notes
- Lane motion stays kinematic/script-owned (ARCHITECTURE §5); collision remains the game-side AABB.
- Keep the state machine explicit and small — it's exemplary code users will copy.

## Out of scope
- Full input abstraction / touch (KE-0304, Phase 3). The HUD text rendering itself (KE-0404/KE-0707).

## Test gate
`cargo test --workspace` green incl. new tests: `just_pressed` edge semantics; one-lane-per-press;
crash → GameOver → replay transitions; score reset on replay. `--smoke` exits 0; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; document the input edge API and the demo's state machine; note the KE-0304
overlap in ARCHITECTURE.
