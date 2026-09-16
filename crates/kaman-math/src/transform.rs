// Copyright (c) 2025 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! 3D transformation mathematics.
//!
//! This module provides the [`Transform`] struct for representing 3D transformations
//! using position, rotation, and scale. It's built on top of the [`glam`] library
//! for efficient vector and matrix operations.
//!
//! # Example
//!
//! ```rust
//! use kaman_math::Transform;
//! use kaman_math::glam::Vec3;
//!
//! // Create a transform at a position
//! let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
//!
//! // Transform a point from local to world space
//! let local_point = Vec3::new(1.0, 0.0, 0.0);
//! let world_point = transform.transform_point(local_point);
//! ```

use glam::{Mat4, Quat, Vec3};

/// A 3D transformation consisting of position, rotation, and scale.
///
/// The `Transform` struct represents a complete 3D transformation using:
/// - **Position**: Translation in 3D space (Vec3)
/// - **Rotation**: Orientation as a quaternion (Quat)
/// - **Scale**: Non-uniform scaling (Vec3)
///
/// This follows the standard TRS (Translation-Rotation-Scale) order when
/// converted to a matrix.
///
/// # Example
///
/// ```rust
/// use kaman_math::Transform;
/// use kaman_math::glam::{Vec3, Quat};
/// use std::f32::consts::PI;
///
/// let mut transform = Transform::from_position(Vec3::new(0.0, 5.0, 0.0));
/// transform.rotate(Quat::from_rotation_y(PI / 4.0)); // Rotate 45° around Y
/// transform.scale = Vec3::new(2.0, 1.0, 2.0); // Scale 2x on X and Z
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    /// Position in 3D world space
    pub position: Vec3,
    /// Rotation as a unit quaternion
    pub rotation: Quat,
    /// Scale factors along each axis
    pub scale: Vec3,
}

impl Transform {
    /// Creates an identity transform (no translation, rotation, or scaling).
    ///
    /// This is equivalent to:
    /// - Position: (0, 0, 0)
    /// - Rotation: no rotation (identity quaternion)
    /// - Scale: (1, 1, 1)
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::{Vec3, Quat};
    ///
    /// let transform = Transform::identity();
    /// assert_eq!(transform.position, Vec3::ZERO);
    /// assert_eq!(transform.rotation, Quat::IDENTITY);
    /// assert_eq!(transform.scale, Vec3::ONE);
    /// ```
    pub fn identity() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Creates a transform at the specified position with default rotation and scale.
    ///
    /// # Arguments
    ///
    /// * `position` - The position in 3D world space
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
    /// assert_eq!(transform.position, Vec3::new(1.0, 2.0, 3.0));
    /// ```
    pub fn from_position(position: Vec3) -> Self {
        Self {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Creates a transform with the specified position and rotation.
    ///
    /// Scale defaults to (1, 1, 1).
    ///
    /// # Arguments
    ///
    /// * `position` - The position in 3D world space
    /// * `rotation` - The rotation as a quaternion
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::{Vec3, Quat};
    /// use std::f32::consts::PI;
    ///
    /// let rotation = Quat::from_rotation_y(PI / 2.0); // 90° around Y axis
    /// let transform = Transform::from_position_rotation(
    ///     Vec3::new(0.0, 5.0, 0.0),
    ///     rotation
    /// );
    /// ```
    pub fn from_position_rotation(position: Vec3, rotation: Quat) -> Self {
        Self {
            position,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Converts this transform to a 4×4 transformation matrix.
    ///
    /// The resulting matrix applies transformations in the standard order:
    /// Scale → Rotate → Translate (TRS).
    ///
    /// # Returns
    ///
    /// A 4×4 matrix that can transform homogeneous coordinates.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
    /// let matrix = transform.to_matrix();
    /// let point = matrix.transform_point3(Vec3::ZERO);
    /// assert_eq!(point, Vec3::new(1.0, 2.0, 3.0));
    /// ```
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }

    /// Transforms a point from local space to world space.
    ///
    /// Applies scale, rotation, and translation to the point.
    ///
    /// # Arguments
    ///
    /// * `point` - The point in local space
    ///
    /// # Returns
    ///
    /// The point transformed to world space.
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let transform = Transform::from_position(Vec3::new(5.0, 0.0, 0.0));
    /// let local = Vec3::new(1.0, 0.0, 0.0);
    /// let world = transform.transform_point(local);
    /// assert_eq!(world, Vec3::new(6.0, 0.0, 0.0));
    /// ```
    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        self.rotation * (point * self.scale) + self.position
    }

    /// Transforms a vector (direction) from local space to world space.
    ///
    /// Applies scale and rotation but NOT translation (since directions don't have position).
    ///
    /// # Arguments
    ///
    /// * `vector` - The vector in local space
    ///
    /// # Returns
    ///
    /// The vector transformed to world space (without translation).
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::{Vec3, Quat};
    /// use std::f32::consts::PI;
    ///
    /// let rotation = Quat::from_rotation_y(PI / 2.0);
    /// let transform = Transform::from_position_rotation(Vec3::new(10.0, 0.0, 0.0), rotation);
    /// let forward = Vec3::new(0.0, 0.0, 1.0);
    /// let rotated = transform.transform_vector(forward);
    /// // Direction is rotated but not translated
    /// ```
    pub fn transform_vector(&self, vector: Vec3) -> Vec3 {
        self.rotation * (vector * self.scale)
    }

    /// Rotates this transform by the given quaternion.
    ///
    /// The rotation is applied in world space (pre-multiplied).
    ///
    /// # Arguments
    ///
    /// * `rotation` - The rotation to apply
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Quat;
    /// use std::f32::consts::PI;
    ///
    /// let mut transform = Transform::identity();
    /// transform.rotate(Quat::from_rotation_y(PI / 2.0)); // Rotate 90° around Y
    /// ```
    pub fn rotate(&mut self, rotation: Quat) {
        self.rotation = rotation * self.rotation;
    }

    /// Translates this transform by the given vector.
    ///
    /// # Arguments
    ///
    /// * `translation` - The translation to apply
    ///
    /// # Example
    ///
    /// ```rust
    /// use kaman_math::Transform;
    /// use kaman_math::glam::Vec3;
    ///
    /// let mut transform = Transform::identity();
    /// transform.translate(Vec3::new(1.0, 2.0, 3.0));
    /// assert_eq!(transform.position, Vec3::new(1.0, 2.0, 3.0));
    /// ```
    pub fn translate(&mut self, translation: Vec3) {
        self.position += translation;
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_identity_transform() {
        let transform = Transform::identity();
        assert_eq!(transform.position, Vec3::ZERO);
        assert_eq!(transform.rotation, Quat::IDENTITY);
        assert_eq!(transform.scale, Vec3::ONE);
    }

    #[test]
    fn test_transform_point() {
        let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
        let point = Vec3::new(1.0, 0.0, 0.0);
        let transformed = transform.transform_point(point);
        assert_eq!(transformed, Vec3::new(2.0, 2.0, 3.0));
    }

    #[test]
    fn test_transform_rotation() {
        let rotation = Quat::from_rotation_y(PI / 2.0); // 90 degrees around Y
        let transform = Transform::from_position_rotation(Vec3::ZERO, rotation);
        let point = Vec3::new(1.0, 0.0, 0.0);
        let transformed = transform.transform_point(point);

        // After 90° rotation around Y, X becomes -Z
        assert!((transformed.x - 0.0).abs() < 0.001);
        assert!((transformed.y - 0.0).abs() < 0.001);
        assert!((transformed.z - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_to_matrix() {
        let transform = Transform::from_position(Vec3::new(1.0, 2.0, 3.0));
        let matrix = transform.to_matrix();
        let point = Vec3::new(1.0, 0.0, 0.0);
        let transformed = matrix.transform_point3(point);
        assert_eq!(transformed, Vec3::new(2.0, 2.0, 3.0));
    }

    #[test]
    fn test_translate() {
        let mut transform = Transform::identity();
        transform.translate(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.position, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_rotate() {
        let mut transform = Transform::identity();
        let rotation = Quat::from_rotation_y(PI / 2.0);
        transform.rotate(rotation);

        let point = Vec3::new(1.0, 0.0, 0.0);
        let transformed = transform.transform_point(point);
        assert!((transformed.z - (-1.0)).abs() < 0.001);
    }

    // Table-driven known-value tests mirroring the prototype's `examples/math_test.rs`
    // expectations for the transform/projection helpers.
    #[test]
    fn test_transform_point_table() {
        // (transform, input point, expected world point)
        let cases: &[(Transform, Vec3, Vec3)] = &[
            // Pure translation (from math_test.rs test 2).
            (
                Transform::from_position(Vec3::new(5.0, 10.0, 15.0)),
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(6.0, 12.0, 18.0),
            ),
            // Identity leaves the point untouched.
            (Transform::identity(), Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 3.0)),
            // Translation only, second sample.
            (
                Transform::from_position(Vec3::new(5.0, 0.0, 0.0)),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(6.0, 0.0, 0.0),
            ),
        ];

        for (transform, input, expected) in cases {
            let got = transform.transform_point(*input);
            assert!(
                (got - *expected).length() < 1e-4,
                "transform_point({input:?}) = {got:?}, expected {expected:?}",
            );
        }
    }

    #[test]
    fn test_to_matrix_matches_transform_point() {
        // `to_matrix().transform_point3()` must agree with `transform_point()`
        // across a spread of TRS configurations.
        let cases: &[(Transform, Vec3)] = &[
            (Transform::from_position(Vec3::new(1.0, 2.0, 3.0)), Vec3::new(1.0, 0.0, 0.0)),
            (
                Transform::from_position_rotation(
                    Vec3::new(0.0, 5.0, 0.0),
                    Quat::from_rotation_y(PI / 2.0),
                ),
                Vec3::new(1.0, 0.0, 0.0),
            ),
            (
                {
                    let mut t = Transform::from_position(Vec3::new(-2.0, 4.0, 1.0));
                    t.scale = Vec3::new(2.0, 1.0, 3.0);
                    t.rotation = Quat::from_rotation_z(PI / 4.0);
                    t
                },
                Vec3::new(1.0, 2.0, -1.0),
            ),
        ];

        for (transform, input) in cases {
            let via_helper = transform.transform_point(*input);
            let via_matrix = transform.to_matrix().transform_point3(*input);
            assert!(
                (via_helper - via_matrix).length() < 1e-4,
                "helper {via_helper:?} != matrix {via_matrix:?} for input {input:?}",
            );
        }
    }
}
