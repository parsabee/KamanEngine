# KE-0404 — 2D HUD / SDF text overlay

Phase:         4
Priority:      P1
Status:        Todo
Integration:   New
Size:          M · A1
Time:          M
Risk:          Low
Depends on:    KE-0102      Blocks: —
Serves:        KR4.3

## Problem / Motivation
The score is stdout-only. Draw a 2D overlay on top of the 3D scene: an **SDF (signed-distance-field)
text** renderer for crisp glyphs at any scale, so the runner shows score/state on screen. Must be
**safe-area aware** for iPhone notches/rounded corners.

## Scope & Acceptance
- [ ] A 2D overlay pass rendered after the 3D scene (orthographic, no depth), through the seam.
- [ ] SDF font atlas + text draw: position, scale, color; crisp at multiple sizes.
- [ ] Safe-area insets respected (config now; real insets wired on iOS in Phase 3).
- [ ] `car-runner` draws its score (and a "crash / restart" prompt) via the HUD instead of stdout.

## Technical notes
- Keep it engine-generic: a `kaman-render` (or small `kaman-ui`) overlay API taking quads + text;
  the game supplies strings/positions. No game types in the engine.
- SDF atlas can be prebaked (asset) — reuse the KE-0402/0403 texture upload path.

## Out of scope
- Rich text layout/i18n/shaping. Interactive UI widgets. Localization.

## Test gate
`cargo test --workspace` green (glyph-quad generation unit tests; an overlay pixel-hash);
macOS oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; document the overlay/text API + safe-area model.
