# kaman-camera

Camera state and controller math for KamanEngine.

Owns the engine's camera model:

- **`Camera`** — a target-based perspective camera. Produces the view,
  projection, and view-projection matrices the render seam carries; supports
  aspect-ratio updates, `set_position` / `set_target`, and the forward / right
  basis vectors. Migrated (and trimmed to the seam-facing core) from the
  prototype camera.
- **`ChaseController`** — a follow camera. Given a target position and a facing
  direction it places the camera **behind and above** the target and points it
  at the target, with configurable back-distance, height, and optional smoothing
  (an exponential ease toward the desired pose). Engine-generic: it names no game
  type.

The view-projection matrix flows game → engine → render seam
(`FrameRecorder::set_view_projection`) → backend, so the game owns the camera and
the render backend stays camera-free.

Depends only on `kaman-math`. Part of the
[KamanEngine](../../README.md) workspace. Apache-2.0.
