// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The C-visible types and constants: everything `kaman.h` declares besides the
//! functions.
//!
//! Every enum-like type is a fixed-width integer alias with constants rather
//! than a Rust `enum`, so any value a C caller passes is defined behaviour; an
//! unknown value is rejected with [`KAMAN_ERROR_INVALID_ARGUMENT`].

use std::ffi::{c_char, c_void};

// ---------------------------------------------------------------------------
// Versioning
// ---------------------------------------------------------------------------

/// The API version this header describes. It increases with every incompatible
/// change; set [`KamanContextDesc::api_version`] to it.
pub const KAMAN_API_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// A result code. `KAMAN_OK` (0) is success, small positive values are non-error
/// outcomes, negative values are errors.
pub type KamanResult = i32;

/// Done.
pub const KAMAN_OK: KamanResult = 0;
/// No frame this time: too many frames in flight, no drawable, or the context
/// is not ready. Not an error.
pub const KAMAN_FRAME_SKIPPED: KamanResult = 1;
/// A resource the call needs is pending or failed; nothing was done.
pub const KAMAN_NOT_READY: KamanResult = 2;
/// NULL where a pointer is required, a bad enum value, or sizes that don't match.
pub const KAMAN_ERROR_INVALID_ARGUMENT: KamanResult = -1;
/// The handle is null, destroyed, of the wrong kind, or another context's.
pub const KAMAN_ERROR_INVALID_HANDLE: KamanResult = -2;
/// The call is not allowed now (no frame open, frame already open, ...).
pub const KAMAN_ERROR_WRONG_STATE: KamanResult = -3;
/// Another call on this context is in progress.
pub const KAMAN_ERROR_BUSY: KamanResult = -4;
/// No Metal device, or Metal is unavailable on this platform.
pub const KAMAN_ERROR_NO_DEVICE: KamanResult = -5;
/// Not supported by this context or build.
pub const KAMAN_ERROR_UNSUPPORTED: KamanResult = -6;
/// A file could not be read.
pub const KAMAN_ERROR_IO: KamanResult = -7;
/// A glTF or image could not be decoded.
pub const KAMAN_ERROR_PARSE: KamanResult = -8;
/// An engine limit was hit (handle table, texture size, buffer length).
pub const KAMAN_ERROR_LIMIT_EXCEEDED: KamanResult = -9;
/// A bug in the engine (a Rust panic caught at the boundary). Destroy the context.
pub const KAMAN_ERROR_INTERNAL: KamanResult = -10;

// ---------------------------------------------------------------------------
// Enum-like values
// ---------------------------------------------------------------------------

/// Which renderer a context uses.
pub type KamanBackend = u32;
/// The Metal renderer (Apple platforms).
pub const KAMAN_BACKEND_METAL: KamanBackend = 0;
/// No GPU: resources and frames are tracked, draws counted, nothing drawn, and
/// read-backs return zeros. For host-side tests.
pub const KAMAN_BACKEND_NULL: KamanBackend = 1;

/// The state of a resource handle.
pub type KamanResourceState = u32;
/// Not a live handle: never issued, destroyed, or another context's.
pub const KAMAN_RESOURCE_STATE_INVALID: KamanResourceState = 0;
/// Accepted; the work to make it usable has not finished.
pub const KAMAN_RESOURCE_STATE_PENDING: KamanResourceState = 1;
/// Usable in draws.
pub const KAMAN_RESOURCE_STATE_READY: KamanResourceState = 2;
/// The work failed; draws using it are skipped. Destroy it.
pub const KAMAN_RESOURCE_STATE_FAILED: KamanResourceState = 3;

/// What a [`KamanEvent`] reports.
pub type KamanEventType = u32;
/// The context can render.
pub const KAMAN_EVENT_CONTEXT_READY: KamanEventType = 1;
/// A resource (`object`) became ready.
pub const KAMAN_EVENT_RESOURCE_READY: KamanEventType = 2;
/// A resource (`object`) failed; `result` and `message` say why.
pub const KAMAN_EVENT_RESOURCE_FAILED: KamanEventType = 3;
/// The GPU finished frame `frame`.
pub const KAMAN_EVENT_FRAME_COMPLETED: KamanEventType = 4;
/// The pixels requested for frame `frame` are in the caller's buffer.
pub const KAMAN_EVENT_READBACK_COMPLETE: KamanEventType = 5;
/// The pixels requested for frame `frame` will never arrive.
pub const KAMAN_EVENT_READBACK_FAILED: KamanEventType = 6;

/// The vertex layout of a mesh, which also selects the pipeline that draws it.
pub type KamanVertexLayout = u32;
/// `float3 position, float3 normal, float3 linear RGB colour` (36 bytes).
pub const KAMAN_VERTEX_LAYOUT_POS_NORMAL_COLOR: KamanVertexLayout = 0;
/// `float3 position, float3 normal, float2 uv` (32 bytes), drawn with a
/// base-colour texture.
pub const KAMAN_VERTEX_LAYOUT_POS_NORMAL_UV: KamanVertexLayout = 1;

/// Shadow quality.
pub type KamanShadowQuality = u32;
/// No shadow pass.
pub const KAMAN_SHADOW_QUALITY_OFF: KamanShadowQuality = 0;
/// A 2048² shadow map.
pub const KAMAN_SHADOW_QUALITY_LOW: KamanShadowQuality = 1;
/// A 4096² shadow map (the default).
pub const KAMAN_SHADOW_QUALITY_HIGH: KamanShadowQuality = 2;

/// How a 2D quad is filled.
pub type KamanFill = u32;
/// The quad's colour, untextured.
pub const KAMAN_FILL_SOLID: KamanFill = 0;
/// `texture` sampled as RGBA, times the colour.
pub const KAMAN_FILL_TEXTURE: KamanFill = 1;
/// `texture` sampled as a signed-distance field (distance in red), tinted by
/// the colour. How text is drawn.
pub const KAMAN_FILL_SDF: KamanFill = 2;

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

/// A 3-component vector.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanVec3 {
    /// X.
    pub x: f32,
    /// Y (up).
    pub y: f32,
    /// Z.
    pub z: f32,
}

/// A rotation quaternion.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanQuat {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Z.
    pub z: f32,
    /// W (1 for the identity).
    pub w: f32,
}

/// A 4×4 matrix, **column-major**: `m[0..4]` is the first column. The same
/// memory layout as Swift's `simd_float4x4`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanMat4 {
    /// The 16 elements, column by column.
    pub m: [f32; 16],
}

/// A linear RGBA colour, each channel `0..1`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanColor {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha.
    pub a: f32,
}

// ---------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------

/// A mesh owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KamanMesh {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// A texture owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KamanTexture {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// A glTF model owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KamanModel {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// An SDF font owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KamanFont {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

// ---------------------------------------------------------------------------
// Data ownership, events, completions
// ---------------------------------------------------------------------------

/// Called exactly once when the engine no longer needs borrowed [`KamanData`].
/// NULL means the data is copied.
pub type KamanReleaseFn =
    Option<unsafe extern "C" fn(user: *mut c_void, ptr: *const c_void, len: usize)>;

/// Bulk data passed to the engine: copied (`release` NULL) or borrowed until
/// `release` is called.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanData {
    /// The bytes. May be NULL only when `len` is 0.
    pub ptr: *const c_void,
    /// Length in bytes.
    pub len: usize,
    /// NULL: the engine copies the bytes before the call returns. Otherwise the
    /// bytes are borrowed until the engine calls `release(user, ptr, len)`.
    pub release: KamanReleaseFn,
    /// Passed to `release`.
    pub user: *mut c_void,
}

/// A completion or status report. See `docs/C_API.md`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanEvent {
    /// What happened: a `KAMAN_EVENT_*` value.
    pub kind: KamanEventType,
    /// `KAMAN_OK`, or why it failed.
    pub result: KamanResult,
    /// The handle id the event is about, or 0.
    pub object: u64,
    /// The frame number the event is about, or 0.
    pub frame: u64,
    /// Detail for failures (NUL-terminated UTF-8), or NULL. Valid until the
    /// callback returns, or until the next `kaman_poll_events` on the context.
    pub message: *const c_char,
}

/// Receives one event, on a host thread inside a `kaman_*` call. May be NULL.
pub type KamanCompletionFn =
    Option<unsafe extern "C" fn(user: *mut c_void, event: *const KamanEvent)>;

/// A per-call completion callback: the call's event goes to `callback` instead
/// of the poll queue.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanCompletion {
    /// Receives the event. NULL behaves like passing no completion.
    pub callback: KamanCompletionFn,
    /// Passed to `callback`.
    pub user: *mut c_void,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// How to create a context.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanContextDesc {
    /// Must be `KAMAN_API_VERSION`.
    pub api_version: u32,
    /// `KAMAN_BACKEND_METAL` or `KAMAN_BACKEND_NULL`.
    pub backend: KamanBackend,
    /// A `CAMetalLayer *` to render into, or NULL for an offscreen context.
    /// Must be NULL with `KAMAN_BACKEND_NULL`.
    pub metal_layer: *mut c_void,
    /// Drawable width in pixels (offscreen: the fixed size), `1..=16384`.
    pub width: u32,
    /// Drawable height in pixels, `1..=16384`.
    pub height: u32,
    /// Drawable pixels per point; `<= 0` means 1.
    pub scale: f32,
}

/// Render quality settings.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KamanRenderSettings {
    /// A `KAMAN_SHADOW_QUALITY_*` value.
    pub shadows: KamanShadowQuality,
    /// Shadow range as a fraction `(0, 1]` of the draw distance; `<= 0` matches
    /// the draw distance.
    pub shadow_distance: f32,
    /// Draw-distance scale; `1` is the default. Scales the fog and the shadow
    /// range. Must be finite and positive.
    pub draw_distance_scale: f32,
}

/// Counters for diagnostics and tests.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KamanStats {
    /// Frames handed to the GPU.
    pub frames_submitted: u64,
    /// Frames the GPU has finished.
    pub frames_completed: u64,
    /// `kaman_frame_acquire` or `kaman_frame_submit` calls that returned
    /// `KAMAN_FRAME_SKIPPED`.
    pub frames_skipped: u64,
    /// 3D draws handed to the GPU.
    pub draws_submitted: u64,
    /// Draws not made because a resource was pending, failed or destroyed.
    pub draws_skipped: u64,
    /// Live mesh handles, any state.
    pub meshes: u32,
    /// Live texture handles, any state.
    pub textures: u32,
    /// Live model handles, any state.
    pub models: u32,
    /// Live font handles, any state.
    pub fonts: u32,
    /// Model loads that are neither ready nor failed yet.
    pub loads_pending: u32,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// How to create a mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanMeshDesc {
    /// A `KAMAN_VERTEX_LAYOUT_*` value.
    pub layout: KamanVertexLayout,
    /// Interleaved vertices: a whole number of vertices of `layout`.
    pub vertices: KamanData,
    /// `uint32_t` triangle-list indices: a multiple of 3, each below the vertex
    /// count. Any alignment.
    pub indices: KamanData,
}

/// How to create a texture.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanTextureDesc {
    /// Width in pixels, `1..=16384`.
    pub width: u32,
    /// Height in pixels, `1..=16384`.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8, rows top to bottom.
    pub rgba8: KamanData,
    /// Reserved; must be 0.
    pub flags: u32,
}

/// One glyph of an SDF font, in em units.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanGlyph {
    /// Atlas rectangle, left.
    pub u0: f32,
    /// Atlas rectangle, top.
    pub v0: f32,
    /// Atlas rectangle, right.
    pub u1: f32,
    /// Atlas rectangle, bottom.
    pub v1: f32,
    /// Quad width.
    pub width: f32,
    /// Quad height.
    pub height: f32,
    /// Pen to quad left edge.
    pub bearing_x: f32,
    /// Pen (baseline) to quad top edge; negative rises above the baseline.
    pub bearing_y: f32,
    /// Pen advance after the glyph.
    pub advance: f32,
}

/// How to create an SDF font.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KamanFontDesc {
    /// The SDF atlas texture (distance in the red channel).
    pub atlas: KamanTexture,
    /// The code point `glyphs[0]` describes.
    pub first_codepoint: u32,
    /// `glyph_count` glyphs for consecutive code points. May be NULL when
    /// `glyph_count` is 0.
    pub glyphs: *const KamanGlyph,
    /// The number of glyphs.
    pub glyph_count: u32,
    /// Baseline-to-baseline distance, em units.
    pub line_height: f32,
    /// Used for code points outside the range.
    pub fallback: KamanGlyph,
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// The camera a frame is seen from.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanCamera {
    /// World to clip.
    pub view_projection: KamanMat4,
    /// World-space eye position, for specular highlights.
    pub position: KamanVec3,
}

/// The sun and sky a frame is lit by.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanSunSky {
    /// Sun height above the horizon, degrees: 0 horizon, 90 overhead.
    pub sun_elevation_deg: f32,
    /// Sun bearing, degrees from -Z (north) toward +X (east).
    pub sun_azimuth_deg: f32,
    /// Linear RGB sunlight colour.
    pub sun_color: [f32; 3],
    /// Direct-light strength.
    pub sun_intensity: f32,
    /// Ambient fill; keep it well below `sun_intensity`.
    pub sky_fill: f32,
    /// Linear RGB sky colour straight up.
    pub sky_zenith_color: [f32; 3],
    /// Linear RGB sky colour at the horizon; the fog fades toward it.
    pub sky_horizon_color: [f32; 3],
}

/// A mesh draw's material.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanMaterial {
    /// Multiplies the base-colour texture (`POS_NORMAL_UV` meshes).
    pub base_color: KamanColor,
    /// The base-colour texture for `POS_NORMAL_UV` meshes; `{0}` for white.
    pub base_color_texture: KamanTexture,
}

/// A 2D overlay quad, in drawable pixels with the origin at the top-left.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KamanQuad {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Texture rectangle, left (ignored for `KAMAN_FILL_SOLID`).
    pub u0: f32,
    /// Texture rectangle, top.
    pub v0: f32,
    /// Texture rectangle, right.
    pub u1: f32,
    /// Texture rectangle, bottom.
    pub v1: f32,
    /// Tint; alpha blends.
    pub color: KamanColor,
    /// A `KAMAN_FILL_*` value.
    pub fill: KamanFill,
    /// The texture for `KAMAN_FILL_TEXTURE` and `KAMAN_FILL_SDF`.
    pub texture: KamanTexture,
}
