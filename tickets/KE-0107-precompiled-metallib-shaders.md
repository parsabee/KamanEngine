# KE-0107 — Precompiled `.metallib` shaders

Phase:         1
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          S · A1
Time:          M
Risk:          Med
Depends on:    KE-0102      Blocks: —
Serves:        KR1.4

## Problem / Motivation
The prototype compiles MSL at runtime (`new_library_with_source(include_str!("../shaders/
rasterization.metal"))`). Runtime compilation costs startup time, can fail on device, and is not
the shipping pattern. Precompile shaders to `.metallib` at build time and load the compiled
library at runtime (`new_library_with_data` / default library), with **no runtime source compile**.

## Scope & Acceptance
- [ ] Build step (`build.rs` or `xtask`) compiles `shaders/*.metal` → `.metallib` via `xcrun
      metal`/`metallib`, keyed to the build. Output path wired to the crate (env/`OUT_DIR`).
- [ ] `kaman-render` loads the precompiled `.metallib` at runtime; remove the
      `new_library_with_source` runtime-compile path from the default build.
- [ ] `.metallib` artifacts are **git-ignored** (already in `.gitignore`) and produced by the build;
      the build fails clearly if `xcrun metal` is unavailable.
- [ ] CI compiles the `.metallib` as part of the build and the oracle loads it (no runtime compile).
- [ ] Render pixel-hash from KE-0102 unchanged (same shaders, compiled ahead of time).

## Technical notes
- Keep `.metal` source under `shaders/` (ARCHITECTURE §3); MSL layout must stay in sync with the
  Rust-side uniform/vertex structs (KE-0104) — cross-reference in both files.
- On iOS the toolchain target differs; structure the build step so the Phase-3 iOS target can
  select the right SDK without rework (don't solve iOS here, just don't wall it off).
- If `raytracer` is enabled (KE-0106), its `.metallib` builds under the same feature gate.

## Out of scope
- iOS-specific shader packaging (Phase 3). Shader hot-reload (never for MSL; N/A).

## Test gate
`cargo build --workspace` produces and loads `.metallib` (no runtime `new_library_with_source` in
the default path); oracle green loading the compiled library; pixel-hash stable; clippy green.

## Doc gate
`#![deny(missing_docs)]`; README documents the shader build step and the "no runtime compile" rule;
the MSL↔Rust layout sync contract cross-referenced; ARCHITECTURE notes precompiled shaders.
