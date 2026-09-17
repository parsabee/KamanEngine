# KE-0104 — Uniform ring + argument buffers

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
The prototype allocates a fresh uniform buffer every frame (`renderer.rs:458`). The mobile-safe
pattern is a **persistent ring** of uniform storage written each frame at a rotating offset, plus
**argument buffers** to bind per-draw resources without per-draw allocation. This kills the last
per-frame allocation and sets up frames-in-flight (KE-0105).

## Scope & Acceptance
- [ ] Allocate a persistent uniform buffer (ring) once; each frame writes uniforms at a rotating
      offset — **no `new_buffer*` per frame**. Size the ring for the frames-in-flight count landed
      in KE-0105 (parameterize now, default acceptable until then).
- [ ] Move per-draw/per-object parameters into **argument buffers** bound from persistent storage;
      `draw_mesh`'s `material_params`/transform flow through the ring/argument buffer, not a new alloc.
- [ ] Respect `MTLBuffer` offset alignment (256-byte on Apple GPUs) when sub-allocating the ring.
- [ ] **Allocation instrument:** per-frame `new_buffer*` count `== 0` for the reference scene
      (combined with KE-0103 this satisfies KR1.2's "zero allocations in the per-frame path").
- [ ] Render pixel-hash from KE-0102 stable (or re-blessed with justification).

## Technical notes
- The ring must not be overwritten while an in-flight frame still reads it — the actual guard is
  the frames-in-flight semaphore in KE-0105; here, structure the ring so KE-0105 only has to pick
  the slot index (`frame_index % N`), not restructure allocation.
- Keep uniform struct layouts in sync with the MSL side (the shaders); document the layout
  contract next to the struct (mirrors the prototype's scene.rs layout comment).

## Out of scope
- The semaphore / triple-buffering itself (KE-0105). Mesh buffers (KE-0103). `.metallib` (KE-0107).

## Test gate
`cargo test -p kaman-render` green; per-frame allocation count `== 0`; uniform ring offset
alignment asserted in a test; pixel-hash stable (or re-blessed); oracle + clippy green.

## Doc gate
`#![deny(missing_docs)]`; ring + argument-buffer strategy documented incl. the alignment and
"don't stomp an in-flight slot" invariants; uniform layout contract stated next to the struct.
