// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The C-visible types and constants: everything `stickshift.h` declares besides the
//! functions.
//!
//! Every enum-like type is a fixed-width integer alias with constants rather
//! than a Rust `enum`, so any value a C caller passes is defined behaviour; an
//! unknown value is rejected with [`STICKSHIFT_ERROR_INVALID_ARGUMENT`].

use std::ffi::{c_char, c_void};

// ---------------------------------------------------------------------------
// Versioning
// ---------------------------------------------------------------------------

/// The API version this header describes. It increases with every incompatible
/// change; set [`StickShiftContextDesc::api_version`] to it.
pub const STICKSHIFT_API_VERSION: u32 = 3;

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// A result code. `STICKSHIFT_OK` (0) is success, small positive values are non-error
/// outcomes, negative values are errors.
pub type StickShiftResult = i32;

/// Done.
pub const STICKSHIFT_OK: StickShiftResult = 0;
/// No frame this time: too many frames in flight, no drawable, or the context
/// is not ready. Not an error.
pub const STICKSHIFT_FRAME_SKIPPED: StickShiftResult = 1;
/// A resource the call needs is pending or failed; nothing was done.
pub const STICKSHIFT_NOT_READY: StickShiftResult = 2;
/// NULL where a pointer is required, a bad enum value, or sizes that don't match.
pub const STICKSHIFT_ERROR_INVALID_ARGUMENT: StickShiftResult = -1;
/// The handle is null, destroyed, of the wrong kind, or another context's.
pub const STICKSHIFT_ERROR_INVALID_HANDLE: StickShiftResult = -2;
/// The call is not allowed now (no frame open, frame already open, ...).
pub const STICKSHIFT_ERROR_WRONG_STATE: StickShiftResult = -3;
/// Another call on this context is in progress.
pub const STICKSHIFT_ERROR_BUSY: StickShiftResult = -4;
/// No Metal device, or Metal is unavailable on this platform.
pub const STICKSHIFT_ERROR_NO_DEVICE: StickShiftResult = -5;
/// Not supported by this context or build.
pub const STICKSHIFT_ERROR_UNSUPPORTED: StickShiftResult = -6;
/// A file could not be read.
pub const STICKSHIFT_ERROR_IO: StickShiftResult = -7;
/// A glTF or image could not be decoded.
pub const STICKSHIFT_ERROR_PARSE: StickShiftResult = -8;
/// An engine limit was hit (handle table, texture size, buffer length).
pub const STICKSHIFT_ERROR_LIMIT_EXCEEDED: StickShiftResult = -9;
/// A bug in the engine (a Rust panic caught at the boundary). Destroy the context.
pub const STICKSHIFT_ERROR_INTERNAL: StickShiftResult = -10;

// ---------------------------------------------------------------------------
// Enum-like values
// ---------------------------------------------------------------------------

/// Which renderer a context uses.
pub type StickShiftBackend = u32;
/// The Metal renderer (Apple platforms).
pub const STICKSHIFT_BACKEND_METAL: StickShiftBackend = 0;
/// No GPU: resources and frames are tracked, draws counted, nothing drawn, and
/// read-backs return zeros. For host-side tests.
pub const STICKSHIFT_BACKEND_NULL: StickShiftBackend = 1;

/// The state of a resource handle.
pub type StickShiftResourceState = u32;
/// Not a live handle: never issued, destroyed, or another context's.
pub const STICKSHIFT_RESOURCE_STATE_INVALID: StickShiftResourceState = 0;
/// Accepted; the work to make it usable has not finished.
pub const STICKSHIFT_RESOURCE_STATE_PENDING: StickShiftResourceState = 1;
/// Usable in draws.
pub const STICKSHIFT_RESOURCE_STATE_READY: StickShiftResourceState = 2;
/// The work failed; draws using it are skipped. Destroy it.
pub const STICKSHIFT_RESOURCE_STATE_FAILED: StickShiftResourceState = 3;

/// What a [`StickShiftEvent`] reports.
pub type StickShiftEventType = u32;
/// The context can render.
pub const STICKSHIFT_EVENT_CONTEXT_READY: StickShiftEventType = 1;
/// A resource (`object`) became ready.
pub const STICKSHIFT_EVENT_RESOURCE_READY: StickShiftEventType = 2;
/// A resource (`object`) failed; `result` and `message` say why.
pub const STICKSHIFT_EVENT_RESOURCE_FAILED: StickShiftEventType = 3;
/// The GPU finished frame `frame`.
pub const STICKSHIFT_EVENT_FRAME_COMPLETED: StickShiftEventType = 4;
/// The pixels requested for frame `frame` are in the caller's buffer.
pub const STICKSHIFT_EVENT_READBACK_COMPLETE: StickShiftEventType = 5;
/// The pixels requested for frame `frame` will never arrive.
pub const STICKSHIFT_EVENT_READBACK_FAILED: StickShiftEventType = 6;

/// The vertex layout of a mesh, which also selects the pipeline that draws it.
pub type StickShiftVertexLayout = u32;
/// `float3 position, float3 normal, float3 linear RGB colour` (36 bytes).
pub const STICKSHIFT_VERTEX_LAYOUT_POS_NORMAL_COLOR: StickShiftVertexLayout = 0;
/// `float3 position, float3 normal, float2 uv` (32 bytes), drawn with a
/// base-colour texture.
pub const STICKSHIFT_VERTEX_LAYOUT_POS_NORMAL_UV: StickShiftVertexLayout = 1;

/// Shadow quality.
pub type StickShiftShadowQuality = u32;
/// No shadow pass.
pub const STICKSHIFT_SHADOW_QUALITY_OFF: StickShiftShadowQuality = 0;
/// A 2048² shadow map.
pub const STICKSHIFT_SHADOW_QUALITY_LOW: StickShiftShadowQuality = 1;
/// A 4096² shadow map (the default).
pub const STICKSHIFT_SHADOW_QUALITY_HIGH: StickShiftShadowQuality = 2;

/// How a texture's colour channels are encoded.
pub type StickShiftColorSpace = u32;
/// sRGB-encoded colour (photos, painted albedo, glTF base-colour images): the
/// sampler decodes it to linear before lighting. The default (0).
pub const STICKSHIFT_COLOR_SPACE_SRGB: StickShiftColorSpace = 0;
/// Data sampled as stored: SDF font atlases, masks. Required for font atlases
/// and `STICKSHIFT_FILL_SDF` quads.
pub const STICKSHIFT_COLOR_SPACE_LINEAR: StickShiftColorSpace = 1;

/// How a 2D quad is filled.
pub type StickShiftFill = u32;
/// The quad's colour, untextured.
pub const STICKSHIFT_FILL_SOLID: StickShiftFill = 0;
/// `texture` sampled as RGBA, times the colour.
pub const STICKSHIFT_FILL_TEXTURE: StickShiftFill = 1;
/// `texture` sampled as a signed-distance field (distance in red), tinted by
/// the colour. How text is drawn. The texture must be
/// `STICKSHIFT_COLOR_SPACE_LINEAR`.
pub const STICKSHIFT_FILL_SDF: StickShiftFill = 2;

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

/// A 3-component vector.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftVec3 {
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
pub struct StickShiftQuat {
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
pub struct StickShiftMat4 {
    /// The 16 elements, column by column.
    pub m: [f32; 16],
}

/// A linear RGBA colour, each channel `0..1`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftColor {
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
pub struct StickShiftMesh {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// A texture owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StickShiftTexture {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// A glTF model owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StickShiftModel {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

/// An SDF font owned by a context. `id == 0` is the null handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StickShiftFont {
    /// The handle's identity; meaningful only to the context that issued it.
    pub id: u64,
}

// ---------------------------------------------------------------------------
// Data ownership, events, completions
// ---------------------------------------------------------------------------

/// Called exactly once when the engine no longer needs borrowed [`StickShiftData`].
/// NULL means the data is copied.
pub type StickShiftReleaseFn =
    Option<unsafe extern "C" fn(user: *mut c_void, ptr: *const c_void, len: usize)>;

/// Bulk data passed to the engine: copied (`release` NULL) or borrowed until
/// `release` is called.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StickShiftData {
    /// The bytes. May be NULL only when `len` is 0.
    pub ptr: *const c_void,
    /// Length in bytes.
    pub len: usize,
    /// NULL: the engine copies the bytes before the call returns. Otherwise the
    /// bytes are borrowed until the engine calls `release(user, ptr, len)`.
    pub release: StickShiftReleaseFn,
    /// Passed to `release`.
    pub user: *mut c_void,
}

/// A completion or status report. See `docs/C_API.md`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StickShiftEvent {
    /// What happened: a `STICKSHIFT_EVENT_*` value.
    pub kind: StickShiftEventType,
    /// `STICKSHIFT_OK`, or why it failed.
    pub result: StickShiftResult,
    /// The handle id the event is about, or 0.
    pub object: u64,
    /// The frame number the event is about, or 0.
    pub frame: u64,
    /// Detail for failures (NUL-terminated UTF-8), or NULL. Valid until the
    /// callback returns, or until the next `stickshift_poll_events` on the context.
    pub message: *const c_char,
}

/// Receives one event, on a host thread inside a `stickshift_*` call. May be NULL.
pub type StickShiftCompletionFn =
    Option<unsafe extern "C" fn(user: *mut c_void, event: *const StickShiftEvent)>;

/// A per-call completion callback: the call's event goes to `callback` instead
/// of the poll queue.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StickShiftCompletion {
    /// Receives the event. NULL behaves like passing no completion.
    pub callback: StickShiftCompletionFn,
    /// Passed to `callback`.
    pub user: *mut c_void,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// How to create a context.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StickShiftContextDesc {
    /// Must be `STICKSHIFT_API_VERSION`.
    pub api_version: u32,
    /// `STICKSHIFT_BACKEND_METAL` or `STICKSHIFT_BACKEND_NULL`.
    pub backend: StickShiftBackend,
    /// A `CAMetalLayer *` to render into, or NULL for an offscreen context.
    /// Must be NULL with `STICKSHIFT_BACKEND_NULL`.
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
pub struct StickShiftRenderSettings {
    /// A `STICKSHIFT_SHADOW_QUALITY_*` value.
    pub shadows: StickShiftShadowQuality,
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
pub struct StickShiftStats {
    /// Frames handed to the GPU.
    pub frames_submitted: u64,
    /// Frames the GPU has finished.
    pub frames_completed: u64,
    /// `stickshift_frame_acquire` or `stickshift_frame_submit` calls that returned
    /// `STICKSHIFT_FRAME_SKIPPED`.
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
pub struct StickShiftMeshDesc {
    /// A `STICKSHIFT_VERTEX_LAYOUT_*` value.
    pub layout: StickShiftVertexLayout,
    /// Interleaved vertices: a whole number of vertices of `layout`.
    pub vertices: StickShiftData,
    /// `uint32_t` triangle-list indices: a multiple of 3, each below the vertex
    /// count. Any alignment.
    pub indices: StickShiftData,
}

/// How to create a texture.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StickShiftTextureDesc {
    /// Width in pixels, `1..=16384`.
    pub width: u32,
    /// Height in pixels, `1..=16384`.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8, rows top to bottom.
    pub rgba8: StickShiftData,
    /// How the colour channels are encoded: `STICKSHIFT_COLOR_SPACE_SRGB` for colour
    /// images, `STICKSHIFT_COLOR_SPACE_LINEAR` for data such as a font atlas.
    pub color_space: StickShiftColorSpace,
    /// Reserved; must be 0.
    pub flags: u32,
}

/// One glyph of an SDF font, in em units.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftGlyph {
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
pub struct StickShiftFontDesc {
    /// The SDF atlas texture (distance in the red channel).
    pub atlas: StickShiftTexture,
    /// The code point `glyphs[0]` describes.
    pub first_codepoint: u32,
    /// `glyph_count` glyphs for consecutive code points. May be NULL when
    /// `glyph_count` is 0.
    pub glyphs: *const StickShiftGlyph,
    /// The number of glyphs.
    pub glyph_count: u32,
    /// Baseline-to-baseline distance, em units.
    pub line_height: f32,
    /// Used for code points outside the range.
    pub fallback: StickShiftGlyph,
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// The camera a frame is seen from.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftCamera {
    /// World to clip.
    pub view_projection: StickShiftMat4,
    /// World-space eye position, for specular highlights.
    pub position: StickShiftVec3,
}

/// The sun and sky a frame is lit by.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftSunSky {
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
pub struct StickShiftMaterial {
    /// Multiplies the base-colour texture (`POS_NORMAL_UV` meshes).
    pub base_color: StickShiftColor,
    /// The base-colour texture for `POS_NORMAL_UV` meshes; `{0}` for white.
    pub base_color_texture: StickShiftTexture,
}

/// A 2D overlay quad, in drawable pixels with the origin at the top-left.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StickShiftQuad {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Texture rectangle, left (ignored for `STICKSHIFT_FILL_SOLID`).
    pub u0: f32,
    /// Texture rectangle, top.
    pub v0: f32,
    /// Texture rectangle, right.
    pub u1: f32,
    /// Texture rectangle, bottom.
    pub v1: f32,
    /// Tint; alpha blends.
    pub color: StickShiftColor,
    /// A `STICKSHIFT_FILL_*` value.
    pub fill: StickShiftFill,
    /// The texture for `STICKSHIFT_FILL_TEXTURE` and `STICKSHIFT_FILL_SDF`.
    pub texture: StickShiftTexture,
}
