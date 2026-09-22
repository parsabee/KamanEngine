# KE-0004 — Migrate perf → `kaman-perf`

Phase:         0
Priority:      P1
Status:        Done
Integration:   Reuse-as-is
Size:          S · A0
Time:          S
Risk:          Low
Depends on:    KE-0001      Blocks: —
Serves:        KR0.3

## Problem / Motivation
The prototype has ~670 LOC of profiling/frame-timing already wired into the loop. It is a
low-coupling module worth keeping; migrate it cleanly so Phase 1/3 have profiling from day one.

## Scope & Acceptance
- [x] **Move commit:** relocate `perf.rs` into `crates/kaman-perf` unchanged; fix visibility.
- [x] Feature-gate any stdout/debug-print behavior behind a `perf-hud` feature (off by default
      in the oracle so smoke output stays clean).
- [x] Public API: frame timer start/stop, rolling averages, a snapshot struct.
- [x] Unit tests for the timing/rolling-average math (use a mock clock, not wall time).

## Technical notes
- GPU timing via `MTLCommandBuffer` completion handlers is added later (Phase 3, KE-0602); this
  ticket keeps CPU-side timing only.

## Out of scope
- On-device GPU capture / thermal / memory (KE-0602).

## Test gate
`cargo test -p kaman-perf` green (deterministic via mock clock); oracle green.

## Doc gate
`#![deny(missing_docs)]`; crate `README`; document the `perf-hud` feature flag.
