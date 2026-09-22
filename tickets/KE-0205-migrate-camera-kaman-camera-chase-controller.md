# KE-0205 — Migrate camera → `kaman-camera` + chase controller

Phase:         2
Priority:      P1
Status:        Done
Integration:   Refactor
Size:          L · A2
Time:          M
Risk:          Med
Depends on:    KE-0102, KE-0204      Blocks: on-screen "is it fun?" review (KR2.4)
Serves:        KR2.4

## Problem / Motivation
KE-0102 inlined a minimal, static camera into `kaman-render` (flagged as a deviation) because
`kaman-camera` was a stub and the seam had no way to carry a view. Result: the game cannot control
the camera, so the KE-0204 runner drives *away* from a fixed viewpoint — the gameplay is correct
(tested headlessly) but not visually playable. This ticket migrates the prototype camera into
`crates/kaman-camera`, threads the view-projection through the render seam so the game owns the
camera, deletes the inlined camera, and adds a **chase controller** so the camera follows the car.
This also restores the intended migration order (camera before scene, INTEGRATION §2.2).

## Scope & Acceptance
- [x] **`kaman-camera`:** migrate `ProjectRigor/src/camera.rs` into the crate (currently a stub):
      `Camera` with `view_matrix`/`projection_matrix`/`view_projection_matrix`, aspect update,
      `set_position`/`set_target`, basis vectors as present. Depend on `kaman-math`.
- [x] **Chase controller:** add a follow camera — given a target position (and facing/axis), place
      the camera behind + above the target and look at it, with a configurable offset and optional
      smoothing. Engine-generic (no game types).
- [x] **Seam carries the view (A2):** add `set_view_projection(&mut self, view_proj: Mat4)` (or
      `set_camera`) to `FrameRecorder` in `kaman-render-api` (`Mat4` via `kaman-math`, **no metal**).
      Update the `NullRenderer` double to record it.
- [x] **`kaman-render`:** use the seam-provided view-projection for the MVP; **delete the inlined
      `camera.rs`** (the feature-gated raytracer, if it needs a camera, uses `kaman-camera`).
- [x] **`kaman-core`:** engine owns a `kaman_camera::Camera`; `EngineCtx` exposes `camera_mut()`;
      the driver pushes the camera's view-projection to the recorder each frame before `Game::render`;
      aspect updates on resize. Keep `kaman-core` **metal-free**; dep direction stays acyclic
      (`kaman-core → kaman-camera → kaman-math`).
- [x] **`car-runner`:** drive the chase controller so the camera follows the car → the car stays
      framed on screen. `cargo run -p car-runner` is now visually playable; `--smoke` still exits 0.
- [x] Pixel-hash (KE-0102) updated: the reference-scene view now comes through the seam. Re-bless the
      committed hash **with a written justification** (the camera path changed, not the geometry).

## Technical notes
- Keep the seam minimal: a single per-frame `set_view_projection` alongside `set_pipeline` is the
  natural place for view state; document its ordering (set before the first `draw_mesh`).
- Chase math: camera at `target - forward*dist + up*height`, looking at `target`; expose the offsets.
- `kaman-camera` is engine-generic — add a no-game-symbols guard if it grows game-ish surface.

## Out of scope
- FPS/orbit controllers beyond what the runner needs (migrate lazily if a consumer needs them).
- HUD / letterboxing / cinematic cameras (Phase 4).

## Test gate
`cargo test --workspace` green incl. `kaman-camera` unit tests (view/proj known values, chase
placement) and the updated `NullRenderer` view-projection recording; pixel-hash green (re-blessed
w/ justification); `--smoke` exits 0; clippy clean; firewall (`kaman-core`/`kaman-render-api` no
metal) holds.

## Doc gate
`#![deny(missing_docs)]`; `kaman-camera` README (responsibility + chase controller); seam method
documents its ordering contract; ARCHITECTURE note that the camera migrated and the inlined one was
removed; car-runner README notes the chase camera.
