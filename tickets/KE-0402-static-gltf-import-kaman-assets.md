# KE-0402 — Static glTF import (`kaman-assets`) + flexible vertex layout

Phase:         4
Priority:      P1
Status:        Todo
Integration:   New
Size:          L · A2
Time:          L
Risk:          Med
Depends on:    KE-0006, KE-0102, KE-0103      Blocks: KE-0403
Serves:        KR4.2

## Problem / Motivation
The runner draws procedurally-generated boxes. To use **real meshes**, stand up a new engine-generic
`kaman-assets` crate that imports static **glTF** (`.gltf`/`.glb`) into the engine's vertex/index
data on the render seam, and make the backend honor an arbitrary `VertexLayout` (today the Metal
backend hardcodes the 36-byte `[pos,normal,color]` layout — a KE-0102 deferral). The runner then
loads a real car/obstacle mesh instead of a box. Geometry half of KR4.2 (textures are KE-0403).

## Scope & Acceptance
- [ ] New crate `crates/kaman-assets` — **metal-free, engine-generic** (no game types); depends on
      `gltf`, `kaman-math`, `kaman-render-api`.
- [ ] Parse glTF meshes + node tree into a `SceneAsset` (meshes with position/normal/UV, and a node
      hierarchy of transforms); pack onto the seam's `MeshData`/`VertexLayout`.
- [ ] **Flexible vertex layout through the seam:** the `kaman-render` backend builds its vertex
      descriptor from the `VertexLayout` in `MeshData` (add UV/`Float32x2`), not a hardcoded stride.
      Keep the existing color path byte-identical (pixel-hash stable for the box scene).
- [ ] **Upload once, reference by handle:** assets load once into persistent buffers (the KE-0103
      registry discipline); a small load-once/dedup cache so one file parses+uploads once and is
      shared by handle across N instances.
- [ ] A `kaman-scene`/game helper spawns ECS entities from a `SceneAsset`'s node graph.
- [ ] `car-runner` loads a real static mesh and renders it in place of the box.

## Technical notes
- Optional cheap stepping stone: a tiny Wavefront `.obj` reader first to prove the file→seam path
  before pulling in `gltf` — keep it behind the same `SceneAsset` API if you do.
- The node tree bakes to world transforms for static v1; skinning/animation is explicitly out.

## Out of scope
- Textures/materials sampling (KE-0403). Normal/roughness maps + ASTC (KE-0403). Animation/skinning.

## Test gate
`cargo test -p kaman-assets` green (parse known-value meshes; layout packing; dedup cache);
box-scene pixel-hash stable; macOS oracle green; firewall (`kaman-assets` no metal); clippy clean.

## Doc gate
`#![deny(missing_docs)]`; `kaman-assets` README (import → seam mapping, cache); ARCHITECTURE note the
new crate + the vertex-layout-through-the-seam fix.
