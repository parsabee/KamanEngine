# KE-0407 — Real shadows: fitted shadow-map pass

Phase:         4
Priority:      P1
Status:        Todo
Integration:   New
Size:          L · A2
Time:          L
Risk:          High
Depends on:    KE-0406      Blocks: —
Serves:        KR4.1

## Problem / Motivation
The renderer has **no shadows**. What exists is a single fake blob — `ground_shadow` darkens
fragments within `shadow_radius` of `shadow_center` — and in the demo it is *dead code you have never
seen*:

- `shadow_center` is permanently `[0, 0, 0]` because nothing ever sets it. The car drives to
  z ≈ -400, so the blob is a stationary dark patch at the world origin that the player leaves behind
  within the first second of a run.
- It ignores `Y` entirely, so it is a vertical **column** of darkening, not a ground blob.
- Nothing occludes anything: buildings, traffic, and guardrails cast nothing, and there is no
  self-shadowing. With a low sun (KE-0406) the absence is the single most obvious thing in the frame.

Replace it with a real depth-only shadow pass. One tight map is the right call here, not cascades:
horizon fog is fully opaque by ~35 world units, so the shadow-relevant slab is small and a single
fitted map spends its whole resolution where the player can actually see.

## Scope & Acceptance
- [ ] A **depth-only render pass** from the sun's point of view into an offscreen depth texture,
      recorded before the scene pass each frame. Everything that draws into the scene casts into it.
- [ ] The light-space frustum is **fitted to the visible slab** each frame (not the whole world), so
      resolution follows the camera. Fit must be stable: document and test what keeps the map from
      shimmering as the fit slides (e.g. snapping the light-space origin to texel increments).
- [ ] The scene pass samples the map and shadows both pipelines — untextured **and** textured — so
      buildings shadow the road, the guardrail stripes the asphalt, and car bodies self-shadow.
- [ ] Filtering that is soft at the demo's resolution rather than a hard aliased edge (PCF or
      equivalent), with acne and peter-panning controlled by a **documented, justified** bias — state
      what the bias is in world units and why that value.
- [ ] Shadowing is driven by the **KE-0406 sun**: changing the sun's elevation/azimuth moves the
      shadows correspondingly, with no second source of truth for the light direction.
- [ ] Shadows respect the receiver, not a hardcoded plane: the road deck, the hill terrain at its own
      height, and building faces all receive correctly. (`ground_height`'s replacement, or its
      deletion — see KE-0406.)
- [ ] The fake `ground_shadow` blob and its now-unused uniform fields are **removed**, not left
      alongside the real path.
- [ ] Runs on a GPU-less CI runner without failing: the shadow pass must degrade or skip the same way
      the pixel-hash tests do, never panic.

## Technical notes
- TBDR: the shadow pass is depth-only, so it wants `storeAction` chosen deliberately — the map must
  survive to be sampled, unlike the memoryless MSAA the scene pass resolves in-tile. Getting this
  wrong is silent (you sample garbage), so assert it.
- `MTLPixelFormat::Depth32Float` is already the depth format in use for the scene pipelines.
- The light-space matrix must reach the shader. `LightUniforms` was 128 bytes and grows in KE-0406;
  a `float4x4` is another 64. Keep every `float3` 16-byte aligned and assert the final size — this
  struct has already caused one silent look-breaking padding bug.
- An orthographic projection is correct for a directional sun; do not reuse the perspective camera.
- Depth-only means no fragment shader work for casters — keep a dedicated minimal vertex function
  rather than reusing the lit pipelines' vertex stage with its normal/uv plumbing.

## Out of scope
Cascaded shadow maps. Contact-hardening / area-light softness. Transparent or alpha-tested casters.
Ray-traced shadows (the raytracer path is separate). Point/spot-light shadows — this is the sun only.

## Test gate
`cargo test --workspace` green. A **shadow pixel-hash** proving occlusion actually happens: a
reference scene where a caster demonstrably darkens a receiver, whose hash changes if the shadow pass
is disabled (verify that by disabling it, not by assuming). `REFERENCE_HASH` and
`TEXTURED_REFERENCE_HASH` re-blessed with justification; `OVERLAY_REFERENCE_HASH` unchanged. Metal
API Validation clean (no unset attachments, no store-action warnings). `--smoke` exits 0 headlessly;
clippy clean; macOS oracle green.

## Doc gate
`#![deny(missing_docs)]`; rustdoc clean. Document the pass order, the frustum-fit strategy and its
stability guarantee, the bias values and why, and the TBDR store-action reasoning. Update
`kaman-render/README.md` (pass order + the new baseline) and `docs/ARCHITECTURE.md`'s look-stack
section, which currently advertises the blob shadow.
