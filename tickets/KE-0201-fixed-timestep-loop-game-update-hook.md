# KE-0201 — Fixed-timestep loop + `Game::update` hook

Phase:         2
Priority:      P0
Status:        Done
Integration:   Refactor
Size:          M · A2
Time:          M
Risk:          Med
Depends on:    KE-0101      Blocks: KE-0203, KE-0204
Serves:        KR2.1

## Problem / Motivation
KE-0101 drives `update` once per rendered frame with a wall-clock `dt`. Gameplay and physics need
a **deterministic fixed timestep** decoupled from render rate, so simulation is identical at 60 and
120 Hz and physics stays stable. Introduce a fixed-timestep accumulator in `kaman-core` that calls
`Game::update(ctx, dt)` a whole number of times per frame with a constant `dt`, and interpolates
render state between steps.

## Scope & Acceptance
- [x] `kaman-core` loop accumulates real elapsed time and runs `update` in fixed increments
      (`FIXED_DT = 1/60`), draining the accumulator; `render` runs once per frame.
- [x] Correct behavior at **60 and 120 Hz** display rates: same number of `update` calls per second
      (± the accumulator remainder), asserted by a test that feeds a synthetic clock.
- [x] **Spiral-of-death guard:** cap the number of catch-up steps per frame (e.g. clamp accumulated
      time) so a stall can't wedge the loop; documented + tested.
- [x] Provide a render **interpolation alpha** (`0..1`) on the render path so drawing can lerp
      between the previous and current fixed states (the game may ignore it in Phase 2).
- [x] Both drivers updated: the headless driver (deterministic, used by `--smoke`/tests) and the
      winit windowed entry. `--smoke` still prints `smoke: 120 frames OK` and exits 0.

## Technical notes
- Keep `FIXED_DT` a single source of truth; physics (`kaman-physics`) steps at this rate so the
  physics golden/behavior is deterministic.
- The headless driver already uses a fixed `dt`; unify it with the accumulator so headless and
  windowed share one loop implementation (only the clock source differs).
- A2: the `Game::update` cadence contract changes (may be called 0..N times per frame) — document it
  and update `car-runner`.

## Out of scope
- Physics removal (KE-0202). World streaming (KE-0203). Gameplay content (KE-0204).

## Test gate
Synthetic-clock tests assert fixed-step counts at 60/120 Hz and the catch-up clamp; `cargo test
--workspace` green; `--smoke` exits 0; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; the update/render cadence + interpolation-alpha contract documented on the
`Game` hooks and in `kaman-core` README; ARCHITECTURE notes the fixed-timestep model.
