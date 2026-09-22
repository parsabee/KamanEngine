# KE-0303 — CAMetalLayer on UIView + CADisplayLink loop

Phase:         3
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A2
Depends on:    KE-0302      Blocks: —
Serves:        KR3.2
> **Backburner (rough draft).** Deferred; needs full Xcode + a device. Flesh out at Phase 3 start.

## Problem
Drive the Metal backend on iOS: attach a `CAMetalLayer` to a `UIView` and run the engine loop
off a `CADisplayLink` (the iOS equivalent of the winit redraw loop), reusing the KE-0201
fixed-timestep driver.

## Scope (rough)
- [ ] `CAMetalLayer`-backed `UIView`; hand its drawable to the `kaman-render` backend factory.
- [ ] `CADisplayLink` callback drives `drive_frame` at the display rate.
- [ ] Frames-in-flight pacing (KE-0105) honored on device.

## Test gate
Renders on a physical device (clear + reference scene); no crash in a short session.

## Doc gate
`#![deny(missing_docs)]`; ARCHITECTURE note on the iOS present/loop path.
