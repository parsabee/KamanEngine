# KE-0002 — Migration characterization harness + golden baselines

Phase:         0
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A3
Time:          M
Risk:          High
Depends on:    KE-0001      Blocks: KE-0003, KE-0005 (and all Phase 1 renderer tickets)
Serves:        KR0.4

## Problem / Motivation
Migration is only safe if we can prove behavior is unchanged. This ticket builds the
regression net described in INTEGRATION §2.1: characterization tests and golden baselines
captured from the prototype's known-good behavior, so every later `move`/`refactor` commit
can be verified.

## Scope & Acceptance
- [ ] Port the prototype's existing tests (`math_test`, `physics_test`, reflectivity/raytracer
      validation) into workspace integration tests; all green.
- [ ] **Physics golden trajectory:** fixed seed + fixed timestep, record a rigid body's
      transform for 300 steps, hash it; a test asserts the hash is stable.
- [ ] **Math known-values:** table-driven tests for the transform/projection helpers.
- [ ] Harness scaffolding for a **render golden pixel-hash** (offscreen render → hash), with
      the actual baseline captured in Phase 1 when `kaman-render` lands. Provide the helper now.
- [ ] Document how to regenerate a golden intentionally (env flag or `xtask bless`) with a
      required justification note in the PR.

## Technical notes
- Golden artifacts live under `tests/golden/`; hashes are committed, large buffers are not.
- Determinism: pin rapier's timestep and disable any wall-clock inputs in golden runs.

## Out of scope
- Capturing the render golden (needs the migrated renderer — Phase 1).

## Test gate
All ported tests green; physics-trajectory and math-known-value goldens green in CI.

## Doc gate
`INTEGRATION §2.1` referenced; `tests/golden/README` explains capture + re-bless workflow.
