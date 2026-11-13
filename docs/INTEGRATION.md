# Integration & Testing Discipline

How changes land in Stick Shift Engine without breaking anything, fully tested and documented. The
engine was built by migrating an earlier prototype module by module under these rules, and new
work follows the same ones.

## 1. Governing principle: macOS CI is a continuous oracle

Every change must keep Stick Shift Engine building, testing and running on macOS. Breakage is caught the
moment CI goes red, not weeks later. CI runs on every push to `main` and every pull request, and
a red run blocks merging.

## 2. The CI gates

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs, on a macOS runner:

- `cargo fmt --all -- --check`
- `scripts/preflight.sh` (the default toolchain tier)
- `cargo build --workspace` and `cargo test --workspace` (unit, integration and doc tests,
  including the golden pixel-hash tests on the runner's Metal device)
- the render-seam firewall: `stickshift-render-api` must not depend on `metal`
- the de-brand guard: no prototype identifiers in `crates/` or `games/`
- `cargo clippy --workspace --all-targets -- -D warnings`
- rustdoc with broken and private intra-doc links denied
- the headless smoke oracle, `cargo run -p playable-demo -- --smoke` (120 frames on the CPU-only
  `NullRenderer`, with Metal API Validation enabled)

Run the same commands locally before pushing; the [README](../README.md#contributing) lists them.

## 3. Mechanics

### 3.1 Golden baselines

Code that resists unit testing is pinned by goldens captured while its behavior is known-good:

- **Renderer:** deterministic offscreen renders of fixed scenes, with the pixel buffer hashed
  (`crates/stickshift-render/tests/pixel_hash.rs`, `textured_pixel_hash.rs`). A hash only changes with
  a written justification next to the new value, and is re-blessed through the documented
  `BLESS=1` path. The tests skip when no Metal device is present.
- **Physics:** there is no golden trajectory test yet. Determinism is covered by the fixed
  timestep and by the deterministic world-streaming tests in `stickshift-scene`.

### 3.2 Move and change separately

When code moves between crates, the move and any behavior change are **separate commits**, so
`git bisect` can localize a regression. Formatting-only changes are kept out of behavior commits
for the same reason.

### 3.3 Per-change pipeline

| Step | Action | Gate |
|---|---|---|
| 1. Characterize | Write tests for the *current* behavior before changing it | green on the old code |
| 2. Change | Make the change | goldens and characterization tests still pass, or are updated with a justification |
| 3. Unit-test | Fine-grained tests for the public API; every invariant gets a test | tests cover the new behavior |
| 4. Document | rustdoc every public item; module docs state invariants; update the crate README and ARCHITECTURE | `#![deny(missing_docs)]` compiles; doc examples run under `cargo test` |
| 5. Verify | The full CI gate set (section 2) | all green |

### 3.4 Test depth by kind of code

These are targets, not CI-enforced: there is no coverage tooling in CI yet.

- **Moved as-is:** keep the existing tests green; add characterization tests where none exist.
- **Refactored** (renderer, physics, scene, camera, core): put a golden or characterization net in
  place *before* the change; aim for at least 80% line coverage of the touched public API.
- **New** (assets, audio, the C API): test-first, aiming for at least 80% coverage of the logic.

### 3.5 No-breakage mechanisms

- Separate move commits from change commits (bisectable).
- macOS CI green at every merge.
- Golden pixel hashes for the renderer.
- Metal API Validation on for the smoke run; Xcode GPU frame capture reviewed after renderer
  changes.
- **Feature-gate or `#[cfg]` risky parallel work** so the main path never breaks: e.g. iOS code
  behind `#[cfg(target_os = "ios")]`.
- The `stickshift-render-api` seam makes a renderer rewrite unable to leak into other crates.

### 3.6 Documentation standard

- `#![deny(missing_docs)]` on every engine crate: an undocumented public item is a build failure.
- Doc examples compile and run under `cargo test`, so docs can't rot.
- **Invariants live in three places at once:** the doc comment states it, a test enforces it, and
  ARCHITECTURE or a README explains why. (E.g. "a physics handle is removed from its ECS
  component atomically with `remove_body`": doc comment, use-after-free test, architecture note.)
- Every crate has a `README.md`; [ARCHITECTURE.md](ARCHITECTURE.md) records the intentional
  constraints.
- Docs that link to source lines with `#L<n>` anchors are checked with
  `python3 scripts/check-doc-anchors.py`.

### 3.7 Special cases

- **Renderer (GPU, not unit-testable):** golden pixel hashes plus Metal validation plus Instruments;
  each buffer or frames-in-flight change lands and is validated on its own.
- **Physics:** the despawn use-after-free test was written *before* the removal API, and stays.
- **iOS (v0.2):** a physical-device run is required for platform and GPU-performance changes (the
  simulator is not valid for GPU performance); signing is set up before engine changes pile up.
