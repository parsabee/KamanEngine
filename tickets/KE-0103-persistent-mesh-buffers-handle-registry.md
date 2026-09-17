# KE-0103 — Persistent mesh buffers + handle registry

Phase:         1
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          M · A2
Time:          M
Risk:          High
Depends on:    KE-0102      Blocks: KE-0105
Serves:        KR1.2

## Problem / Motivation
The prototype re-uploads mesh geometry with `new_buffer_with_data` on the hot path. A mobile-safe
renderer uploads each mesh **once** and keeps a persistent `MTLBuffer`, referencing it by the
`MeshHandle` from the seam. This removes per-frame mesh allocations and gives the draw path a
stable registry to look meshes up in.

## Scope & Acceptance
- [ ] `RenderDevice::create_mesh` uploads vertex/index data into a persistent `MTLBuffer` **once**
      and returns a `MeshHandle`; `destroy_mesh` frees it. Store them in a registry inside
      `kaman-render` (e.g. generational slotmap keyed by `MeshHandle`).
- [ ] `FrameRecorder::draw_mesh` looks the buffer up by handle — **no allocation** in the draw path.
- [ ] Move all mesh uploads to load time (game/scene setup), not the per-frame path.
- [ ] **Allocation instrument:** a test/counter proves zero `new_buffer*` calls occur during the
      per-frame path for the reference scene (see KR1.2). Wire a debug hook or wrap the device so
      per-frame allocations are counted and asserted `== 0`.
- [ ] Render pixel-hash from KE-0102 unchanged (behavior identical) — or re-blessed with written
      justification if a legitimate change.

## Technical notes
- Generational handles guard against stale-`MeshHandle` use after `destroy_mesh`; a lookup on a
  freed/old handle must be a defined error, not a silent wrong-buffer draw — state this invariant
  in the doc and enforce it with a test.
- Coordinate with KE-0104: mesh buffers (static, persistent) vs uniforms (per-frame, ring) are
  different lifetimes; keep the registries separate.

## Out of scope
- Uniform/argument buffers (KE-0104). Frames-in-flight (KE-0105). Instancing (Phase 4+).

## Test gate
`cargo test -p kaman-render` green; per-frame allocation count `== 0` for the reference scene;
pixel-hash stable (or re-blessed w/ justification); oracle + clippy green.

## Doc gate
`#![deny(missing_docs)]`; handle-registry ownership + stale-handle invariant documented and
tested; README notes the "upload once, reference by handle" rule.
