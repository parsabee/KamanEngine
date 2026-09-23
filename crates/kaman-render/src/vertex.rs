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
///
/// # Layout contract
///
/// This `#[repr(C)]` struct is written verbatim into a GPU `MTLBuffer` and read
/// back by `rasterization.metal`'s `Uniforms` at `[[buffer(1)]]`. The two sides
/// must stay in lockstep:
///
/// - Rust `model_view_projection: [[f32; 4]; 4]` (column-major) ⇔ MSL
///   `float4x4 modelViewProjection`.
/// - Size is **exactly 64 bytes** (asserted by [`uniforms_is_64_bytes`]).
/// - When sub-allocated from the uniform ring the struct is written at a
///   [`UNIFORM_RING_STRIDE`]-byte-aligned offset so it satisfies the Apple GPU
///   256-byte `set_vertex_buffer` offset requirement; the struct itself only
///   needs its natural 16-byte alignment, the stride padding lives in the ring.
///
/// [`uniforms_is_64_bytes`]: tests::uniforms_is_64_bytes
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Uniforms {
    /// Combined model-view-projection matrix, column-major.
    pub model_view_projection: [[f32; 4]; 4],
}

/// Per-slot byte stride of the uniform ring.
///
/// Apple GPUs require the `offset` passed to `setVertexBuffer:offset:atIndex:`
/// to be a multiple of 256 bytes. A [`Uniforms`] is only 64 bytes, so each ring
/// slot is padded up to this stride and every per-draw offset is a multiple of
/// it — see [`crate::backend`]'s uniform-ring docs. Kept next to [`Uniforms`] so
/// the layout contract and its GPU alignment requirement live together.
pub const UNIFORM_RING_STRIDE: u64 = 256;

/// Per-draw material parameters for the **textured** pipeline (KE-0403).
///
/// Matches `MaterialUniforms` in `rasterization.metal` — a single `float4`
/// base-color factor, **16 bytes**. Written into the uniform ring alongside the
/// per-draw MVP and bound at fragment `[[buffer(2)]]` so the textured fragment
/// shader multiplies the sampled base-color texture by the material factor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MaterialUniforms {
    /// Linear RGBA base-color factor, multiplied with the sampled texture.
    pub base_color_factor: [f32; 4],
}

impl Default for MaterialUniforms {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
        }
    }
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
    fn uniform_ring_stride_is_256_aligned_and_holds_uniforms() {
        // Apple GPU MTLBuffer offset requirement: every ring slot offset is a
        // multiple of 256, so the stride itself must be 256-aligned and large
        // enough to hold a `Uniforms`.
        assert_eq!(UNIFORM_RING_STRIDE % 256, 0);
        assert!(UNIFORM_RING_STRIDE >= size_of::<Uniforms>() as u64);
    }

    #[test]
    fn light_uniforms_is_48_bytes() {
        assert_eq!(size_of::<LightUniforms>(), 48);
    }

    #[test]
    fn material_uniforms_is_16_bytes() {
        // Must match `MaterialUniforms` (a single float4) in rasterization.metal.
        assert_eq!(size_of::<MaterialUniforms>(), 16);
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
