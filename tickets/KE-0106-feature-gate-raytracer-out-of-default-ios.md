# KE-0106 — Feature-gate raytracer out of default/iOS build

Phase:         1
Priority:      P0
Status:        Done
Integration:   Refactor
Size:          XS · A0
Time:          S
Risk:          Low
Depends on:    KE-0102      Blocks: —
Serves:        KR1.4

## Problem / Motivation
The prototype always compiles the raytracer (`pub mod raytracer` in `lib.rs`; raytracing pipeline
and per-frame RT buffers in `renderer.rs`). It is not part of the shipping mobile runner and pulls
in per-frame allocations and a second shader library. Gate it behind an off-by-default
`raytracer` feature and exclude it from iOS, so the default/mobile path is lean (INTEGRATION §2.5).

## Scope & Acceptance
- [ ] Put the raytracer path (module, pipeline setup, RT buffers, `raytracing.metal` load) behind
      `#[cfg(feature = "raytracer")]` in `kaman-render` (and re-export gating in `kaman-core` as
      needed). Feature is **off by default**.
- [ ] Compile-guard it off on iOS regardless of feature (`#[cfg(all(feature = "raytracer",
      not(target_os = "ios")))]`), so an accidental feature enable can't pull it into an iOS build.
- [ ] Default build has zero raytracer symbols and does not compile `raytracing.metal`.
- [ ] Both states build and are clippy-clean: `--no-default-features`-equivalent default, and
      `--features raytracer` on macOS.

## Technical notes
- This lands right after KE-0102 so the raytracer is gated before KE-0103/0104 optimize the
  rasterization path — the RT per-frame buffers are then simply out of the default hot path.
- Keep the CPU `trace_ray` reference (if migrated) under the same feature so it doesn't rot.

## Out of scope
- Deleting the raytracer. Any rasterization-path buffer work (KE-0103/0104).

## Test gate
Default `cargo build/test --workspace` green with no raytracer compiled; `cargo build -p
kaman-render --features raytracer` (macOS) green; clippy clean in both; a test/CI check asserts no
raytracer symbol in the default build.

## Doc gate
`#![deny(missing_docs)]` holds in both feature states; README documents the `raytracer` feature and
the iOS exclusion rule; ARCHITECTURE notes raytracer is non-shipping / desktop-only.
