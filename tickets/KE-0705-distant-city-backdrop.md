# KE-0705 — Distant city backdrop

Phase:         7
Priority:      P1
Status:        Todo
Integration:   New
Size:          M · A1
Time:          M
Risk:          Med
Depends on:    KE-0701, KE-0401      Blocks: KE-0708
Serves:        KR7.2

## Problem / Motivation
Give the freeway a sense of place: a **solid city skyline in the distance**, behind the fog and
buildings, so the horizon isn't empty sky.

## Scope & Acceptance
- [ ] A distant city backdrop rendered behind the scene (in front of the gradient sky, behind the
      gameplay): e.g. a large textured billboard/curtain mesh or a panoramic band that follows the
      camera in XZ but stays far away, so it reads as "far skyline."
- [ ] It sits behind the distance fog (KE-0401) so the transition to the sky is smooth, and does not
      z-fight or pop as the world streams/rebases.
- [ ] Committed backdrop asset (skyline texture or silhouette mesh) under the demo's `assets/`.
- [ ] No per-frame allocation added (KR1.2 discipline); backdrop resources created once.

## Technical notes
- Simplest approach: a far billboard locked to the camera's XZ (not Y), drawn early with depth-write
  off (like the sky pass) so gameplay draws over it. Keep it engine-public-API-only if possible; if a
  small render feature is needed, put it below the seam and keep the pixel-hash guarded.
- Coordinate with floating-origin rebase (KE-0203): anchor the backdrop to the camera so a rebase
  doesn't shift it visibly.

## Out of scope
- A fully 3D city you can reach (that's the roadside buildings, KE-0706). Day/night cycle.

## Test gate
`cargo run -p playable-demo` shows a distant skyline behind the fog; `cargo test --workspace` green;
pixel-hash stable (or re-blessed with justification if a below-seam change); clippy clean.

## Doc gate
Document the backdrop technique (camera-locked far billboard) in the demo/architecture docs.
