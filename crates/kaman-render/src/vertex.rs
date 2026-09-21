// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! GPU-facing vertex and uniform layouts for the raster backend.
//!
//! These `#[repr(C)]` structs mirror the layouts declared in
//! `shaders/rasterization.metal`. Their byte layout is load-bearing: it must
//! match the Metal side exactly, so the sizes are asserted by tests.

/// One interleaved vertex: position, normal, and per-vertex color.
///
/// Matches `VertexIn` in `rasterization.metal` — three `float3`s, 36 bytes,
/// with attributes at offsets 0 / 12 / 24.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// Model-space position.
    pub position: [f32; 3],
    /// Surface normal.
    pub normal: [f32; 3],
    /// Per-vertex RGB color in `0.0..=1.0`.
    pub color: [f32; 3],
}

/// Per-draw transform uniform: the model-view-projection matrix.
///
/// Matches `Uniforms` in `rasterization.metal` — a single `float4x4`, 64 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Uniforms {
    /// Combined model-view-projection matrix, column-major.
    pub model_view_projection: [[f32; 4]; 4],
}

/// Directional light + Phong parameters shared across all draws in a frame.
///
/// Matches `Light` in `rasterization.metal`, padded to 48 bytes for Metal's
/// alignment rules.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LightUniforms {
    /// Light direction in world space (points *from* the light).
    pub direction: [f32; 3],
    /// Alignment padding.
    pub _padding1: f32,
    /// Light RGB color.
    pub color: [f32; 3],
    /// Alignment padding.
    pub _padding2: f32,
    /// Constant ambient strength.
    pub ambient_intensity: f32,
    /// Diffuse (Lambertian) strength.
    pub diffuse_intensity: f32,
    /// Specular highlight strength.
    pub specular_intensity: f32,
    /// Phong specular exponent.
    pub shininess: f32,
}

impl Default for LightUniforms {
    fn default() -> Self {
        Self {
            direction: [-0.5, -1.0, -0.3],
            _padding1: 0.0,
            color: [1.0, 1.0, 1.0],
            _padding2: 0.0,
            ambient_intensity: 0.6,
            diffuse_intensity: 0.8,
            specular_intensity: 0.5,
            shininess: 32.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn vertex_is_36_bytes() {
        assert_eq!(size_of::<Vertex>(), 36);
    }

    #[test]
    fn uniforms_is_64_bytes() {
        assert_eq!(size_of::<Uniforms>(), 64);
    }

    #[test]
    fn light_uniforms_is_48_bytes() {
        assert_eq!(size_of::<LightUniforms>(), 48);
    }

    #[test]
    fn light_uniforms_default_matches_reference() {
        let l = LightUniforms::default();
        assert_eq!(l.direction, [-0.5, -1.0, -0.3]);
        assert_eq!(l.color, [1.0, 1.0, 1.0]);
        assert_eq!(l.ambient_intensity, 0.6);
        assert_eq!(l.diffuse_intensity, 0.8);
        assert_eq!(l.specular_intensity, 0.5);
        assert_eq!(l.shininess, 32.0);
    }
}
