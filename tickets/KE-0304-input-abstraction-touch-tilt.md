# KE-0304 — Input abstraction + touch/tilt

Phase:         3
Priority:      P1
Status:        Todo
Integration:   Refactor
Size:          S · A2
Depends on:    KE-0301, KE-0303      Blocks: —
Serves:        KR3.3
> **Backburner (rough draft).** Deferred; needs a device for touch/tilt. Flesh out at Phase 3 start.

## Problem
Generalize `kaman-core::InputState` into a shared `InputEvent` path so keyboard (macOS) and
touch/tilt (iOS) both feed the same game input. Add edge-triggered (just-pressed) queries — also
the fix the car-runner wants for crisp lane changes.

## Scope (rough)
- [ ] Backend-agnostic `InputEvent` feeding `InputState`; macOS keyboard + iOS touch/tilt both map in.
- [ ] Edge-triggered `just_pressed`/`just_released`.
- [ ] car-runner lane input uses edge-triggered presses.

## Test gate
Input maps deterministically (unit-testable); macOS oracle green; touch works on device.

## Doc gate
`#![deny(missing_docs)]`; document the input path in the crate README.
