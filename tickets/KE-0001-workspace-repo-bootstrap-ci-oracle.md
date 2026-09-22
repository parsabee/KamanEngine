# KE-0001 — Workspace + repo bootstrap + CI oracle

Phase:         0
Priority:      P0
Status:        Done
Integration:   New
Size:          L · A3
Time:          M
Risk:          High
Depends on:    —      Blocks: KE-0003, KE-0005, KE-0006
Serves:        KR0.1, KR0.2

## Problem / Motivation
KamanEngine needs to exist as a Cargo **workspace** with the crate skeleton from
ARCHITECTURE §3, and a CI **oracle** that proves the tree builds, tests pass, and the macOS
app still runs on every commit. Everything else in the migration hangs off this. This is the
one A3 that must land first and alone.

## Scope & Acceptance
- [x] Root `Cargo.toml` with `[workspace]` and empty-but-compiling member crates:
      `kaman-math`, `kaman-perf`, `kaman-ecs`, `kaman-physics`, `kaman-camera`, `kaman-scene`,
      `kaman-render-api`, `kaman-render`, `kaman-core`, and `games/car-runner` (stub `bin`).
- [x] Each crate has `#![deny(missing_docs)]` and a one-line crate doc so the lint passes empty.
- [x] Toolchain: `rust-toolchain.toml` pins a rustup channel; document `rustup target add
      aarch64-apple-ios aarch64-apple-ios-sim` as a Phase-3 prerequisite (not installed yet).
- [x] `cargo build --workspace` and `cargo test --workspace` succeed locally.
- [x] **Headless macOS smoke oracle**: a `games/car-runner` (or `xtask`) mode that boots a
      fixed scene, renders 120 frames offscreen, and exits 0. (Renders a clear color only until
      the renderer migrates in Phase 1.)
- [x] CI (GitHub Actions, `macos-latest`) runs: `cargo build --workspace`,
      `cargo test --workspace`, `cargo clippy -- -D warnings`, and the smoke oracle. Red blocks merge.
- [x] `.cargo/config.toml` stub with a place for iOS target triples (Phase 3).

## Technical notes
- Keep crate members minimal; real code arrives via KE-0003/0005/0006 and Phase 1.
- Metal API Validation env (`METAL_DEVICE_WRAPPER_TYPE=1`) enabled in the CI debug run.
- Commit `Cargo.lock`.

## Out of scope
- Any module migration (separate tickets). Any iOS target wiring (Phase 3).

## Test gate
`cargo build/test --workspace` + clippy clean + smoke oracle exits 0, all in CI.

## Doc gate
Root `README` links resolve; each crate compiles under `#![deny(missing_docs)]`;
`ARCHITECTURE §3` layout matches the created crates.
