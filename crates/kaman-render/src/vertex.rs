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

/// Per-draw transform uniform: the model-view-projection **and** model matrices.
///
/// Matches `Uniforms` in `rasterization.metal` — two `float4x4`s, 128 bytes.
///
/// # Layout contract
///
/// This `#[repr(C)]` struct is written verbatim into a GPU `MTLBuffer` and read
/// back by `rasterization.metal`'s `Uniforms` at `[[buffer(1)]]`. The two sides
/// must stay in lockstep:
///
/// - Rust `model_view_projection: [[f32; 4]; 4]` (column-major) ⇔ MSL
///   `float4x4 modelViewProjection`.
/// - Rust `model: [[f32; 4]; 4]` ⇔ MSL `float4x4 model`. The **model matrix**
///   is needed by the KE-0401 look stack: fog and the blob shadow are computed
///   in world space, so the vertex shader must reconstruct the world position
///   and normal from the un-projected model transform.
/// - Size is **exactly 128 bytes** (asserted by `uniforms_is_128_bytes`).
/// - When sub-allocated from the uniform ring the struct is written at a
///   [`UNIFORM_RING_STRIDE`]-byte-aligned offset so it satisfies the Apple GPU
///   256-byte `set_vertex_buffer` offset requirement; the struct itself only
///   needs its natural 16-byte alignment, the stride padding lives in the ring.
///
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Uniforms {
    /// Combined model-view-projection matrix, column-major.
    pub model_view_projection: [[f32; 4]; 4],
    /// Model (world) matrix, column-major — drives world-space fog + shadow.
    pub model: [[f32; 4]; 4],
}

/// Per-slot byte stride of the uniform ring.
///
/// Apple GPUs require the `offset` passed to `setVertexBuffer:offset:atIndex:`
/// to be a multiple of 256 bytes. A [`Uniforms`] is only 128 bytes, so each ring
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

/// Directional light + Phong parameters, plus the KE-0401 look parameters
/// (gradient sky, distance fog, blob shadow), shared across all draws in a frame.
///
/// Matches `Light` in `rasterization.metal`. Laid out to Metal's `constant`
/// buffer alignment rules — every `float3` is 16-byte aligned/sized — for a
/// total of **128 bytes** (asserted by `light_uniforms_is_128_bytes`). The
/// explicit `_pad*` fields reproduce the padding the MSL compiler inserts, so
/// the Rust bytes land on the exact offsets the shader reads.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LightUniforms {
    /// Light direction in world space (points *from* the light). Offset 0.
    pub direction: [f32; 3],
    /// Alignment padding.
    pub _padding1: f32,
    /// Light RGB color. Offset 16.
    pub color: [f32; 3],
    /// Alignment padding.
    pub _padding2: f32,
    /// Constant ambient strength. Offset 32.
    pub ambient_intensity: f32,
    /// Diffuse (Lambertian) strength.
    pub diffuse_intensity: f32,
    /// Specular highlight strength.
    pub specular_intensity: f32,
    /// Phong specular exponent.
    pub shininess: f32,

    /// Linear zenith color of the gradient sky. Offset 48.
    pub sky_top_color: [f32; 3],
    /// Alignment padding.
    pub _padding3: f32,
    /// Linear horizon color; distance fog blends toward this. Offset 64.
    pub sky_horizon_color: [f32; 3],
    /// Padding: an MSL `float3` occupies 16 bytes, so `fog_density` must land at
    /// offset 80 (not 76) to match the shader's `Light` struct.
    pub _padding_horizon: f32,
    /// Exponential fog density (0 disables fog). Offset 80.
    pub fog_density: f32,
    /// View distance at which fog begins. Offset 84.
    pub fog_start: f32,
    /// Alignment padding so `shadow_center` (a float3) lands 16-byte aligned at 96.
    pub _padding4: [f32; 2],
    /// World-space point the car sits above (blob-shadow center). Offset 96.
    pub shadow_center: [f32; 3],
    /// Padding: `shadow_center` is an MSL `float3` (16 bytes), so `shadow_radius`
    /// must land at offset 112 to match the shader.
    pub _padding_shadow: f32,
    /// Blob shadow radius in world units. Offset 112.
    pub shadow_radius: f32,
    /// 0..1 darkening under the car. Offset 116.
    pub shadow_strength: f32,
    /// World Y of the ground plane (shadow receiver). Offset 120.
    pub ground_height: f32,
    /// Alignment padding to a 16-byte multiple (total 128 bytes).
    pub _padding5: f32,
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
            // Look defaults: a calm blue gradient sky, very light distance fog, and
            // a soft blob shadow centered at the origin on a ground plane at y=-0.5.
            sky_top_color: [0.09, 0.22, 0.44],
            _padding3: 0.0,
            sky_horizon_color: [0.55, 0.62, 0.72],
            _padding_horizon: 0.0,
            fog_density: 0.012,
            fog_start: 5.0,
            _padding4: [0.0, 0.0],
            shadow_center: [0.0, 0.0, 0.0],
            _padding_shadow: 0.0,
            shadow_radius: 1.2,
            shadow_strength: 0.5,
            ground_height: -0.5,
            _padding5: 0.0,
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
    fn uniforms_is_128_bytes() {
        // Two float4x4s (MVP + model). The model matrix drives world-space fog +
        // blob shadow in the KE-0401 look stack.
        assert_eq!(size_of::<Uniforms>(), 128);
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
    fn light_uniforms_is_128_bytes() {
        // Phong params (48 bytes) + KE-0401 sky/fog/shadow params, padded to
        // Metal's constant-buffer alignment (every float3 is 16-byte aligned).
        assert_eq!(size_of::<LightUniforms>(), 128);
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
