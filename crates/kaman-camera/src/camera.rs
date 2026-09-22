// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! A target-based perspective [`Camera`] producing view / projection matrices.
//!
//! This is the migrated core of the prototype camera, trimmed to what the
//! engine's render seam actually consumes: a look-at view
//! matrix, a right-handed perspective projection, their product, an aspect
//! update, position/target setters, and the forward/right basis vectors the
//! (feature-gated) ray tracer reads. The prototype's free-fly `move_*` / orbit
//! `rotate_*` controllers are intentionally not migrated here — the runner needs
//! a chase camera (see [`crate::chase`]), and those controllers migrate lazily if
//! a consumer needs them.

use kaman_math::glam::{Mat4, Vec3};

/// A perspective camera positioned at `position` and looking toward `target`.
///
/// The camera uses a right-handed "look-at" system with `up` as the world up
/// axis and a right-handed perspective projection. It exposes the view /
/// projection / view-projection matrices the render seam carries, plus the
/// forward / right basis vectors.
///
/// # Example
///
/// ```rust
/// use kaman_camera::Camera;
/// use kaman_math::glam::Vec3;
///
/// let mut camera = Camera::new(16.0 / 9.0);
/// camera.set_position(Vec3::new(0.0, 5.0, 10.0));
/// camera.set_target(Vec3::ZERO);
/// let view_projection = camera.view_projection_matrix();
/// assert!(view_projection.is_finite());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    position: Vec3,
    target: Vec3,
    up: Vec3,
    fov_y: f32,
    aspect_ratio: f32,
    near: f32,
    far: f32,
}

impl Camera {
    /// Create a camera with the given viewport aspect ratio and sensible
    /// defaults: 45° vertical FOV, a `0.1..100.0` clip range, `+Y` up, and an
    /// elevated, pulled-back placement looking at the origin.
    ///
    /// # Arguments
    ///
    /// * `aspect_ratio` - viewport width / height (e.g. `16.0 / 9.0`).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_camera::Camera;
    ///
    /// let camera = Camera::new(16.0 / 9.0);
    /// ```
    #[must_use]
    pub fn new(aspect_ratio: f32) -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 8.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y: 45.0_f32.to_radians(),
            aspect_ratio,
            near: 0.1,
            far: 100.0,
        }
    }

    /// The right-handed view matrix (world → view space).
    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
    }

    /// The right-handed perspective projection matrix (view → clip space).
    #[must_use]
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect_ratio, self.near, self.far)
    }

    /// The combined view-projection matrix (world → clip space).
    ///
    /// Equivalent to `projection_matrix() * view_matrix()`.
    #[must_use]
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Update the viewport aspect ratio (width / height); call this on resize.
    pub fn set_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio;
    }

    /// Set the camera's world position.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Set the point the camera looks at.
    pub fn set_target(&mut self, target: Vec3) {
        self.target = target;
    }

    /// Set the world up axis used to orient the view.
    pub fn set_up(&mut self, up: Vec3) {
        self.up = up;
    }

    /// The camera's current world position.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// The point the camera is currently looking at.
    #[must_use]
    pub fn target(&self) -> Vec3 {
        self.target
    }

    /// The world up axis.
    #[must_use]
    pub fn up(&self) -> Vec3 {
        self.up
    }

    /// The normalized forward direction (position → target).
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize()
    }

    /// The normalized right direction (perpendicular to forward and up).
    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.forward().cross(self.up).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_projection_is_finite_and_not_identity() {
        let cam = Camera::new(16.0 / 9.0);
        assert_ne!(cam.view_projection_matrix(), Mat4::IDENTITY);
        assert!(cam.view_projection_matrix().is_finite());
    }

    #[test]
    fn view_projection_is_projection_times_view() {
        let cam = Camera::new(4.0 / 3.0);
        let expected = cam.projection_matrix() * cam.view_matrix();
        assert_eq!(cam.view_projection_matrix(), expected);
    }

    #[test]
    fn setters_update_state() {
        let mut cam = Camera::new(1.0);
        cam.set_position(Vec3::new(1.0, 2.0, 3.0));
        cam.set_target(Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(cam.position(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(cam.target(), Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn forward_and_right_are_unit_and_orthogonal() {
        let mut cam = Camera::new(1.0);
        cam.set_position(Vec3::new(0.0, 0.0, 5.0));
        cam.set_target(Vec3::ZERO);
        assert!((cam.forward().length() - 1.0).abs() < 1e-5);
        assert!((cam.right().length() - 1.0).abs() < 1e-5);
        assert!(cam.forward().dot(cam.right()).abs() < 1e-4);
    }

    #[test]
    fn forward_points_from_position_to_target() {
        let mut cam = Camera::new(1.0);
        cam.set_position(Vec3::new(0.0, 0.0, 10.0));
        cam.set_target(Vec3::ZERO);
        // Looking down -Z.
        assert!((cam.forward() - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5);
    }

    #[test]
    fn aspect_ratio_change_alters_projection() {
        let mut cam = Camera::new(1.0);
        let before = cam.projection_matrix();
        cam.set_aspect_ratio(2.0);
        assert_ne!(before, cam.projection_matrix());
    }

    #[test]
    fn known_projection_entries() {
        // Right-handed perspective: m[0][0] = f / aspect, m[1][1] = f, where
        // f = 1 / tan(fov_y / 2). Check the aspect scaling is applied to X.
        let cam = Camera::new(2.0);
        let proj = cam.projection_matrix();
        let cols = proj.to_cols_array_2d();
        let f = 1.0 / (45.0_f32.to_radians() / 2.0).tan();
        assert!((cols[0][0] - f / 2.0).abs() < 1e-4);
        assert!((cols[1][1] - f).abs() < 1e-4);
    }
}
