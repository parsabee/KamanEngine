// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Minimal view/projection camera inlined into the render backend.
//!
//! # Deviation note (temporary)
//!
//! `kaman-camera` is still a stub and Phase 1 has no dedicated camera-migration
//! ticket, but the raster backend needs a view/projection matrix to place the
//! reference scene on screen. This is a **minimal** look-at + perspective camera,
//! carrying only what the draw path consumes (`view_projection_matrix`). It is
//! intentionally not the full prototype camera (no orbit/pan/zoom controllers);
//! a later camera-migration ticket should replace this with the real
//! `kaman-camera` type and delete this module. Flagged as a deviation in the
//! KE-0102 report.

use kaman_math::glam::{Mat4, Vec3};

/// A perspective look-at camera producing a view-projection matrix.
///
/// The camera sits at `position` and looks toward `target` with `up` as the
/// world up axis, using a right-handed perspective projection. It exposes only
/// the matrix the backend needs plus the basis vectors the (feature-gated) ray
/// tracer reads.
#[derive(Debug, Clone, Copy)]
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
    /// Create a camera with the given aspect ratio and the reference-scene
    /// defaults (45° vertical FOV, `0.1..100.0` clip range).
    #[must_use]
    pub fn new(aspect_ratio: f32) -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 8.0),
            target: Vec3::new(0.0, 5.0, -5.0),
            up: Vec3::new(0.0, 1.0, 0.0),
            fov_y: 45.0_f32.to_radians(),
            aspect_ratio,
            near: 0.1,
            far: 100.0,
        }
    }

    /// Set the camera's world position.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Set the point the camera looks at.
    pub fn set_target(&mut self, target: Vec3) {
        self.target = target;
    }

    /// Update the viewport aspect ratio (width / height).
    pub fn set_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio;
    }

    /// The camera world position.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// The point the camera looks at.
    #[must_use]
    pub fn target(&self) -> Vec3 {
        self.target
    }

    /// Right-handed view matrix (world → view space).
    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
    }

    /// Right-handed perspective projection matrix (view → clip space).
    #[must_use]
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect_ratio, self.near, self.far)
    }

    /// Combined view-projection matrix (world → clip space).
    #[must_use]
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Normalized forward direction (position → target). Used by the ray tracer.
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize()
    }

    /// Normalized right direction. Used by the ray tracer.
    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.forward().cross(self.up).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_projection_is_not_identity() {
        let cam = Camera::new(16.0 / 9.0);
        assert_ne!(cam.view_projection_matrix(), Mat4::IDENTITY);
        assert!(cam.view_projection_matrix().is_finite());
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
        let cam = Camera::new(1.0);
        assert!((cam.forward().length() - 1.0).abs() < 1e-5);
        assert!((cam.right().length() - 1.0).abs() < 1e-5);
        assert!(cam.forward().dot(cam.right()).abs() < 1e-4);
    }

    #[test]
    fn aspect_ratio_change_alters_projection() {
        let mut cam = Camera::new(1.0);
        let before = cam.projection_matrix();
        cam.set_aspect_ratio(2.0);
        assert_ne!(before, cam.projection_matrix());
    }
}
