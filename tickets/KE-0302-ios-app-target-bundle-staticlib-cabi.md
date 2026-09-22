# KE-0302 — iOS app target + bundle + staticlib C-ABI

Phase:         3
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A2
Depends on:    KE-0301      Serves: KR3.2

> **Backburner (rough draft).** Deferred until the engine/game is mature; needs full Xcode +
> `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`. Flesh out at Phase 3 start.

## Problem
Ship the engine as an iOS app: build the Rust engine as a **staticlib** exposing a **C-ABI**
entry point, and wrap it in a minimal iOS app bundle (Xcode project under `platform/ios/`).

## Scope (rough)
- [ ] `staticlib` crate/target exposing `extern "C"` init/tick/teardown entry points.
- [ ] Minimal iOS app target/bundle (Info.plist, launch) that links the staticlib.
- [ ] Builds for device + simulator triples.

## Test gate
iOS target links + produces a bundle; device smoke (boots without crashing) once signing lands.

## Doc gate
`#![deny(missing_docs)]` on the C-ABI surface; README notes the iOS build steps.
