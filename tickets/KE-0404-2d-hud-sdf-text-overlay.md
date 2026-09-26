# KE-0404 — 2D HUD / SDF text overlay

Phase:         4
Priority:      P1
Status:        Done
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
- [x] A 2D overlay pass rendered after the 3D scene (orthographic, no depth), through the seam.
      → `FrameRecorder::draw_overlay_quad` records engine-generic `OverlayQuad`s; the Metal backend
      flushes them in `flush_overlay` during `submit()`, before `end_encoding()`, through a dedicated
      pipeline with source-over blending and no depth. `overlay_vertex_main` maps pixels straight to
      NDC with the `Y` flip, so the pass is orthographic by construction.
- [x] SDF font atlas + text draw: position, scale, color; crisp at multiple sizes.
      → `FontAtlas::layout(text, origin, px, color, emit)` emits one quad per glyph at any `px`;
      `overlay_fragment_main` mode 2 resolves coverage with `smoothstep` around a `fwidth`-derived
      width, so edges stay sharp at any scale. The demo draws three sizes at once (`HUD_SCORE_PX`
      30, `HUD_BANNER_PX` 30, `HUD_TITLE_PX` 64).
- [x] Safe-area insets respected (config now; real insets wired on iOS in Phase 3).
      → `RenderDevice::safe_area_insets` returns `[top, right, bottom, left]` in pixels; the demo
      derives every HUD position from `surface_size` shrunk by them. macOS reports zeros.
- [x] `car-runner` draws its score (and a "crash / restart" prompt) via the HUD instead of stdout.
      → The on-screen HUD is now the score readout (`SCORE n` top-left) and carries the game-over
      banner with the final score, the best, and the replay prompt. The demo's stdout lines are
      retained only as a headless/CI diagnostic — the `--smoke` oracle has no surface to draw to.

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
