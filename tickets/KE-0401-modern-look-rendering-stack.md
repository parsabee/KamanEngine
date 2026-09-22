# KE-0401 — Modern-look rendering stack

Phase:         4
Priority:      P1
Status:        Todo
Integration:   Refactor
Size:          L · A1
Time:          L
Risk:          Med
Depends on:    KE-0102, KE-0205      Blocks: —
Serves:        KR4.1

## Problem / Motivation
The runner currently clears to a flat color and draws flat-shaded boxes. Give it a modern,
mobile-safe look: correct color management plus a small set of high-impact effects, all cheap
enough for a TBDR GPU. This is what turns "recognizable" into "looks like a real game."

## Scope & Acceptance
- [ ] **sRGB + tonemap:** render in linear space, output through an sRGB-correct swapchain format,
      apply a tonemap (e.g. ACES/Reinhard) so colors are correct and not washed out.
- [ ] **Sky + fog:** a gradient sky background and distance fog blending the far plane into the sky
      (hides the streaming spawn edge — pairs with KE-0203).
- [ ] **MSAA:** multisampled color/depth, resolved in-tile; structured so KE-0305 can make it
      memoryless on iOS. `#[cfg]`/config so macOS keeps working.
- [ ] **One shadow:** a single directional shadow (shadow map or a cheap blob) grounding the car.
- [ ] **Bloom:** a light bloom pass on bright pixels.
- [ ] Pixel-hash: the reference scene changes intentionally here — re-bless with written justification.

## Technical notes
- Keep every pass mobile-safe: prefer tile/memoryless attachments, avoid full-res offscreen where a
  half-res blur suffices; budget against KR3.4/KR4 perf targets.
- Effects live in `kaman-render` below the seam; nothing above the seam learns about them.

## Out of scope
- glTF meshes/textures (KE-0402/0403). HUD/text (KE-0404). iOS memoryless specifics (KE-0305).

## Test gate
`cargo test --workspace` green; pixel-hash re-blessed w/ justification; macOS oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; ARCHITECTURE/README note the look stack + which passes are memoryless-ready.
