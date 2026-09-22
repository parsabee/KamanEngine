# KE-0305 — TBDR memoryless depth / MSAA

Phase:         3
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          S · A1
Depends on:    KE-0303      Blocks: —
Serves:        KR3.3, KR3.4

> **Backburner (rough draft).** Deferred; needs a device to validate TBDR/thermals. Flesh out at
> Phase 3 start.

## Problem
Use Apple TBDR features on device: make the depth (and MSAA) attachments **memoryless** (tile
memory only, never written to DRAM), which is the mobile-safe, bandwidth/thermal-friendly setup.

## Scope (rough)
- [ ] Depth attachment `StorageModeMemoryless` on iOS; `#[cfg]`-guarded so macOS is unaffected.
- [ ] Optional MSAA resolved in-tile (memoryless MSAA color/depth) where it helps.
- [ ] No visual regression on macOS (pixel-hash stable).

## Test gate
Device renders correctly; macOS pixel-hash unchanged; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; ARCHITECTURE note on the TBDR memoryless choice.
