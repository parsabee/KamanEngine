# KE-0704 — Asphalt road texture

Phase:         7
Priority:      P1
Status:        Todo
Integration:   New
Size:          S · A0
Time:          S
Risk:          Low
Depends on:    KE-0701, KE-0403      Blocks: KE-0708
Serves:        KR7.2

## Problem / Motivation
The road is a flat black box. Give it a real **asphalt texture** (with lane markings if easy) using
the texture pipeline (KE-0403), so the freeway reads as a road.

## Scope & Acceptance
- [ ] Commit an asphalt base-color texture (tileable) under `games/playable-demo/assets/`.
- [ ] Apply it to the road tiles: UV the road geometry and draw on the textured pipeline (KE-0403),
      tiling along the road so it scrolls seamlessly with streaming.
- [ ] Optional: painted lane lines separating the 3 lanes (baked into the texture or a second decal).
- [ ] Mipmaps + trilinear (KE-0403) so the receding road doesn't shimmer.

## Technical notes
- Road tiles are streamed and scaled; pick a UV scale that tiles cleanly across the tile length so
  seams between tiles are invisible.
- Keep it engine-public-API-only in the demo (create_texture/bind_texture via the seam).

## Out of scope
- Normal/roughness maps for the road (later polish). Car textures.

## Test gate
`cargo run -p playable-demo` shows a textured asphalt road; `cargo test --workspace` green; `--smoke`
exits 0; clippy clean.

## Doc gate
Note the road texture + UV/tiling approach in the demo docs.
