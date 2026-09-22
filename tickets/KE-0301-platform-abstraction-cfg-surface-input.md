# KE-0301 — Platform abstraction (`#[cfg]` surface/input)

Phase:         3
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          M · A3
Depends on:    KE-0101      Blocks: —
Serves:        KR3.1
> **Backburner (rough draft).** Phase 3 / iOS is deferred until the engine is solid and the game
> is semi-playable, and it needs full Xcode + rustup iOS targets (`./scripts/preflight.sh --ios`).
> Fleshed out properly when Phase 3 actually starts.

## Problem
`kaman-core` has macOS-only winit/AppKit paths. Split the surface + window + input behind a
`#[cfg(target_os = ...)]` platform layer so iOS can plug in without touching engine logic.

## Scope (rough)
- [ ] A small platform trait/module for window-surface creation + event pump, `#[cfg]`-split macOS/iOS.
- [ ] No AppKit-only calls on the shared path; macOS keeps working exactly as today.
- [ ] iOS stubs compile under the iOS target (even if unimplemented).

## Test gate
macOS oracle still green; `cargo build` for the iOS target compiles the platform stubs.

## Doc gate
`#![deny(missing_docs)]`; note the platform seam in ARCHITECTURE.
