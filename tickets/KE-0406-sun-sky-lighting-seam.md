# KE-0406 — Drivable sun + sky sun disc (lighting through the seam)

Phase:         4
Priority:      P1
Status:        Done
Integration:   Refactor
Size:          M · A2
Time:          M
Risk:          Med
Depends on:    KE-0401, KE-0102      Blocks: KE-0407
Serves:        KR4.1

## Problem / Motivation
Lighting is currently **unreachable and wrong**. `LightUniforms::default()` is uploaded into a Metal
buffer once at renderer construction and never updated, and `kaman-render-api` has no light type at
all — so nothing above the seam can set the sun. Three concrete consequences:

- The sun sits at **~60° elevation** (`direction: [-0.5, -1.0, -0.3]`) — a noon sun. There is no way
  to ask for any other time of day without editing an engine constant and recompiling.
- **The sun is invisible.** `sky_fragment_main` is a two-colour vertical gradient with no
  relationship to the light direction: no disc, no glow. You only ever see the sun's effect.
- `ambient_intensity: 0.6` against `diffuse_intensity: 0.8` leaves unlit faces at 43% of lit, which
  reads as overcast. Nothing in the scene looks *sunlit*.

Specular compounds it: `viewDir` is hardcoded to `float3(0, 0, 1)` rather than derived from the
camera, so highlights never move as the player drives. A low sun is exactly the angle that makes
that obvious on car bodywork.

## Scope & Acceptance
- [x] An engine-generic sun/sky description in `kaman-render-api` (no game types, no time-of-day
      *policy* — just the physical parameters) settable through the seam and honoured per frame:
      sun elevation + azimuth in degrees, colour, intensity, ambient/sky-fill level, and the sky
      gradient colours. The backend uploads it per frame instead of once at construction.
- [x] Sun direction is **derived** from elevation/azimuth, so a caller never hand-builds a vector.
      Unit-tested: elevation 90° points straight down, 0° is horizontal, and azimuth rotates in a
      documented, asserted direction.
- [x] A **sun disc in the sky** at the direction the light actually comes from, with a soft
      surrounding glow that falls off into the sky gradient. It must stay consistent with the shaded
      geometry — turn the sun and both the disc and the lighting move together.
- [x] Camera world position reaches the fragment shader so **specular is view-dependent**; highlights
      track the camera instead of a constant `(0,0,1)`.
- [x] Ambient rebalanced as sky fill (directional light dominant), so geometry reads as sunlit.
- [x] `playable-demo` asks for a **summer 4pm sun**: ~30–35° elevation from the west, slightly warm,
      with the sky tuned to match. The *choice* lives in the demo's config, not in the engine.
- [x] `LightUniforms` stays byte-compatible with the MSL `Light` struct, with the layout asserted.

## Technical notes
- `LightUniforms` is **exactly 128 bytes** (pinned by `light_uniforms_is_128_bytes`) and this file has
  a history of a Rust↔MSL padding mismatch that silently disabled the whole look stack — see the
  re-bless note on `REFERENCE_HASH`. An MSL `float3` occupies 16 bytes; every `float3` must land
  16-byte aligned. Growing past 128 bytes is allowed, but assert the new size and keep every field
  offset commented as it is today.
- `ground_height` is declared in both structs and **read by no shader** — dead. Either wire it up
  (KE-0407 needs a receiver plane) or delete it; do not leave it dangling.
- Per-frame light upload should reuse the existing uniform-ring discipline rather than reallocating.
- Keep the sky a single full-screen pass; the disc is a direction dot-product, not extra geometry.

## Out of scope
- Shadow maps (KE-0407). Bloom on the sun. Physically-based sky scattering (Hosek/Preetham).
  Time-of-day animation or a day/night cycle — this ticket only makes the sun *settable*.

## Test gate
`cargo test --workspace` green, including new unit tests for elevation/azimuth → direction and the
`LightUniforms` layout assertion. The three pixel-hash baselines are re-blessed **with a written
justification** (this is an intended look change): `REFERENCE_HASH`, `TEXTURED_REFERENCE_HASH`;
`OVERLAY_REFERENCE_HASH` must come back **unchanged** (the overlay is screen-space and must not be
affected — if it moves, something leaked). `--smoke` exits 0; clippy clean; macOS oracle green.

## Doc gate
`#![deny(missing_docs)]`; rustdoc clean. Document the sun/sky API and the elevation/azimuth
convention (which axis is north, which way azimuth turns) — an unstated convention here is a
guaranteed future bug. Note the look change in `kaman-render/README.md` and `docs/ARCHITECTURE.md`.
