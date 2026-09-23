# KE-0403 — Textures + ASTC + mipmaps (base-color sampling)

Phase:         4
Priority:      P1
Status:        Done
Integration:   New
Size:          M · A2
Time:          M
Risk:          Med
Depends on:    KE-0402      Blocks: —
Serves:        KR4.2

## Problem / Motivation
KE-0402 gives real geometry; this gives it real surfaces. Decode glTF images, upload them through
the seam's `create_texture`, and make the shader actually **sample the bound base-color texture** at
the mesh UV — closing the KE-0102 deviation where `MaterialParams`/textures are accepted at the seam
but never sampled. Add mipmaps and ASTC for mobile-safe texture memory/bandwidth.

## Scope & Acceptance
- [x] Decode glTF base-color images (`image` crate) → RGBA8 → `create_texture` (`TextureHandle`).
- [x] Shader samples the **bound base-color texture** at the UV; `bind_texture` wired end-to-end so
      a textured mesh renders with its texture (not vertex color).
- [x] **Mipmaps** generated for sampled textures; trilinear sampling.
- [x] **ASTC** compressed textures on device (decode/transcode or import pre-compressed); raw RGBA8
      acceptable on macOS as a fallback. `#[cfg]`/config split.
- [ ] Also plumb normal + roughness texture slots (sampled if present) toward a basic lit material.
      *(Deferred: base-color is sampled; normal + roughness are decoded and plumbed onto
      MeshAsset, but shader sampling of them is a follow-up.)*
- [x] `car-runner`'s mesh shows a real base-color texture.

## Technical notes
- Extends `MaterialParams`/the pipeline to carry texture bindings; keep untextured meshes working
  (the box path stays vertex-colored → its pixel-hash stays stable).
- Uploads use the load-once cache (KE-0402) — one image → one GPU texture shared by handle.

## Out of scope
- Full PBR/IBL (later look work). Streaming textures from disk mid-run. Video textures.

## Test gate
`cargo test --workspace` green (texture upload + sampling on an offscreen render; a textured-quad
pixel-hash); box-scene pixel-hash stable; macOS oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; document the texture path + ASTC/mipmap choices; note the KE-0102
material-sampling deviation is now closed.
