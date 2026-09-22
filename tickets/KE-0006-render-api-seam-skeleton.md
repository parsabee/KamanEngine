# KE-0006 — `kaman-render-api` seam skeleton

Phase:         0
Priority:      P0
Status:        Done
Integration:   New
Size:          S · A3
Time:          S
Risk:          Med
Depends on:    KE-0001      Blocks: KE-0102 (renderer migration)
Serves:        KR0.4

## Problem / Motivation
The render seam (ARCHITECTURE §2) must exist before the renderer migrates, so `kaman-render`
implements a contract rather than exporting Metal types. This is the firewall that keeps a
renderer rewrite from rippling into ECS/scene/physics.

## Scope & Acceptance
- [x] Define opaque handles: `MeshHandle`, `TextureHandle`, `PipelineHandle` (newtypes, no
      Metal types).
- [x] Define `RenderDevice` trait: create/destroy mesh, texture, pipeline (load-time ops).
- [x] Define `FrameRecorder` trait: `begin_frame`, `set_pipeline`, `bind_texture`,
      `draw_mesh(handle, transform, material_params)`, `submit`.
- [x] Define plain-data descriptor structs (vertex layout, material params) with **no** `metal`
      dependency in this crate's `Cargo.toml`.
- [x] A `NullRenderer` test-double implementing both traits (records calls) so scene/game code
      can be unit-tested headlessly without a GPU.

## Technical notes
- `kaman-render-api` depends only on `kaman-math` (for `Transform`) — never on `metal`. A CI
  check asserts `metal` is absent from this crate's dependency tree.

## Out of scope
- The Metal implementation (KE-0102). Instancing/argument-buffer specifics (Phase 1 tickets refine the trait).

## Test gate
`NullRenderer` unit tests green; dependency-tree check confirms no `metal` in `kaman-render-api`.

## Doc gate
`#![deny(missing_docs)]`; every trait method documents its contract + invariants; crate
`README` explains the seam and the "no Metal above this line" rule.
