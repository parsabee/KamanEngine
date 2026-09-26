// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! GPU-facing vertex and uniform layouts for the raster backend.
//!
//! These `#[repr(C)]` structs mirror the layouts declared in
//! `shaders/rasterization.metal`. Their byte layout is load-bearing: it must
//! match the Metal side exactly, so the sizes — and, for [`LightUniforms`], the
//! offset of every alignment-sensitive field — are asserted by tests.

use kaman_math::glam::{Mat4, Vec3};
use kaman_render_api::SunSky;

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

/// The frame's shared fragment uniforms: the directional sun + Phong parameters,
/// the KE-0401 look parameters (gradient sky, distance fog, blob shadow), and the
/// KE-0406 camera block. One of these is uploaded per frame and bound at fragment
/// `[[buffer(0)]]` for **every** pass that shades (sky, untextured, textured).
///
/// Matches `Light` in `rasterization.metal`. Laid out to Metal's `constant`
/// buffer alignment rules — every `float3` is 16-byte aligned/sized — for a
/// total of **208 bytes** (asserted by `light_uniforms_is_208_bytes`, with the
/// offset of every `float3`/`float4x4` field asserted alongside it). The explicit
/// `_pad*` fields reproduce the padding the MSL compiler inserts, so the Rust
/// bytes land on the exact offsets the shader reads.
///
/// # Layout contract
///
/// A Rust↔MSL padding mismatch here once silently disabled the entire look stack
/// and was only noticed when a pixel hash was re-blessed (see the re-bless note on
/// `REFERENCE_HASH` in `tests/pixel_hash.rs`). The failure is silent because the
/// shader still reads *something* at every offset. Two rules keep it from
/// recurring:
///
/// 1. Every field's byte offset is commented below **and** asserted by
///    `light_uniforms_field_offsets` for the alignment-sensitive ones, so a
///    reordering that shifts a `float3` fails a test instead of changing the look.
/// 2. The struct is written into a 256-byte uniform-ring slot
///    ([`UNIFORM_RING_STRIDE`]), so it may keep growing — but the size assertion
///    must be updated deliberately, never widened to a range.
///
/// # Camera block (KE-0406)
///
/// The last two fields are the frame's *camera*, not its light. They live in this
/// struct because it is the one fragment buffer bound to every shading pass, and
/// both consumers are lighting: `lit_linear` needs
/// [`camera_position`](Self::camera_position) for view-dependent specular, and the
/// sky pass needs [`inverse_view_projection`](Self::inverse_view_projection) to
/// turn a fullscreen-pass NDC into a world-space view ray so it can find the sun.
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
    /// World `Y` at or below which the distance fog is at **full** strength.
    /// Above it the fog thins out over [`fog_falloff`](Self::fog_falloff), so the
    /// fog hugs the ground/horizon (hiding the streaming spawn edge) without
    /// washing out tall geometry like the skyline backdrop. Offset 88.
    pub fog_height: f32,
    /// How fast the fog thins above [`fog_height`](Self::fog_height), in world
    /// units (an e-fold). `0` disables the height falloff (fog is uniform with
    /// height, the pre-KE-0706 behaviour). Offset 92.
    pub fog_falloff: f32,
    /// World-space point the car sits above (blob-shadow center). Offset 96.
    pub shadow_center: [f32; 3],
    /// Padding: `shadow_center` is an MSL `float3` (16 bytes), so `shadow_radius`
    /// must land at offset 112 to match the shader.
    pub _padding_shadow: f32,
    /// Blob shadow radius in world units. Offset 112.
    pub shadow_radius: f32,
    /// 0..1 darkening under the car. Offset 116.
    pub shadow_strength: f32,
    /// Alignment padding so `camera_position` (a float3) lands 16-byte aligned at
    /// 128. Was `ground_height`, deleted by KE-0406: it was declared on both sides
    /// and read by no shader. KE-0407's shadow-map pass builds a real receiver.
    pub _padding5: f32,
    /// Alignment padding (pairs with [`_padding5`](Self::_padding5)).
    pub _padding6: f32,

    /// Camera position in world space (KE-0406), so specular is view-dependent
    /// instead of using a constant view vector. Offset 128.
    pub camera_position: [f32; 3],
    /// Padding: `camera_position` is an MSL `float3` (16 bytes), so the matrix
    /// below must land at offset 144.
    pub _padding_camera: f32,
    /// Inverse of the frame's view-projection, column-major (KE-0406). The sky is a
    /// fullscreen pass with no geometry, so it unprojects its NDC through this to
    /// recover the world-space view ray it needs to find the sun. Offset 144.
    pub inverse_view_projection: [[f32; 4]; 4],
}

impl LightUniforms {
    /// Overwrite the sun + sky fields from the engine-generic seam description
    /// (KE-0406), leaving everything else (specular response, fog, blob shadow,
    /// camera) untouched.
    ///
    /// This is the **only** place a [`SunSky`] becomes GPU bytes, and the light
    /// direction is taken from [`SunSky::direction`] rather than rebuilt here — so
    /// the vector the geometry shades with and the vector the sky draws its sun disc
    /// at are the same number, by construction.
    pub fn apply_sun_sky(&mut self, sun: &SunSky) {
        self.direction = sun.direction().to_array();
        self.color = sun.sun_color;
        // The seam's "sky fill" *is* the Phong ambient term, and its "sun intensity"
        // the diffuse one; the mapping lives here so the seam can keep talking about
        // a sky instead of a lighting model.
        self.ambient_intensity = sun.sky_fill;
        self.diffuse_intensity = sun.sun_intensity;
        self.sky_top_color = sun.sky_zenith_color;
        self.sky_horizon_color = sun.sky_horizon_color;
    }

    /// Fold the frame's camera into the uniforms (KE-0406): its world position (for
    /// view-dependent specular) and the inverse of its view-projection (for the sky
    /// pass's view ray).
    ///
    /// Takes the matrix the caller already pushed through the seam rather than
    /// deriving the position from it — the position arrives across the seam too, so
    /// no inverse is trusted with it.
    pub fn apply_camera(&mut self, position: Vec3, view_projection: Mat4) {
        self.camera_position = position.to_array();
        self.inverse_view_projection = view_projection.inverse().to_cols_array_2d();
    }
}

impl Default for LightUniforms {
    fn default() -> Self {
        // The sun/sky half of the defaults comes from the seam's own
        // `SunSky::default()` so there is exactly one default sun in the engine; the
        // rest (specular response, fog, blob shadow) are backend look defaults with
        // no seam representation yet.
        let sun = SunSky::default();
        Self {
            // Derived from the angles, never hand-written: see `apply_sun_sky`.
            direction: sun.direction().to_array(),
            _padding1: 0.0,
            color: sun.sun_color,
            _padding2: 0.0,
            ambient_intensity: sun.sky_fill,
            diffuse_intensity: sun.sun_intensity,
            specular_intensity: 0.5,
            shininess: 32.0,
            // Look defaults: a calm blue gradient sky, a soft blob shadow centered at
            // the origin, and **deep horizon fog**: the scene stays completely clear
            // out to `fog_start`, then the density ramps hard so everything near the
            // streaming spawn edge is fully blended into the horizon — which hides
            // content popping in at the spawn distance while leaving the mid-ground
            // (and the skyline backdrop) crisp.
            sky_top_color: sun.sky_zenith_color,
            _padding3: 0.0,
            sky_horizon_color: sun.sky_horizon_color,
            _padding_horizon: 0.0,
            fog_density: 0.10,
            fog_start: 35.0,
            // Fog hugs the ground: full strength at/below the roadway, thinning
            // upward so the skyline backdrop and tall buildings stay readable.
            fog_height: 2.0,
            fog_falloff: 5.0,
            shadow_center: [0.0, 0.0, 0.0],
            _padding_shadow: 0.0,
            shadow_radius: 1.2,
            shadow_strength: 0.5,
            _padding5: 0.0,
            _padding6: 0.0,
            // No camera pushed yet: the origin and an identity view-projection (whose
            // inverse is the identity), matching the backend's own default camera.
            camera_position: [0.0, 0.0, 0.0],
            _padding_camera: 0.0,
            inverse_view_projection: Mat4::IDENTITY.to_cols_array_2d(),
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
    fn light_uniforms_is_208_bytes() {
        // Phong params (48 bytes) + KE-0401 sky/fog/shadow params (80) + the
        // KE-0406 camera block (a float3 + a float4x4, 80), padded to Metal's
        // constant-buffer alignment (every float3 is 16-byte aligned).
        //
        // Grew from 128 in KE-0406, which added the camera block and deleted the
        // dead `ground_height`. Update this deliberately and in lockstep with the
        // MSL `Light` struct — never relax it to a bound.
        assert_eq!(size_of::<LightUniforms>(), 208);
    }

    #[test]
    fn light_uniforms_field_offsets_match_the_shader() {
        // The exact failure this file has already suffered: a padding mismatch that
        // shifted a `float3` and silently disabled the look stack, because the
        // shader happily reads whatever bytes sit at each offset. An MSL `float3`
        // occupies 16 bytes, so every one of these must be 16-byte aligned and land
        // where `Light` in `rasterization.metal` says it does.
        use std::mem::offset_of;
        assert_eq!(offset_of!(LightUniforms, direction), 0);
        assert_eq!(offset_of!(LightUniforms, color), 16);
        assert_eq!(offset_of!(LightUniforms, ambient_intensity), 32);
        assert_eq!(offset_of!(LightUniforms, diffuse_intensity), 36);
        assert_eq!(offset_of!(LightUniforms, specular_intensity), 40);
        assert_eq!(offset_of!(LightUniforms, shininess), 44);
        assert_eq!(offset_of!(LightUniforms, sky_top_color), 48);
        assert_eq!(offset_of!(LightUniforms, sky_horizon_color), 64);
        assert_eq!(offset_of!(LightUniforms, fog_density), 80);
        assert_eq!(offset_of!(LightUniforms, fog_start), 84);
        assert_eq!(offset_of!(LightUniforms, fog_height), 88);
        assert_eq!(offset_of!(LightUniforms, fog_falloff), 92);
        assert_eq!(offset_of!(LightUniforms, shadow_center), 96);
        assert_eq!(offset_of!(LightUniforms, shadow_radius), 112);
        assert_eq!(offset_of!(LightUniforms, shadow_strength), 116);
        assert_eq!(offset_of!(LightUniforms, camera_position), 128);
        assert_eq!(offset_of!(LightUniforms, inverse_view_projection), 144);

        // Every float3 / float4x4 must be 16-byte aligned or MSL reads it shifted.
        for offset in [
            offset_of!(LightUniforms, direction),
            offset_of!(LightUniforms, color),
            offset_of!(LightUniforms, sky_top_color),
            offset_of!(LightUniforms, sky_horizon_color),
            offset_of!(LightUniforms, shadow_center),
            offset_of!(LightUniforms, camera_position),
            offset_of!(LightUniforms, inverse_view_projection),
        ] {
            assert_eq!(offset % 16, 0, "offset {offset} is not 16-byte aligned");
        }
    }

    #[test]
    fn light_uniforms_fits_one_uniform_ring_slot() {
        // KE-0406 uploads the light once per frame into a ring slot, so it must fit
        // inside the 256-byte stride (which also keeps the fragment-buffer offset
        // 256-byte aligned, the Apple GPU requirement).
        assert!(size_of::<LightUniforms>() as u64 <= UNIFORM_RING_STRIDE);
    }

    #[test]
    fn material_uniforms_is_16_bytes() {
        // Must match `MaterialUniforms` (a single float4) in rasterization.metal.
        assert_eq!(size_of::<MaterialUniforms>(), 16);
    }

    #[test]
    fn light_uniforms_default_matches_reference() {
        let l = LightUniforms::default();
        assert_eq!(l.color, [1.0, 1.0, 1.0]);
        // KE-0406: the fill is now sky fill and the sun dominates (was 0.6 vs 0.8,
        // which left unlit faces at 43% of lit and read overcast).
        assert_eq!(l.ambient_intensity, 0.2);
        assert_eq!(l.diffuse_intensity, 0.8);
        assert!(l.ambient_intensity < l.diffuse_intensity * 0.5);
        assert_eq!(l.specular_intensity, 0.5);
        assert_eq!(l.shininess, 32.0);
    }

    #[test]
    fn light_uniforms_default_sun_comes_from_the_seam() {
        // One default sun in the engine: the backend's default light direction is
        // whatever `SunSky::default()` derives, never a second hardcoded vector.
        let l = LightUniforms::default();
        let sun = SunSky::default();
        assert_eq!(l.direction, sun.direction().to_array());
        assert_eq!(l.sky_top_color, sun.sky_zenith_color);
        assert_eq!(l.sky_horizon_color, sun.sky_horizon_color);
    }

    #[test]
    fn apply_sun_sky_takes_its_direction_from_the_seam_type() {
        // The single-source-of-truth property the sun disc depends on: what the
        // shader shades with is exactly `SunSky::direction()`, so the disc drawn at
        // that direction and the lighting can never disagree.
        let sun = SunSky {
            sun_elevation_deg: 32.0,
            sun_azimuth_deg: 284.0,
            sun_color: [1.0, 0.96, 0.88],
            sun_intensity: 1.15,
            sky_fill: 0.22,
            sky_zenith_color: [0.12, 0.28, 0.55],
            sky_horizon_color: [0.62, 0.68, 0.76],
        };
        let mut l = LightUniforms::default();
        l.apply_sun_sky(&sun);
        assert_eq!(l.direction, sun.direction().to_array());
        assert_eq!(l.color, sun.sun_color);
        assert_eq!(l.ambient_intensity, sun.sky_fill);
        assert_eq!(l.diffuse_intensity, sun.sun_intensity);
        assert_eq!(l.sky_top_color, sun.sky_zenith_color);
        assert_eq!(l.sky_horizon_color, sun.sky_horizon_color);
        // Untouched by the sun/sky: the fog + shadow look params keep their values.
        let base = LightUniforms::default();
        assert_eq!(l.fog_density, base.fog_density);
        assert_eq!(l.shadow_radius, base.shadow_radius);
        assert_eq!(l.shininess, base.shininess);
    }

    #[test]
    fn apply_camera_stores_the_position_and_the_inverted_matrix() {
        let mut l = LightUniforms::default();
        let eye = Vec3::new(3.0, 4.0, -5.0);
        let vp = Mat4::perspective_rh(1.0, 1.6, 0.1, 100.0)
            * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        l.apply_camera(eye, vp);

        assert_eq!(l.camera_position, eye.to_array());
        // The stored matrix must actually invert the pushed one: round-tripping a
        // clip-space point through both returns it. A transposed or mis-ordered
        // store would break the sky's view ray silently.
        let inv = Mat4::from_cols_array_2d(&l.inverse_view_projection);
        let round_trip = vp * inv;
        for (a, b) in round_trip
            .to_cols_array()
            .iter()
            .zip(Mat4::IDENTITY.to_cols_array().iter())
        {
            assert!((a - b).abs() < 1e-4, "{round_trip:?} is not vp⁻¹");
        }
    }

    #[test]
    fn identity_view_projection_inverts_to_identity() {
        // The backend's pre-first-camera state, and the state the screen-space
        // overlay reference renders in: the sky's unprojection must degenerate to a
        // constant forward ray rather than producing NaNs.
        let l = LightUniforms::default();
        assert_eq!(
            l.inverse_view_projection,
            Mat4::IDENTITY.to_cols_array_2d()
        );
    }
}
