// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`ChaseController`] — a follow camera that trails a moving target.
//!
//! Given a target world position and a facing direction, the controller places
//! the camera **behind and above** the target and points it at the target (with a
//! small look-ahead height so the framing sits a touch above the ground). The
//! placement is a pure function of a few offsets, so it is fully deterministic and
//! engine-generic — it names no game type.
//!
//! # Placement math
//!
//! For a target at `T` moving toward unit `forward`, with world up `up`,
//! back-distance `distance`, and height `height`:
//!
//! ```text
//! camera_position = T - forward * distance + up * height
//! camera_target   = T + up * look_at_height
//! ```
//!
//! The camera therefore sits `distance` behind the target along its facing and
//! `height` above it, looking slightly up at the target.
//!
//! # Smoothing
//!
//! With a `smoothing` factor in `0.0..=1.0`, each [`follow`](ChaseController::follow)
//! lerps the camera's current pose toward the freshly-computed desired pose by
//! that fraction, so the camera eases in rather than snapping. `0.0` disables
//! smoothing (the camera jumps straight to the desired pose); `1.0` also snaps.
//! Values in between trail the target with an exponential ease.

use kaman_math::glam::Vec3;

use crate::camera::Camera;

/// A follow camera that trails a target from behind and above.
///
/// Configure the back-distance, height, and (optional) smoothing, then call
/// [`follow`](Self::follow) each frame with the target's position and facing to
/// drive a [`Camera`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaseController {
    /// Distance the camera trails behind the target, along the target's facing.
    distance: f32,
    /// Height the camera sits above the target, along the world up axis.
    height: f32,
    /// Height above the target the camera aims at (keeps framing slightly high).
    look_at_height: f32,
    /// World up axis used for the height offset and the look-at up vector.
    up: Vec3,
    /// Per-follow ease factor in `0.0..=1.0` (`0.0` snaps; smaller trails more).
    smoothing: f32,
}

impl ChaseController {
    /// A chase controller trailing `distance` behind and `height` above the
    /// target, with no smoothing (the camera snaps to the desired pose).
    ///
    /// Uses `+Y` as the world up axis and aims at the target itself
    /// (`look_at_height == 0`). Use the builder-style setters to adjust.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_camera::ChaseController;
    ///
    /// let chase = ChaseController::new(8.0, 4.0);
    /// ```
    #[must_use]
    pub fn new(distance: f32, height: f32) -> Self {
        Self {
            distance,
            height,
            look_at_height: 0.0,
            up: Vec3::Y,
            smoothing: 0.0,
        }
    }

    /// Set the world up axis (default `+Y`). Returns `self` for chaining.
    #[must_use]
    pub fn with_up(mut self, up: Vec3) -> Self {
        self.up = up.normalize();
        self
    }

    /// Set the height above the target the camera aims at (default `0.0`).
    /// Returns `self` for chaining.
    #[must_use]
    pub fn with_look_at_height(mut self, look_at_height: f32) -> Self {
        self.look_at_height = look_at_height;
        self
    }

    /// Set the per-follow ease factor in `0.0..=1.0` (default `0.0`, i.e. snap).
    ///
    /// Each [`follow`](Self::follow) moves the camera this fraction of the way to
    /// the desired pose, so smaller values trail the target more loosely. The
    /// value is clamped into `0.0..=1.0`. Returns `self` for chaining.
    #[must_use]
    pub fn with_smoothing(mut self, smoothing: f32) -> Self {
        self.smoothing = smoothing.clamp(0.0, 1.0);
        self
    }

    /// The desired camera position for a target at `target` facing `forward`.
    ///
    /// `camera_position = target - forward_norm * distance + up * height`.
    #[must_use]
    pub fn desired_position(&self, target: Vec3, forward: Vec3) -> Vec3 {
        let forward = normalize_or(forward, -Vec3::Z);
        target - forward * self.distance + self.up * self.height
    }

    /// The desired look-at point for a target at `target`
    /// (`target + up * look_at_height`).
    #[must_use]
    pub fn desired_target(&self, target: Vec3) -> Vec3 {
        target + self.up * self.look_at_height
    }

    /// Place `camera` to follow a target at `target` facing `forward`.
    ///
    /// Computes the desired behind-and-above pose, then (with smoothing) eases the
    /// camera's current position/target toward it, or (with no smoothing) snaps to
    /// it. `forward` need not be normalized; a zero `forward` falls back to `-Z`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_camera::{Camera, ChaseController};
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut camera = Camera::new(16.0 / 9.0);
    /// let chase = ChaseController::new(8.0, 4.0);
    /// // Target at the origin moving toward -Z.
    /// chase.follow(&mut camera, Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    /// // Camera sits behind (+Z) and above (+Y) the target.
    /// assert!(camera.position().z > 0.0);
    /// assert!(camera.position().y > 0.0);
    /// ```
    pub fn follow(&self, camera: &mut Camera, target: Vec3, forward: Vec3) {
        let desired_pos = self.desired_position(target, forward);
        let desired_target = self.desired_target(target);

        let (new_pos, new_target) = if self.smoothing <= 0.0 {
            (desired_pos, desired_target)
        } else {
            let t = self.smoothing;
            (
                camera.position().lerp(desired_pos, t),
                camera.target().lerp(desired_target, t),
            )
        };

        camera.set_position(new_pos);
        camera.set_target(new_target);
        camera.set_up(self.up);
    }
}

/// Normalize `v`, or return `fallback` if `v` is (near) zero-length.
fn normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let len = v.length();
    if len > 1e-6 {
        v / len
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_camera_behind_and_above_target() {
        let chase = ChaseController::new(8.0, 4.0);
        // Target moving toward -Z.
        let pos = chase.desired_position(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        // Behind (+Z by distance) and above (+Y by height).
        assert!((pos - Vec3::new(0.0, 4.0, 8.0)).length() < 1e-5);
    }

    #[test]
    fn follow_snaps_without_smoothing() {
        let mut cam = Camera::new(1.0);
        let chase = ChaseController::new(10.0, 5.0);
        chase.follow(&mut cam, Vec3::new(0.0, 0.0, -20.0), Vec3::new(0.0, 0.0, -1.0));
        assert!((cam.position() - Vec3::new(0.0, 5.0, -10.0)).length() < 1e-5);
        assert!((cam.target() - Vec3::new(0.0, 0.0, -20.0)).length() < 1e-5);
    }

    #[test]
    fn follow_tracks_target_movement() {
        let mut cam = Camera::new(1.0);
        let chase = ChaseController::new(6.0, 3.0);
        chase.follow(&mut cam, Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        let first = cam.position();
        chase.follow(&mut cam, Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -1.0));
        // The camera followed the target along -Z.
        assert!(cam.position().z < first.z);
    }

    #[test]
    fn smoothing_eases_toward_desired_pose() {
        let mut cam = Camera::new(1.0);
        cam.set_position(Vec3::ZERO);
        cam.set_target(Vec3::ZERO);
        let chase = ChaseController::new(10.0, 5.0).with_smoothing(0.5);
        let desired = chase.desired_position(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        chase.follow(&mut cam, Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        // Moved halfway toward the desired pose, not all the way.
        let halfway = Vec3::ZERO.lerp(desired, 0.5);
        assert!((cam.position() - halfway).length() < 1e-5);
        // Repeated follows converge on the desired pose.
        for _ in 0..40 {
            chase.follow(&mut cam, Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        }
        assert!((cam.position() - desired).length() < 1e-3);
    }

    #[test]
    fn zero_forward_falls_back_to_minus_z() {
        let chase = ChaseController::new(4.0, 0.0);
        let pos = chase.desired_position(Vec3::ZERO, Vec3::ZERO);
        // Fallback forward -Z ⇒ camera behind at +Z.
        assert!((pos - Vec3::new(0.0, 0.0, 4.0)).length() < 1e-5);
    }

    #[test]
    fn look_at_height_raises_the_target() {
        let chase = ChaseController::new(4.0, 2.0).with_look_at_height(1.0);
        let t = chase.desired_target(Vec3::new(1.0, 0.0, -3.0));
        assert!((t - Vec3::new(1.0, 1.0, -3.0)).length() < 1e-5);
    }
}
