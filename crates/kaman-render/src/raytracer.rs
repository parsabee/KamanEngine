// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! CPU-side ray tracer and GPU triangle layouts (feature `raytracer`, off by
//! default).
//!
//! This module is gated behind the off-by-default `raytracer` cargo feature.
//! The default build is rasterization-only and never compiles
//! `shaders/raytracing.metal`. KE-0106 formalizes the gating (and the iOS
//! exclusion); this port simply brings the entangled ray-tracing code along
//! behind a feature so the default path stays lean and green.
//!
//! The CPU [`trace_ray`] is kept in sync with `shaders/raytracing.metal` and is
//! used for validation.

use kaman_math::glam::Vec3;

use crate::vertex::LightUniforms;

const EPSILON: f32 = 0.001;
const INTERSECTION_EPSILON: f32 = 0.000001;

/// Triangle geometry with material properties and an AABB, matching the Metal
/// `Triangle` layout in `raytracing.metal` (124 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    /// Vertex 0 position.
    pub p0: [f32; 3],
    /// Vertex 1 position.
    pub p1: [f32; 3],
    /// Vertex 2 position.
    pub p2: [f32; 3],
    /// Normal at vertex 0.
    pub n0: [f32; 3],
    /// Normal at vertex 1.
    pub n1: [f32; 3],
    /// Normal at vertex 2.
    pub n2: [f32; 3],
    /// Base RGB color.
    pub color: [f32; 3],
    /// Bounding-box minimum corner.
    pub aabb_min: [f32; 3],
    /// Bounding-box maximum corner.
    pub aabb_max: [f32; 3],
    /// Surface reflectivity (0 = matte, 1 = mirror).
    pub reflectivity: f32,
    /// Alignment padding.
    pub _padding1: f32,
    /// Alignment padding.
    pub _padding2: f32,
    /// Alignment padding.
    pub _padding3: f32,
}

/// Parameters passed to the compute ray-tracing kernel, matching
/// `RayTracingParams` in `raytracing.metal`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RayTracingParams {
    /// Camera world position.
    pub camera_position: [f32; 3],
    /// Viewport aspect ratio.
    pub aspect_ratio: f32,
    /// Camera look-at target.
    pub camera_target: [f32; 3],
    /// Maximum reflection bounces.
    pub max_depth: i32,
    /// Background/miss color.
    pub background_color: [f32; 3],
    /// Legacy default reflectivity.
    pub default_reflectivity: f32,
    /// Precomputed camera forward basis.
    pub camera_forward: [f32; 3],
    /// Alignment padding.
    pub _padding1: f32,
    /// Precomputed camera right basis.
    pub camera_right: [f32; 3],
    /// Alignment padding.
    pub _padding2: f32,
    /// Precomputed camera up basis.
    pub camera_up: [f32; 3],
    /// Alignment padding.
    pub _padding3: f32,
}

/// Ray-triangle intersection via the Möller–Trumbore algorithm.
///
/// Returns `(t, u, v)` (distance + barycentric coords) on a hit.
fn ray_triangle_intersection(
    ray_origin: Vec3,
    ray_dir: Vec3,
    tri: &Triangle,
) -> Option<(f32, f32, f32)> {
    let p0 = Vec3::from(tri.p0);
    let p1 = Vec3::from(tri.p1);
    let p2 = Vec3::from(tri.p2);

    let edge1 = p1 - p0;
    let edge2 = p2 - p0;

    let h = ray_dir.cross(edge2);
    let a = edge1.dot(h);

    if a.abs() < INTERSECTION_EPSILON {
        return None;
    }

    let f = 1.0 / a;
    let s = ray_origin - p0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let q = s.cross(edge1);
    let v = f * ray_dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let t = f * edge2.dot(q);
    if t > INTERSECTION_EPSILON {
        Some((t, u, v))
    } else {
        None
    }
}

/// Trace a ray through `triangles` and return its color. Mirrors the Metal
/// kernel for CPU-side validation.
#[must_use]
pub fn trace_ray(
    ray_origin: Vec3,
    ray_dir: Vec3,
    triangles: &[Triangle],
    light: &LightUniforms,
    depth: i32,
    max_depth: i32,
) -> Vec3 {
    if depth >= max_depth {
        return Vec3::ZERO;
    }

    let mut closest_t = f32::INFINITY;
    let mut closest_idx: Option<usize> = None;
    let mut closest_u = 0.0;
    let mut closest_v = 0.0;

    for (i, tri) in triangles.iter().enumerate() {
        if let Some((t, u, v)) = ray_triangle_intersection(ray_origin, ray_dir, tri) {
            if t < closest_t {
                closest_t = t;
                closest_idx = Some(i);
                closest_u = u;
                closest_v = v;
            }
        }
    }

    let Some(idx) = closest_idx else {
        return Vec3::new(0.2, 0.0, 0.2);
    };

    let tri = &triangles[idx];
    let w = 1.0 - closest_u - closest_v;

    let n0 = Vec3::from(tri.n0);
    let n1 = Vec3::from(tri.n1);
    let n2 = Vec3::from(tri.n2);
    let normal = (w * n0 + closest_u * n1 + closest_v * n2).normalize();

    let hit_point = ray_origin + ray_dir * closest_t;

    let light_dir = Vec3::from(light.direction).normalize();
    let ndotl = normal.dot(-light_dir).max(0.0);

    let light_color = Vec3::from(light.color);
    let ambient = light.ambient_intensity * light_color;
    let diffuse = light.diffuse_intensity * ndotl * light_color;

    let mut specular = Vec3::ZERO;
    if ndotl > 0.0 {
        let view_dir = -ray_dir;
        let half_dir = (-light_dir + view_dir).normalize();
        let spec = normal.dot(half_dir).max(0.0).powf(light.shininess);
        specular = light.specular_intensity * spec * light_color;
    }

    let lighting = ambient + diffuse + specular;
    let local_color = Vec3::from(tri.color) * lighting;
    let reflectivity = tri.reflectivity;

    if depth < max_depth - 1 {
        let reflection_dir = ray_dir - 2.0 * ray_dir.dot(normal) * normal;
        let reflection_origin = hit_point + normal * EPSILON;
        let reflection_color = trace_ray(
            reflection_origin,
            reflection_dir,
            triangles,
            light,
            depth + 1,
            max_depth,
        );
        return local_color.lerp(reflection_color, reflectivity);
    }

    local_color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aabb(p0: &[f32; 3], p1: &[f32; 3], p2: &[f32; 3]) -> ([f32; 3], [f32; 3]) {
        (
            [
                p0[0].min(p1[0]).min(p2[0]),
                p0[1].min(p1[1]).min(p2[1]),
                p0[2].min(p1[2]).min(p2[2]),
            ],
            [
                p0[0].max(p1[0]).max(p2[0]),
                p0[1].max(p1[1]).max(p2[1]),
                p0[2].max(p1[2]).max(p2[2]),
            ],
        )
    }

    fn simple_triangle(color: [f32; 3]) -> Triangle {
        let p0 = [0.0, 0.0, 0.0];
        let p1 = [1.0, 0.0, 0.0];
        let p2 = [0.0, 1.0, 0.0];
        let (aabb_min, aabb_max) = aabb(&p0, &p1, &p2);
        Triangle {
            p0,
            p1,
            p2,
            n0: [0.0, 0.0, 1.0],
            n1: [0.0, 0.0, 1.0],
            n2: [0.0, 0.0, 1.0],
            color,
            aabb_min,
            aabb_max,
            reflectivity: 0.15,
            _padding1: 0.0,
            _padding2: 0.0,
            _padding3: 0.0,
        }
    }

    #[test]
    fn ray_misses_returns_background() {
        let tris = vec![simple_triangle([1.0, 0.0, 0.0])];
        let light = LightUniforms::default();
        let color = trace_ray(
            Vec3::new(0.5, 0.5, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
            &tris,
            &light,
            0,
            2,
        );
        assert_eq!(color, Vec3::new(0.2, 0.0, 0.2));
    }

    #[test]
    fn ray_hits_returns_red() {
        let tris = vec![simple_triangle([1.0, 0.0, 0.0])];
        let light = LightUniforms::default();
        let color = trace_ray(
            Vec3::new(0.5, 0.25, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
            &tris,
            &light,
            0,
            2,
        );
        assert!(color.x > 0.0);
        assert!(color.y < 0.1);
        assert!(color.z < 0.1);
    }

    #[test]
    fn max_depth_returns_black() {
        let tris = vec![simple_triangle([1.0, 1.0, 1.0])];
        let light = LightUniforms::default();
        let color = trace_ray(
            Vec3::new(0.5, 0.25, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
            &tris,
            &light,
            2,
            2,
        );
        assert_eq!(color, Vec3::ZERO);
    }

    #[test]
    fn triangle_layout_is_124_bytes() {
        assert_eq!(std::mem::size_of::<Triangle>(), 124);
    }
}
