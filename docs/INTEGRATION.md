# Integration & Testing Strategy

How prototype code becomes KamanEngine code without breaking anything, fully tested and
documented. This document governs step 3 ("Implement") and step 4 ("SQA") of every phase
in [ROADMAP.md](ROADMAP.md).

## 1. Governing principle: the macOS app is a continuous oracle

The prototype builds and runs on macOS today. Every integration step must keep KamanEngine
building and running on macOS. Breakage is caught the moment the oracle goes red — not weeks
later. CI runs the oracle on every commit.

## 2. Mechanics

### 2.1 Safety net (built once, in Phase 0)

- **CI oracle:** every commit runs `cargo build --workspace`, `cargo test --workspace`, and a
  headless macOS smoke run (boot a scene, render 120 frames, exit 0). Red blocks merge.
- **Metal API Validation** on in all debug/CI runs.
- **Golden baselines captured while behavior is known-good:**
  - *Renderer* → deterministic offscreen render of a fixed scene → **hash the pixel buffer**.
    (Captured in Phase 1 when the renderer migrates.)
  - *Physics* → fixed seed + fixed timestep → record a 300-step trajectory → **golden hash**.
  - *Existing tests* pinned green.

### 2.2 Migration order — leaves before roots

```
math → perf → ecs → physics → camera → scene → render-api → render → core/platform → assets → script
```

Pure, zero-coupling modules first: they prove the workspace split at near-zero risk. The
renderer (highest risk) comes only after the seam and golden-hash net exist.

### 2.3 Per-module pipeline (repeat for each crate)

`move` and `change` are **separate commits** so `git bisect` can localize any regression.

| Step | Action | Gate |
|---|---|---|
| 1. Characterize | Write tests for *current* behavior before moving anything | green on old code |
| 2. Extract | Move into `crates/kaman-*`, wire path deps, fix visibility. **No logic change.** | oracle still green |
| 3. Refactor | Apply the ticket's change (only now) | golden/characterization still pass, or updated with justification |
| 4. Unit-test up | Fine-grained tests for the public API; every invariant → a test | coverage target for Int-class |
| 5. Document | rustdoc every public item; module doc states invariants; port "gotchas" into doc + tests; update crate README + ARCHITECTURE | `#![deny(missing_docs)]` compiles; doc examples run under `cargo test` |
| 6. Verify | Full oracle + (iOS tickets) device smoke | all green |
| 7. Merge | behind green CI | — |

### 2.4 Coverage targets by integration class

- **Reuse-as-is** (math, perf, ecs): keep existing tests green; add characterization where none exist; no coverage regression.
- **Refactor** (renderer, physics, scene, camera, core): golden/characterization net *before* the change; raise unit coverage on touched public API to **≥ 80% lines**.
- **New** (assets, script, later spatial queries): **test-first (TDD)**, **≥ 80%** on logic; KamanScript adds an example-script corpus asserted end-to-end.

### 2.5 No-breakage mechanisms

- Separate move-commits from change-commits (bisectable).
- macOS oracle green at every commit (CI-enforced).
- Golden pixel-hash + golden physics-trajectory for code that resists unit testing.
- Metal API Validation always on; GPU Frame Capture reviewed after each renderer sub-step.
- **Feature-gate / `#[cfg]` risky parallel work** so the main path never breaks: raytracer
  behind `feature = "raytracer"`; iOS behind `#[cfg(target_os = "ios")]`.
- The `kaman-render-api` seam makes a renderer rewrite physically unable to leak into other crates.

### 2.6 Documentation standard

- `#![deny(missing_docs)]` on every engine crate — undocumented public item = build failure.
- Doc examples compile and run under `cargo test` (docs can't rot).
- **Invariants live in three places at once:** the doc comment states it, a test enforces it,
  ARCHITECTURE/README explains why. (E.g. "a physics handle is removed from its ECS component
  atomically with `remove_body`" → doc + use-after-free test + arch note.)
- Per-crate `README.md`; repo-root `ARCHITECTURE.md` records intentional constraints.

### 2.7 Special cases

- **Renderer (GPU, not unit-testable):** golden pixel-hash + Metal validation + Instruments per sub-step; each buffer/frames-in-flight change lands and is validated independently.
- **Physics:** golden trajectory guards determinism; the despawn use-after-free test is written *before* the removal API.
- **iOS:** physical-device smoke is part of the Test gate for platform tickets (the simulator is invalid for GPU perf); provisioning/signing resolved before engine changes pile up.

## 3. SQA at the phase boundary

Step 4 of the phase loop runs the **full** gate set, not just the touched module's:
1. `cargo test --workspace` (unit + integration + doc tests).
2. Characterization/golden suites (render hash, physics trajectory).
3. macOS oracle smoke.
4. Device smoke (Phase 3+).
5. Coverage report meets per-class targets.
6. `#![deny(missing_docs)]` + `cargo clippy -D warnings` clean.

A phase's OKR review (step 5) may only be marked pass if the SQA gate is fully green.
