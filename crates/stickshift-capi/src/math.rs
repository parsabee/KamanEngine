// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Math helpers: pure, thread-safe, never fail. Right-handed, Y up, Metal clip
//! depth `0..1`, column-major matrices; the same conventions as `kaman-camera`.

use kaman_math::glam::{Mat4, Quat, Vec3};

use crate::types::{KamanColor, KamanMat4, KamanQuat, KamanVec3};

impl From<KamanVec3> for Vec3 {
    fn from(v: KamanVec3) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

impl From<Vec3> for KamanVec3 {
    fn from(v: Vec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }
}

impl From<KamanQuat> for Quat {
    fn from(q: KamanQuat) -> Self {
        Quat::from_xyzw(q.x, q.y, q.z, q.w)
    }
}

impl From<Quat> for KamanQuat {
    fn from(q: Quat) -> Self {
        Self {
            x: q.x,
            y: q.y,
            z: q.z,
            w: q.w,
        }
    }
}

impl From<KamanMat4> for Mat4 {
    fn from(m: KamanMat4) -> Self {
        Mat4::from_cols_array(&m.m)
    }
}

impl From<Mat4> for KamanMat4 {
    fn from(m: Mat4) -> Self {
        Self {
            m: m.to_cols_array(),
        }
    }
}

impl KamanColor {
    pub(crate) fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// The identity matrix.
#[no_mangle]
pub extern "C" fn kaman_mat4_identity() -> KamanMat4 {
    Mat4::IDENTITY.into()
}

/// `a × b`: the transform that applies `b` first, then `a`.
#[no_mangle]
pub extern "C" fn kaman_mat4_mul(a: KamanMat4, b: KamanMat4) -> KamanMat4 {
    (Mat4::from(a) * Mat4::from(b)).into()
}

/// Translation × rotation × scale.
#[no_mangle]
pub extern "C" fn kaman_mat4_from_trs(
    translation: KamanVec3,
    rotation: KamanQuat,
    scale: KamanVec3,
) -> KamanMat4 {
    Mat4::from_scale_rotation_translation(scale.into(), rotation.into(), translation.into()).into()
}

/// A right-handed view matrix looking from `eye` toward `target`.
#[no_mangle]
pub extern "C" fn kaman_mat4_look_at(
    eye: KamanVec3,
    target: KamanVec3,
    up: KamanVec3,
) -> KamanMat4 {
    Mat4::look_at_rh(eye.into(), target.into(), up.into()).into()
}

/// A right-handed perspective projection with Metal's `0..1` clip depth: the
/// projection `kaman-camera` uses. `fov_y` is in radians.
#[no_mangle]
pub extern "C" fn kaman_mat4_perspective(
    fov_y: f32,
    aspect: f32,
    z_near: f32,
    z_far: f32,
) -> KamanMat4 {
    Mat4::perspective_rh(fov_y, aspect, z_near, z_far).into()
}

/// A rotation of `angle` radians about `axis` (normalised here; a zero axis
/// gives the identity).
#[no_mangle]
pub extern "C" fn kaman_quat_from_axis_angle(axis: KamanVec3, angle: f32) -> KamanQuat {
    match Vec3::from(axis).try_normalize() {
        Some(axis) => Quat::from_axis_angle(axis, angle).into(),
        None => Quat::IDENTITY.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, z: f32) -> KamanVec3 {
        KamanVec3 { x, y, z }
    }

    #[test]
    fn matrices_are_column_major_and_match_glam() {
        let t = kaman_mat4_from_trs(v(1.0, 2.0, 3.0), Quat::IDENTITY.into(), v(1.0, 1.0, 1.0));
        // The translation is the fourth column: m[12..15].
        assert_eq!(&t.m[12..15], &[1.0, 2.0, 3.0]);
        let product = kaman_mat4_mul(t, kaman_mat4_identity());
        assert_eq!(product, t);
    }

    #[test]
    fn the_projection_is_the_cameras() {
        // kaman-camera's defaults: 45° vertical FOV, near 0.1, far 100.
        let camera = kaman_camera::Camera::new(1.5);
        let ours = Mat4::from(kaman_mat4_perspective(45f32.to_radians(), 1.5, 0.1, 100.0));
        assert_eq!(ours, camera.projection_matrix());
    }

    #[test]
    fn a_zero_axis_rotation_is_the_identity() {
        assert_eq!(
            kaman_quat_from_axis_angle(v(0.0, 0.0, 0.0), 1.0),
            Quat::IDENTITY.into()
        );
        let q = Quat::from(kaman_quat_from_axis_angle(v(0.0, 2.0, 0.0), 1.0));
        assert!((q.length() - 1.0).abs() < 1e-6, "the axis was normalised");
    }

    #[test]
    fn look_at_puts_the_target_down_negative_z() {
        let view = Mat4::from(kaman_mat4_look_at(
            v(0.0, 0.0, 5.0),
            v(0.0, 0.0, 0.0),
            v(0.0, 1.0, 0.0),
        ));
        let target_in_view = view.transform_point3(Vec3::ZERO);
        assert!((target_in_view - Vec3::new(0.0, 0.0, -5.0)).length() < 1e-5);
    }
}
