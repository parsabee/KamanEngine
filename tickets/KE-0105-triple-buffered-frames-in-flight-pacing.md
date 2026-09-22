# KE-0105 — Triple-buffered frames-in-flight + pacing

Phase:         1
Priority:      P0
Status:        Done
Integration:   Refactor
Size:          S · A1
Time:          M
Risk:          High
Depends on:    KE-0102, KE-0104      Blocks: —
Serves:        KR1.2, KR1.5

## Problem / Motivation
The prototype has no frames-in-flight control (no semaphore), so the CPU can run ahead of or block
on the GPU unpredictably. Add a bounded **triple-buffered** pipeline: the CPU may prepare up to
N=3 frames ahead, gated by a semaphore signalled on `MTLCommandBuffer` completion, indexing the
per-frame ring/argument storage from KE-0104. This is the standard mobile-safe pacing model.

## Scope & Acceptance
- [ ] Introduce `MAX_FRAMES_IN_FLIGHT = 3` and a dispatch semaphore (or equivalent) that blocks the
      CPU when 3 frames are already queued; signal it from the command-buffer completion handler.
- [ ] Each frame selects its ring slot by `frame_index % MAX_FRAMES_IN_FLIGHT`; the KE-0104 ring is
      sized for 3 slots so an in-flight slot is never overwritten.
- [ ] Present pacing tied to the drawable; no busy-wait spin.
- [ ] The `--smoke` oracle runs 120 frames through the in-flight path and exits 0.
- [ ] Render pixel-hash stable across the change (single-frame offscreen hash is unaffected by
      in-flight count; assert it still matches).

## Technical notes
- The completion handler runs on a Metal-owned thread — signal the semaphore there and keep the
  handler allocation-free. Watch for use-after-free of per-frame resources: a slot's CPU writes for
  frame F+3 must wait on frame F's completion (that's exactly what the semaphore enforces).
- Verify under Metal API Validation and (locally) GPU Frame Capture that no in-flight slot is
  written while the GPU still reads it.

## Out of scope
- The uniform ring itself (KE-0104). Fixed-timestep update/render split (KE-0201). On-device pacing
  via CADisplayLink (Phase 3, KE-0303).

## Test gate
`cargo test -p kaman-render` green; smoke runs 120 frames through the semaphore path, exits 0;
pixel-hash stable; Metal validation clean; clippy green.

## Doc gate
`#![deny(missing_docs)]`; the frames-in-flight model + the "wait on frame F before writing slot
F+N" invariant documented in doc + enforced by the semaphore; README/ARCHITECTURE note the pacing.
