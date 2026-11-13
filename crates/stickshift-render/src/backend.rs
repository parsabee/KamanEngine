// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The raw-Metal backend: [`MetalRenderer`], implementing the render seam.
//!
//! This is the migrated rasterization renderer from the prototype, restructured
//! to satisfy the `kaman-render-api` traits ([`RenderDevice`] +
//! [`FrameRecorder`]) so that code above the seam never sees a Metal type. The
//! draw path is a faithful port of the prototype's
//! `render_with_transforms_and_colors`: it clears to a fixed color, binds a
//! depth-tested Phong pipeline, and draws each mesh with a per-draw MVP uniform.
//!
//! # Behavior preservation
//!
//! **Mesh** buffers are persistent: geometry is deindexed and uploaded
//! into an `MTLBuffer` **once** in [`create_mesh`](RenderDevice::create_mesh) and
//! kept in a generational [`Registry`], so the per-frame draw path performs **no
//! mesh allocation** — it looks the buffer up by [`MeshHandle`]. The uniform ring removes
//! the last per-frame allocation, the per-draw uniform buffer, by writing each
//! draw's MVP into a persistent **uniform ring** at a rotating, 256-byte-aligned
//! offset (see [the ring section](#uniform-ring)). The depth texture is
//! the one texture the prototype already cached, and that caching is preserved.
//! None of this changes any rendered pixel, so the pixel-hash guard
//! stays valid.
//!
//! # Uniform ring
//!
//! A single persistent uniform `MTLBuffer` is allocated once in
//! [`build`](MetalRenderer). Each `draw_mesh` writes its [`Uniforms`] into the
//! next ring slot at a **256-byte-aligned offset** (the Apple GPU
//! `set_vertex_buffer` offset requirement) and binds the ring at that offset —
//! no `new_buffer*` on the hot path. Invariants:
//!
//! - **Stride/alignment:** each slot is [`UNIFORM_RING_STRIDE`] (256) bytes so
//!   every per-draw offset is a multiple of 256; a 192-byte [`Uniforms`] (MVP,
//!   model and normal matrices) fits with padding.
//! - **Capacity:** the ring holds `ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT`
//!   slots; [`MAX_FRAMES_IN_FLIGHT`] is `3`. A frame that exceeds its per-frame
//!   capacity grows the ring (doubling, an allocation-time event), never a
//!   per-draw alloc in steady state.
//! - **Don't stomp an in-flight slot:** the ring is partitioned into
//!   `MAX_FRAMES_IN_FLIGHT` disjoint regions. Frame pacing selects the region with
//!   `frame_index % MAX_FRAMES_IN_FLIGHT` (behind the frames-in-flight
//!   semaphore) so the CPU never overwrites a region the GPU is still reading.
//!   The cursor resets each `begin_frame`.
//!
//! # Per-frame light upload
//!
//! The frame's shared fragment uniforms — the sun, the look params, and the
//! camera block — are a [`LightUniforms`] uploaded **once per frame** into a
//! second, dedicated ring (`light_ring`), partitioned exactly like the uniform
//! ring: one [`UNIFORM_RING_STRIDE`]-byte slot per in-flight frame, selected by
//! `frame_index % MAX_FRAMES_IN_FLIGHT`, so the frames-in-flight semaphore proves
//! the CPU never rewrites a slot the GPU is still reading (see
//! [`write_light_to_ring`](MetalRenderer)). It is written in `begin_frame`, before
//! the sky pass, and the resulting offset is bound at fragment `[[buffer(0)]]` for
//! the sky *and* every draw in the frame — one sun per frame, shared by the sky
//! disc and the shading.
//!
//! It is a separate buffer from `uniform_ring` deliberately: the uniform ring may
//! *reallocate* mid-frame when a frame exceeds its capacity, which would leave the
//! frame-long light binding pointing at a stale offset in a dead buffer.
//!
//! Before the sun moved into the seam, the light was a single buffer written once at construction, so
//! nothing above the seam could set the sun at all.
//!
//! # Deferred encoding and the shadow pass
//!
//! A frame is **two** render passes on one command buffer: a depth-only shadow
//! pass from the sun, then the scene pass (sky, lit draws, overlay). The shadow
//! pass needs every caster before the scene pass starts, so the recorder defers
//! encoding: [`begin_frame`](FrameRecorder::begin_frame) acquires the target and
//! uploads the frame's light (including the fitted light matrix — see
//! [`crate::shadow`]), [`draw_mesh`](FrameRecorder::draw_mesh) writes its uniforms
//! into the ring as before but only *records* the draw in a retained list, and
//! [`submit`](FrameRecorder::submit) encodes both passes from that list — the shadow
//! pass first, through the dedicated position-only `shadow_vertex_main`, then the
//! scene pass exactly as it used to be encoded. The list keeps its capacity across
//! frames, so the steady state allocates nothing.
//!
//! **Store actions (TBDR).** The scene pass's MSAA attachments resolve in-tile and
//! are discarded (`MultisampleResolve` / `DontCare`, memoryless on iOS). The shadow
//! map is the opposite case: it is written in one pass and *sampled* by the next,
//! so its depth attachment must be `Store` into a `Private` (never `Memoryless`)
//! texture — otherwise the scene pass silently samples garbage. Both facts are
//! asserted: the texture's storage/usage when it is created, and the attachment's
//! store action every time the pass descriptor is built
//! (see [`SHADOW_MAP_STORE_ACTION`]).
//!
//! One shadow map is shared by every frame in flight. That is safe because it is a
//! hazard-tracked resource on a single command queue: Metal orders frame `N+1`'s
//! shadow-pass write after frame `N`'s scene-pass reads.
//!
//! # Frames-in-flight pacing
//!
//! The backend triple-buffers with a counting [`FrameSemaphore`] initialized to
//! [`MAX_FRAMES_IN_FLIGHT`] (`3`) permits:
//!
//! - **Wait:** [`begin_frame`](FrameRecorder::begin_frame) *acquires* one permit
//!   before it records anything. If three frames are already queued on the GPU
//!   the CPU **blocks** here (on a condvar — never a busy-wait) until the oldest
//!   frame finishes.
//! - **Signal:** [`submit`](FrameRecorder::submit) registers an
//!   `MTLCommandBuffer` **completion handler** that *releases* one permit. The
//!   handler runs on a Metal-owned thread and is **allocation-free** — it only
//!   signals the semaphore (see [`FrameSemaphore::release`]).
//! - **Region selection:** a monotonically increasing `frame_index` advances
//!   once per submitted frame; `begin_frame` sets
//!   `ring_region_base = (frame_index % MAX_FRAMES_IN_FLIGHT) * ring_draws_per_frame`
//!   so consecutive frames write disjoint ring regions in the cycle `0,1,2,0,…`.
//!
//! **Invariant (wait on frame F before writing slot F+N):** frame `F`'s ring
//! region is reused only by frame `F + MAX_FRAMES_IN_FLIGHT`, and the CPU cannot
//! begin that later frame until frame `F`'s completion handler has released a
//! permit. The semaphore therefore *enforces* that no in-flight ring slot is
//! CPU-written while the GPU still reads it. Present is tied to the drawable
//! (windowed path), so pacing needs no spin.
//!
//! # Non-blocking frames
//!
//! The seam's `begin_frame` / `submit` may block (on the semaphore, and on the
//! GPU for an offscreen `submit`). Hosts that must never wait (the C API) use the
//! inherent counterparts instead:
//!
//! - [`try_begin_frame`](MetalRenderer::try_begin_frame) takes a permit only if
//!   one is free and reports a [`FrameSkip`] otherwise;
//! - [`submit_async`](MetalRenderer::submit_async) commits without waiting on any
//!   target and returns the frame's number;
//! - [`completed_frame_count`](MetalRenderer::completed_frame_count), bumped by
//!   each frame's completion handler, says which frames are done (frames on one
//!   queue complete in submission order);
//! - [`request_readback`](MetalRenderer::request_readback) copies an offscreen
//!   frame's pixels into a buffer owned by the returned [`Readback`], encoded in
//!   that frame's own command buffer, so the pixels are that frame's even when
//!   later frames have drawn since;
//! - [`discard_frame`](MetalRenderer::discard_frame) abandons an open frame and
//!   returns its permit.
//!
//! Texture uploads do not wait either: the mipmap blit is committed on the same
//! queue as every later frame, and one queue runs its command buffers in commit
//! order. Destroying a mesh or texture mid-flight is safe for the same kind of
//! reason: command buffers retain every resource they encode, so the GPU memory
//! outlives the frames that use it.
//!
//! Frame opening, submission and the mip blit each run inside an Objective-C
//! autorelease pool, so the drawable, command buffers and encoders Metal hands
//! back autoreleased are drained per frame even on a host thread without a pool
//! of its own.
//!
//! # Allocation instrument
//!
//! Every `new_buffer*` the backend issues goes through
//! [`count_new_buffer`](MetalRenderer) helpers that bump an
//! [`allocation_count`](MetalRenderer::allocation_count). Tests snapshot the
//! count around the per-frame path and assert it stays `0` for the reference
//! scene after load — with meshes persistent and uniforms
//! ring-allocated the per-frame delta is now **0**. The counter is
//! public so the frames-in-flight tests can reuse it.
//!
//! # Vertex color vs. material
//!
//! The untextured (Phong) pipeline reads color from the per-vertex data, so
//! [`MaterialParams`] does not change its output; the textured pipeline
//! multiplies the sampled base color by `MaterialParams::base_color`.

use std::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use block::ConcreteBlock;
use cocoa::base::id as cocoa_id;
use cocoa::foundation::NSRect;
use core_graphics_types::geometry::CGSize;
use metal::{Device, MTLPixelFormat, MTLResourceOptions, MetalLayer};
use objc::rc::autoreleasepool;
use objc::runtime::YES;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use kaman_math::glam::{Mat4, Vec3};
use kaman_math::Transform;
use kaman_render_api::{
    ColorSpace, FrameRecorder, MaterialParams, MeshData, MeshHandle, OverlayFill, OverlayQuad,
    PipelineDescriptor, PipelineHandle, PipelineKind, RenderDevice, RenderSettings, SunSky,
    TextureData, TextureHandle, VertexFormat, VertexLayout,
};

use crate::deindex::deindex;
use crate::error::{check_size, rgba8_len, RenderError};
use crate::frame_sync::FrameSemaphore;
use crate::registry::Registry;
use crate::shadow::{
    fit_shadow_frustum, resolve_shadow_distance, shadow_distance_for_fog, shadow_map_size_for,
    SHADOW_MAP_SIZE,
};
use crate::vertex::{
    normal_matrix, LightUniforms, MaterialUniforms, Uniforms, Vertex, UNIFORM_RING_STRIDE,
};

/// Store action for the shadow map's depth attachment: **`Store`**.
///
/// On a TBDR GPU a depth attachment is normally discarded at the end of its pass
/// (`DontCare`), as the scene pass's MSAA depth is. The shadow map is written by
/// one pass and sampled by the next, so it must be stored to memory; anything else
/// is silent — the scene pass would sample undefined texels. Asserted where the
/// pass descriptor is built (`configure_shadow_depth_attachment`).
pub const SHADOW_MAP_STORE_ACTION: metal::MTLStoreAction = metal::MTLStoreAction::Store;

/// Storage mode of the shadow map: `Private` on every platform.
///
/// Deliberately *not* [`msaa_storage_mode`]: a memoryless texture lives only in
/// tile memory for the duration of one pass and cannot be sampled by a later one.
fn shadow_map_storage_mode() -> metal::MTLStorageMode {
    metal::MTLStorageMode::Private
}

/// Create a `size`² shadow map: `Depth32Float`, `Private`, usable as a
/// render target and for shader reads. Asserted so a future storage-mode
/// "optimisation" (memoryless, as the MSAA targets are on iOS) fails loudly
/// instead of sampling garbage. Called once at build and again only when the
/// shadow quality changes the size.
fn new_shadow_map(device: &Device, size: u32) -> metal::Texture {
    let desc = metal::TextureDescriptor::new();
    desc.set_texture_type(metal::MTLTextureType::D2);
    desc.set_pixel_format(MTLPixelFormat::Depth32Float);
    desc.set_width(u64::from(size));
    desc.set_height(u64::from(size));
    desc.set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
    desc.set_storage_mode(shadow_map_storage_mode());
    let tex = device.new_texture(&desc);
    assert_ne!(
        tex.storage_mode(),
        metal::MTLStorageMode::Memoryless,
        "the shadow map is sampled after its pass ends; it cannot be memoryless"
    );
    assert!(tex
        .usage()
        .contains(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead));
    tex
}

/// Point a render pass's depth attachment at the shadow map: clear to the far
/// plane (`1.0`, nothing occludes) and **store** the result for the scene pass to
/// sample. Asserts the store action, because getting it wrong is silent.
fn configure_shadow_depth_attachment(
    descriptor: &metal::RenderPassDescriptorRef,
    shadow_map: &metal::TextureRef,
) {
    let depth = descriptor
        .depth_attachment()
        .expect("render pass descriptors always have a depth attachment");
    depth.set_texture(Some(shadow_map));
    depth.set_load_action(metal::MTLLoadAction::Clear);
    depth.set_clear_depth(1.0);
    depth.set_store_action(SHADOW_MAP_STORE_ACTION);
    assert!(
        matches!(depth.store_action(), metal::MTLStoreAction::Store),
        "the shadow map must be stored: the scene pass samples it after this pass ends"
    );
}

/// A caster-only vertex layout: just the position attribute (location
/// 0) of `layout`, at `layout`'s stride — all the depth-only shadow pass reads.
fn shadow_caster_layout(layout: &VertexLayout) -> VertexLayout {
    VertexLayout::new(
        layout.stride,
        layout
            .attributes
            .iter()
            .filter(|a| a.location == 0)
            .copied()
            .collect(),
    )
}

/// The clear color of the reference scene (dark blue-grey). Now only the color
/// the MSAA target is cleared to *before* the gradient sky overwrites it, so it
/// is no longer directly visible; the sky pass paints every pixel of the
/// background. Retained for the depth/edge case where the sky triangle
/// does not cover a fragment.
const CLEAR_COLOR: (f64, f64, f64, f64) = (0.1, 0.1, 0.15, 1.0);

/// MSAA sample count for the scene color/depth attachments.
///
/// 4x on both desktop and device — a good quality/cost trade on TBDR GPUs, where
/// the multisampled attachments live in tile memory and are resolved in-tile
/// (store action `MultisampleResolve`), never spilled to system memory. The
/// count is a single source of truth: the scene pipelines, the sky pipeline, and
/// the MSAA color/depth textures all read it, so raising/lowering it stays
/// consistent.
const MSAA_SAMPLE_COUNT: u64 = 4;

/// Whether the MSAA color/depth attachments should use **memoryless** storage.
///
/// On iOS (TBDR) the multisampled color and depth only ever live in tile memory
/// — they are cleared, drawn, and resolved without ever being read back — so
/// they can be `Memoryless`, costing **zero** system-memory bandwidth (iOS
/// support, v0.2, finishes this). On macOS they must be `Private` (memoryless is not a valid
/// storage mode for a resolvable attachment there). This is the single cfg hook
/// the iOS build flips; the rest of the MSAA wiring is platform-neutral.
#[cfg(target_os = "ios")]
const MSAA_MEMORYLESS: bool = true;
/// Whether the MSAA color/depth attachments should use memoryless storage (macOS: no).
#[cfg(not(target_os = "ios"))]
const MSAA_MEMORYLESS: bool = false;

/// Storage mode for the MSAA color/depth attachments, honoring [`MSAA_MEMORYLESS`].
fn msaa_storage_mode() -> metal::MTLStorageMode {
    if MSAA_MEMORYLESS {
        metal::MTLStorageMode::Memoryless
    } else {
        metal::MTLStorageMode::Private
    }
}

/// The canonical `[position_xyz, normal_xyz, color_rgb]` vertex layout the
/// built-in Phong pipeline binds against (36-byte stride, attributes at
/// 0/12/24). Meshes packed onto this layout (the ECS boxes, and imported glTF
/// geometry via `kaman-assets`) all feed the single shared pipeline.
///
/// The vertex descriptor is now built *from* a [`VertexLayout`] rather
/// than a hardcoded 0/12/24 triple; for this layout the mapping is byte-identical
/// to the old hardcoded descriptor, so the pixel hash is preserved.
fn phong_vertex_layout() -> VertexLayout {
    debug_assert_eq!(mem::size_of::<Vertex>(), 36);
    PipelineKind::VertexColor.vertex_layout()
}

/// The interleaved `[position_xyz, normal_xyz, uv]` layout the **textured**
/// pipeline binds against: two `Float3`s at 0/12 and a `Float2` at 24,
/// 32-byte stride. Meshes carrying a base-color texture (`kaman-assets`
/// `textured_vertex_layout`) are packed onto this and drawn by the textured
/// pipeline, which samples the bound base-color texture at the UV. This is a
/// distinct pipeline from the untextured Phong path, so the box pixel-hash is
/// unaffected.
fn textured_vertex_layout() -> VertexLayout {
    PipelineKind::Textured.vertex_layout()
}

/// Which built-in pipeline can draw a mesh packed on `layout`: `Some(false)`
/// for exactly the `[pos,normal,color]` layout, `Some(true)` for exactly the
/// textured `[pos,normal,uv]` layout, `None` for anything else.
///
/// The match is exact (stride and every attribute) because each pipeline's
/// vertex descriptor hard-codes its stride: a mesh on any other layout would make
/// the GPU fetch vertices at the wrong stride, past the end of its buffer.
fn mesh_pipeline_for_layout(layout: &VertexLayout) -> Option<bool> {
    PipelineKind::for_layout(layout).map(|kind| kind == PipelineKind::Textured)
}

/// Map a seam-neutral [`VertexFormat`] to its native `MTLVertexFormat`.
fn metal_vertex_format(format: VertexFormat) -> metal::MTLVertexFormat {
    match format {
        VertexFormat::Float32 => metal::MTLVertexFormat::Float,
        VertexFormat::Float32x2 => metal::MTLVertexFormat::Float2,
        VertexFormat::Float32x3 => metal::MTLVertexFormat::Float3,
        VertexFormat::Float32x4 => metal::MTLVertexFormat::Float4,
    }
}

/// Build a `metal::VertexDescriptor` from a seam [`VertexLayout`].
///
/// Each [`VertexAttribute`](kaman_render_api::VertexAttribute) becomes one Metal
/// attribute at its `location`, `offset`, and mapped format; the single
/// interleaved buffer (index 0) gets `layout.stride` as its per-vertex stride.
/// For the `[pos,normal,color]` layout this reproduces the old hardcoded
/// descriptor byte-for-byte (three `Float3`s at 0/12/24, stride 36), so the box
/// scene's pixel hash is unchanged.
fn build_vertex_descriptor(layout: &VertexLayout) -> &'static metal::VertexDescriptorRef {
    let descriptor = metal::VertexDescriptor::new();
    for attr in &layout.attributes {
        let native = descriptor
            .attributes()
            .object_at(attr.location as u64)
            .unwrap();
        native.set_format(metal_vertex_format(attr.format));
        native.set_offset(attr.offset as u64);
        native.set_buffer_index(0);
    }
    let buffer_layout = descriptor.layouts().object_at(0).unwrap();
    buffer_layout.set_stride(u64::from(layout.stride));
    buffer_layout.set_step_function(metal::MTLVertexStepFunction::PerVertex);
    descriptor
}

/// The pixel format a texture is uploaded as.
///
/// [`ColorSpace::Srgb`] textures use an `_sRGB` format, so the sampler decodes
/// them to linear before shading (the scene shaders light in linear and encode
/// once at the end); [`ColorSpace::Linear`] data is sampled as stored.
///
/// **RGBA8 is the accepted fallback**: we build on the Command Line
/// Tools with no ASTC transcoder, so uncompressed RGBA8 is what `create_texture`
/// uploads on macOS. The cfg split is where a compressed (ASTC) format would slot
/// in for a mobile/device build — the upload path would then supply pre-compressed
/// blocks and skip runtime mip generation. Kept behind cfg so the desktop build
/// is unaffected and the device path has a single, obvious hook.
#[cfg(feature = "astc-textures")]
fn texture_pixel_format(color_space: ColorSpace) -> MTLPixelFormat {
    // Device/mobile path: ASTC 4x4. Enabled by the `astc-textures` feature,
    // which a device build turns on once a transcoder feeds pre-compressed blocks
    // (out of scope on CLT). Left here so the on-device format is a one-line flip.
    match color_space {
        ColorSpace::Srgb => MTLPixelFormat::ASTC_4x4_sRGB,
        ColorSpace::Linear => MTLPixelFormat::ASTC_4x4_LDR,
    }
}

/// The pixel format a texture is uploaded as (RGBA8 fallback).
#[cfg(not(feature = "astc-textures"))]
fn texture_pixel_format(color_space: ColorSpace) -> MTLPixelFormat {
    match color_space {
        ColorSpace::Srgb => MTLPixelFormat::RGBA8Unorm_sRGB,
        ColorSpace::Linear => MTLPixelFormat::RGBA8Unorm,
    }
}

/// Whether `texture` was uploaded as sRGB, so sampling it decodes to linear.
fn is_srgb(texture: &metal::TextureRef) -> bool {
    matches!(
        texture.pixel_format(),
        MTLPixelFormat::RGBA8Unorm_sRGB | MTLPixelFormat::ASTC_4x4_sRGB
    )
}

/// Check that `data.rgba8` is empty (allocate without contents) or holds at
/// least `4 * width * height` bytes, the amount `replace_region` reads.
///
/// `replace_region` takes a raw pointer and a row stride, so a short slice would
/// be an out-of-bounds read from safe code; every upload goes through this first.
fn validate_rgba8(data: &TextureData<'_>) -> Result<(), RenderError> {
    if data.rgba8.is_empty() {
        return Ok(());
    }
    let need = rgba8_len(data.width, data.height).ok_or_else(|| {
        RenderError::InvalidTexture(format!(
            "{}x{} RGBA8 overflows the address space",
            data.width, data.height
        ))
    })?;
    if data.rgba8.len() < need {
        return Err(RenderError::InvalidTexture(format!(
            "{}x{} RGBA8 needs {need} bytes, got {}",
            data.width,
            data.height,
            data.rgba8.len()
        )));
    }
    Ok(())
}

/// Number of frames the CPU may have in flight before it must wait on the GPU.
///
/// The uniform ring is sized so each of `MAX_FRAMES_IN_FLIGHT` frames owns a
/// disjoint slice: while the GPU reads frame *N*'s slice the CPU writes frame
/// *N+1*'s. Frame pacing uses this as the frames-in-flight semaphore's permit
/// count and to select each frame's ring region with
/// `frame_index % MAX_FRAMES_IN_FLIGHT` (see [`MetalRenderer::begin_frame`]).
/// Defined here so the ring sizing and the pacing share one
/// source of truth.
pub const MAX_FRAMES_IN_FLIGHT: u64 = 3;

/// Initial per-frame uniform-slot capacity of the ring, per in-flight frame.
///
/// The ring starts sized for `INITIAL_MAX_DRAWS_PER_FRAME * MAX_FRAMES_IN_FLIGHT`
/// uniform slots. If a frame issues more draws than the current capacity, the
/// ring **grows** at that draw (an allocation-time event, counted by the allocation
/// instrument) rather than allocating per draw — steady state stays allocation
/// free. Growth doubles the per-frame capacity so it amortizes.
///
/// Sized for the playable demo's steady state with headroom: at its 180-unit
/// draw distance a frame uses ~630 slots (a textured draw takes two), so 1024
/// means the ring never has to grow mid-frame. It is 768 KiB across the three
/// in-flight regions.
const INITIAL_MAX_DRAWS_PER_FRAME: u64 = 1024;

/// A persistent mesh resource: a deindexed vertex buffer, uploaded once, and its
/// vertex count. Held in the [`Registry`] for the mesh's whole lifetime.
struct MeshEntry {
    /// Deindexed vertex buffer (one [`Vertex`] per index), or `None` if empty.
    vertex_buffer: Option<metal::Buffer>,
    /// Number of vertices to draw (== index count of the source mesh).
    vertex_count: u64,
    /// The built-in pipeline whose vertex layout this mesh was packed on
    /// (`Some(true)` textured, `Some(false)` untextured), or `None` if its layout
    /// matches neither, in which case no pipeline can draw it. See
    /// [`mesh_pipeline_for_layout`].
    textured: Option<bool>,
}

/// One vertex of the 2D overlay pass, matching `OverlayVertexIn` in the
/// shader: screen-space position in pixels, atlas UV, RGBA tint, and the fill mode
/// (0 = solid, 1 = textured, 2 = SDF).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct OverlayVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    mode: f32,
}

/// Byte stride of [`OverlayVertex`] — must match the overlay pipeline's vertex
/// layout stride.
const OVERLAY_VERTEX_STRIDE: u64 = std::mem::size_of::<OverlayVertex>() as u64;

/// Fill-mode discriminants shared with the overlay fragment shader.
const OVERLAY_MODE_SOLID: f32 = 0.0;
const OVERLAY_MODE_TEXTURED: f32 = 1.0;
const OVERLAY_MODE_SDF: f32 = 2.0;
/// A textured fill whose texture is sRGB. The overlay is display-referred, so
/// the shader re-encodes the sampler's linear decode: an sRGB image shows the
/// colours it stores, exactly like the same bytes uploaded as linear.
const OVERLAY_MODE_TEXTURED_SRGB: f32 = 3.0;

/// A render target the backend presents into.
///
/// The windowed path renders into a `CAMetalLayer` and presents a drawable; the
/// offscreen path (tests, pixel-hash) renders into a plain color texture that
/// can be read back on the CPU.
enum RenderTarget {
    /// A `CAMetalLayer` attached to a window's `NSView`.
    Surface {
        /// The Metal layer backing the window surface.
        layer: MetalLayer,
    },
    /// An offscreen color texture of fixed size for deterministic readback.
    Offscreen {
        /// The BGRA8 color texture drawn into.
        color: metal::Texture,
        /// Texture width in pixels.
        width: u64,
        /// Texture height in pixels.
        height: u64,
    },
}

/// The Metal rasterization backend.
///
/// Implements both [`RenderDevice`] (resource create/destroy) and
/// [`FrameRecorder`] (per-frame recording), and therefore satisfies
/// `kaman-core`'s `Renderer` marker via its blanket impl. Construct it for a
/// window with [`MetalRenderer::new`], for a `CAMetalLayer` the host owns with
/// [`MetalRenderer::try_from_layer`], or for headless/offscreen rendering with
/// [`MetalRenderer::new_offscreen`]. Each infallible constructor has a `try_*`
/// form that returns a [`RenderError`] instead of panicking.
pub struct MetalRenderer {
    device: Device,
    command_queue: metal::CommandQueue,
    target: RenderTarget,

    pipeline_state: metal::RenderPipelineState,
    // The textured `[pos,normal,uv]` pipeline: samples the bound
    // base-color texture. A distinct pipeline state from `pipeline_state` so the
    // untextured box path is byte-for-byte unchanged.
    textured_pipeline_state: metal::RenderPipelineState,
    // Trilinear (min/mag linear, mip linear) sampler bound with any base-color
    // texture on the textured path.
    sampler_state: metal::SamplerState,
    depth_stencil_state: metal::DepthStencilState,
    // Depth-disabled state for the fullscreen gradient sky pass: the
    // sky is drawn first with no depth test and no depth write, so it fills the
    // background without occluding geometry drawn afterward.
    sky_depth_stencil_state: metal::DepthStencilState,
    // Fullscreen gradient-sky pipeline: `sky_vertex_main` /
    // `sky_fragment_main`, sample-count matched to the MSAA scene attachments,
    // no vertex buffer (it derives a fullscreen triangle from `vertex_id`).
    sky_pipeline_state: metal::RenderPipelineState,
    // 2D overlay / HUD: its own alpha-blended pipeline, a clamped
    // sampler for the SDF atlas, and persistent vertex buffers + a CPU staging
    // vec that are cleared (never reallocated) each frame, so recording a HUD
    // adds no per-frame allocation.
    overlay_pipeline_state: metal::RenderPipelineState,
    overlay_sampler_state: metal::SamplerState,
    overlay_vertices: Vec<OverlayVertex>,
    /// One entry per recorded quad (6 vertices): the texture it samples, if any.
    /// Lets `submit` batch consecutive quads that share a texture into one draw.
    overlay_textures: Vec<Option<TextureHandle>>,
    /// One HUD vertex buffer (and its capacity in vertices) per in-flight frame,
    /// selected by `frame_index % MAX_FRAMES_IN_FLIGHT` like the uniform and light
    /// rings, so the CPU never rewrites HUD vertices a frame still on the GPU is
    /// drawing. Each grows on its own when a frame needs more room.
    overlay_buffers: [Option<(metal::Buffer, usize)>; MAX_FRAMES_IN_FLIGHT as usize],

    // Shadow map. A `shadow_map_size`² `Depth32Float` texture, `Private`
    // and stored (never memoryless — it is sampled by the pass after the one that
    // writes it), re-created only when the shadow quality setting changes its size.
    // Two depth-only caster pipelines share the minimal
    // `shadow_vertex_main`; they differ only in the vertex stride of the layout
    // they read the position out of (untextured 36, textured 32).
    shadow_map: metal::Texture,
    shadow_pipeline_state: metal::RenderPipelineState,
    shadow_textured_pipeline_state: metal::RenderPipelineState,
    // Whether the shadow pass renders casters. When false the pass still runs and
    // clears the map (so the lit shaders never sample garbage) but draws nothing
    // into it, so nothing is shadowed. A diagnostic/test knob: the shadow
    // pixel-hash test flips it to prove the occlusion comes from the pass.
    shadows_enabled: bool,
    // Fixed shadow slab depth, or `None` to follow the fog's draw distance (the
    // default). Resolved against the fog every frame in `write_light_to_ring`,
    // so it can never push shadows past what the fog leaves visible.
    shadow_distance_override: Option<f32>,
    // Render quality settings, as last pushed through the seam. The
    // shadow tier decides whether the shadow pass runs at all and the map's size;
    // the draw-distance scale is applied to the fog below.
    render_settings: RenderSettings,
    // The shadow map's current edge length, in texels (follows the shadow tier).
    shadow_map_size: u32,
    // The fog's `(start, density)` look defaults, captured at build. The
    // draw-distance setting scales from these rather than compounding on the live
    // values.
    base_fog: (f32, f32),
    // Drawable pixels per logical window point: the HUD's UI scale.
    // Reported by `surface_scale`, set by `resize_surface`.
    surface_scale: f32,
    // This frame's recorded draws (deferred encoding), replayed by
    // `submit` into the shadow pass and then the scene pass. Cleared, never
    // shrunk, each frame, so a steady-state frame does not allocate.
    draws: Vec<DrawRecord>,

    // The frame's shared fragment uniforms (sun + look + camera), kept CPU-side
    // and uploaded **once per frame** into `light_ring`. Sticky: the
    // sun/sky half is replaced by `set_sun_sky`, the camera half is refreshed from
    // `view_projection` / `camera_position` as each frame opens.
    light: LightUniforms,
    // Per-frame ring for `light`, partitioned exactly like `uniform_ring`: one
    // `UNIFORM_RING_STRIDE`-byte slot per in-flight frame, selected by
    // `frame_index % MAX_FRAMES_IN_FLIGHT`. See `write_light_to_ring` for why that
    // is enough to never stomp a slot the GPU is still reading. A separate buffer
    // from `uniform_ring` on purpose: the light is written once at `begin_frame`
    // and bound for the whole frame, whereas `uniform_ring` may *reallocate*
    // mid-frame when it grows, which would leave a frame-long binding dangling at
    // a stale offset in a dead buffer.
    light_ring: metal::Buffer,

    // The world → clip view-projection, pushed through the seam via
    // `set_view_projection`. The backend owns no camera; the
    // game/engine computes this from a `kaman_camera::Camera` and hands it across
    // the seam. Combined with each draw's model transform to form the MVP. It is
    // **sticky** — retained across frames until replaced (the engine pushes it
    // once per frame before the game records) — and defaults to identity.
    view_projection: Mat4,
    // The camera's world position, pushed through the seam alongside the matrix
    // so view-dependent specular knows where the eye is. Sticky, and
    // the origin until the first push. Deliberately *not* derived from
    // `view_projection`: the caller already has it, and inverting a projection to
    // get it back is both wasteful and numerically fragile.
    camera_position: Vec3,

    // Persistent per-draw uniform ring. One `MTLBuffer` allocated at
    // build time and re-written every frame at rotating, 256-byte-aligned
    // offsets, so the per-frame draw path issues **no** `new_buffer*`.
    // See `write_uniform_to_ring` for the sub-allocation contract.
    uniform_ring: metal::Buffer,
    // Uniform slots per in-flight frame region; the ring holds
    // `ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT` slots total. Grows (with a
    // fresh, larger buffer) if a frame exceeds it — never a per-draw alloc.
    ring_draws_per_frame: u64,
    // Next free uniform slot **within the current frame's region**, reset to 0 in
    // `begin_frame`. Bounded by `ring_draws_per_frame` (grow if it would exceed).
    ring_cursor: u64,
    // Base slot index of the region this frame writes into, set in `begin_frame`
    // to `(frame_index % MAX_FRAMES_IN_FLIGHT) * ring_draws_per_frame` so an
    // in-flight frame's slots are never stomped.
    ring_region_base: u64,

    // Frames-in-flight pacing. The semaphore starts with
    // `MAX_FRAMES_IN_FLIGHT` permits: `begin_frame` acquires one (blocking if 3
    // frames are queued) and each command buffer's completion handler releases
    // one. `Arc` so a completion handler running on a Metal-owned thread can hold
    // a clone. `frame_index` advances once per submitted frame and drives the
    // per-frame ring region.
    frame_semaphore: Arc<FrameSemaphore>,
    frame_index: u64,
    // Frames the GPU has finished, bumped by each frame's completion handler.
    completed_frames: Arc<AtomicU64>,

    // Cached MSAA (multisampled) depth texture — the depth attachment is now
    // multisampled to match the MSAA color. Reallocated only on resize,
    // like the prototype's single-sample depth cache it replaces.
    depth_texture: Option<metal::Texture>,
    depth_texture_size: (u64, u64),

    // Cached MSAA color texture. The scene renders into this
    // `MSAA_SAMPLE_COUNT`-sample color attachment and resolves into the
    // single-sample target (drawable / offscreen texture) in-tile via the
    // `MultisampleResolve` store action. Reallocated only on resize, so the
    // per-frame path stays allocation-free.
    msaa_color_texture: Option<metal::Texture>,
    msaa_color_size: (u64, u64),

    // Mesh resources live in a generational registry keyed by `MeshHandle`
    // (index + generation); a freed handle is a defined error, never a silent
    // wrong-buffer draw. Textures/pipelines keep the simple index-table scheme.
    meshes: Registry<MeshEntry>,
    pipelines: Vec<Option<PipelineHandleData>>,
    textures: Vec<Option<metal::Texture>>,

    // Allocation instrument: bumped on every `new_buffer*` the backend
    // issues. `AtomicU64` so counting works through a `&self` draw path.
    alloc_count: AtomicU64,

    // Draws `draw_mesh` refused because the mesh's vertex layout does not match
    // the selected pipeline (or matches no built-in pipeline). Diagnostic/test aid.
    rejected_draws: u64,

    // Per-frame recording state (valid between `begin_frame` and `submit`).
    frame: Option<FrameState>,
}

/// Backend-private pipeline record. There is a second (textured) built-in
/// pipeline, so a handle records *which* built-in it selects: `textured` routes
/// draws to the `textured_pipeline_state` (`[pos,normal,uv]`, base-color
/// sampling), otherwise the untextured Phong `pipeline_state`
/// (`[pos,normal,color]`). The choice is derived from the descriptor's vertex
/// layout at `create_pipeline` time.
struct PipelineHandleData {
    /// `true` ⇒ the textured pipeline (base-color sampling); `false` ⇒ untextured.
    textured: bool,
}

/// One recorded draw (deferred encoding): everything `submit` needs to
/// encode it into both the shadow pass and the scene pass.
///
/// The buffers are retained handles, not copies: a draw keeps the exact uniform
/// ring buffer its slot was written into, so a mid-frame ring growth (which
/// swaps `uniform_ring` for a larger buffer) cannot leave an earlier draw bound
/// to the wrong buffer.
struct DrawRecord {
    /// The mesh's persistent vertex buffer.
    vertex_buffer: metal::Buffer,
    /// Vertices to draw.
    vertex_count: u64,
    /// The ring buffer (and 256-byte-aligned offset) holding this draw's [`Uniforms`].
    uniforms: (metal::Buffer, u64),
    /// For textured draws: the ring buffer + offset holding its [`MaterialUniforms`].
    material: Option<(metal::Buffer, u64)>,
    /// Whether the draw uses the textured pipeline (and layout).
    textured: bool,
    /// The base-color texture bound when the draw was recorded, if any.
    texture: Option<metal::Texture>,
}

/// The color target a frame renders into, before the frame is opened.
struct FrameTarget {
    color_texture: metal::Texture,
    drawable: Option<metal::MetalDrawable>,
    width: u64,
    height: u64,
}

/// Why [`MetalRenderer::try_begin_frame`] did not open a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSkip {
    /// A frame is already open; submit or discard it first.
    AlreadyOpen,
    /// [`MAX_FRAMES_IN_FLIGHT`] frames are still on the GPU.
    FramesInFlight,
    /// The layer had no drawable to give (for example the window is hidden).
    NoDrawable,
}

/// A pending copy of one frame's pixels, from
/// [`MetalRenderer::request_readback`].
///
/// The pixels land in a CPU-visible buffer this value owns once the GPU finishes
/// the frame; [`is_complete`](Self::is_complete) says when, without blocking.
/// Row-major BGRA8, `width * 4` bytes per row, top row first.
pub struct Readback {
    buffer: metal::Buffer,
    done: Arc<AtomicBool>,
    width: u32,
    height: u32,
}

impl Readback {
    /// Whether the GPU has finished the frame and the pixels are available.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// The frame's width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The frame's height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The number of bytes the pixels take: `width * height * 4`.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Copy the pixels into `dst`. Returns `false`, copying nothing, if the frame
    /// is not complete yet or `dst` is shorter than [`byte_len`](Self::byte_len).
    pub fn copy_to(&self, dst: &mut [u8]) -> bool {
        let len = self.byte_len();
        if !self.is_complete() || dst.len() < len {
            return false;
        }
        // SAFETY: the buffer is shared storage of exactly `len` bytes, and the GPU
        // finished writing it (`done` is set by the frame's completion handler,
        // after which Metal makes shared-buffer writes visible to the CPU).
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.buffer.contents() as *const u8,
                dst.as_mut_ptr(),
                len,
            );
        }
        true
    }
}

/// Live state for an in-flight frame.
struct FrameState {
    command_buffer: metal::CommandBuffer,
    /// A drawable to present at submit (windowed path only).
    drawable: Option<metal::MetalDrawable>,
    /// The single-sample color target the scene pass resolves into (the drawable's
    /// texture, or the offscreen texture) and its size.
    color_texture: metal::Texture,
    width: u64,
    height: u64,
    /// Whether the currently-selected pipeline is the textured one (set by
    /// `set_pipeline`). Recorded into each draw; decides whether `draw_mesh`
    /// writes a material uniform. Defaults to the untextured pipeline.
    textured_pipeline: bool,
    /// The base-color texture most recently bound by `bind_texture`, recorded into
    /// each subsequent draw (the deferred equivalent of a sticky encoder binding).
    bound_texture: Option<metal::Texture>,
    /// Byte offset into `light_ring` of *this* frame's light upload.
    /// Captured when the frame opens so the sky pass and every later `draw_mesh`
    /// bind the identical bytes — the sun cannot change mid-frame.
    light_offset: u64,
    /// Whether this frame runs the shadow pass, and the map it renders and
    /// samples. Captured with the light block (whose shadow strength and fit
    /// match them), so a settings change before `submit` applies from the next
    /// frame instead of half-way through this one.
    casts_shadows: bool,
    shadow_map: metal::Texture,
    /// A requested read-back: the buffer the pixels are copied into at submit,
    /// and the flag the completion handler sets when they are there.
    readback: Option<(metal::Buffer, Arc<AtomicBool>)>,
}

impl MetalRenderer {
    /// Construct a backend attached to a `winit` window's surface.
    ///
    /// Attaches a `CAMetalLayer` to the window's `NSView`, sizes it to the view
    /// bounds, and compiles the Phong raster pipeline. The infallible wrapper over
    /// [`try_new`](Self::try_new).
    ///
    /// # Panics
    /// Panics with the [`RenderError`] if [`try_new`](Self::try_new) fails: no
    /// Metal device, a shader that fails to compile, or a window that is not an
    /// AppKit window.
    #[must_use]
    pub fn new<W: HasWindowHandle>(window: &W, width: u32, height: u32) -> Self {
        Self::try_new(window, width, height).unwrap_or_else(|e| panic!("MetalRenderer::new: {e}"))
    }

    /// Construct a backend attached to a window's surface, or say why not.
    ///
    /// Attaches a fresh `CAMetalLayer` to the window's `NSView` and sizes its
    /// drawable to the view bounds; `width`/`height` are not used (the bounds
    /// decide), matching [`new`](Self::new).
    ///
    /// # Errors
    /// [`RenderError::NoDevice`] without a Metal device,
    /// [`RenderError::InvalidSurface`] if the window is not an AppKit window, and
    /// the shader/pipeline errors of construction.
    pub fn try_new<W: HasWindowHandle>(
        window: &W,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let device = Device::system_default().ok_or(RenderError::NoDevice)?;
        let view = ns_view(window)?;
        let command_queue = device.new_command_queue();

        let layer = MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_presents_with_transaction(false);

        // Attach the layer to the window's NSView.
        // SAFETY: `view` is the live AppKit `NSView` of `window` (checked by
        // `ns_view`), and `layer` is a valid `CAMetalLayer` we own.
        unsafe {
            let bounds: NSRect = msg_send![view, bounds];
            layer.set_drawable_size(CGSize::new(bounds.size.width, bounds.size.height));
            let _: () = msg_send![view, setWantsLayer: YES];
            let layer_ptr = layer.as_ref() as *const metal::MetalLayerRef as cocoa_id;
            let _: () = msg_send![view, setLayer: layer_ptr];
        }

        let _ = (width, height);
        Self::try_build(device, command_queue, RenderTarget::Surface { layer })
    }

    /// Construct a backend that renders into a `CAMetalLayer` the **host owns**.
    ///
    /// The host keeps the layer in its own view hierarchy (an `NSView`'s layer,
    /// an `MTKView`'s layer, or a `UIView`'s on iOS) and resizes the drawable
    /// through [`resize_surface`](RenderDevice::resize_surface). The backend
    /// retains the layer and configures it: `device`, `pixelFormat`
    /// (`BGRA8Unorm`), `presentsWithTransaction = NO`, and `drawableSize` to
    /// `width` × `height` pixels. `scale` is the drawable pixels per point
    /// reported by [`surface_scale`](RenderDevice::surface_scale) (`<= 0` or
    /// non-finite means 1).
    ///
    /// # Errors
    /// [`RenderError::InvalidSurface`] if `layer` is null or not a `CAMetalLayer`,
    /// [`RenderError::InvalidSize`] for a zero or oversized drawable,
    /// [`RenderError::NoDevice`] without a Metal device, and the shader/pipeline
    /// errors of construction.
    ///
    /// # Safety
    /// `layer` must be null or a valid pointer to a live Objective-C object. The
    /// backend retains it for its own lifetime.
    pub unsafe fn try_from_layer(
        layer: *mut c_void,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, RenderError> {
        use metal::foreign_types::ForeignTypeRef;

        if layer.is_null() {
            return Err(RenderError::InvalidSurface(
                "the CAMetalLayer pointer is null".into(),
            ));
        }
        check_size(width, height)?;
        let class = objc::runtime::Class::get("CAMetalLayer").ok_or_else(|| {
            RenderError::InvalidSurface("the CAMetalLayer class is not loaded".into())
        })?;
        let object = layer as *mut objc::runtime::Object;
        // SAFETY: the caller guarantees `layer` is a live Objective-C object, so
        // sending it `isKindOfClass:` is defined.
        let is_metal_layer: objc::runtime::BOOL = msg_send![object, isKindOfClass: class];
        if is_metal_layer == objc::runtime::NO {
            return Err(RenderError::InvalidSurface(
                "the pointer is not a CAMetalLayer".into(),
            ));
        }
        let device = Device::system_default().ok_or(RenderError::NoDevice)?;
        let command_queue = device.new_command_queue();

        // SAFETY: just checked to be a live CAMetalLayer; `to_owned` retains it,
        // so the backend's reference outlives the caller's.
        let layer = metal::MetalLayerRef::from_ptr(layer.cast()).to_owned();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_presents_with_transaction(false);
        layer.set_drawable_size(CGSize::new(f64::from(width), f64::from(height)));

        let mut renderer = Self::try_build(device, command_queue, RenderTarget::Surface { layer })?;
        if scale.is_finite() && scale > 0.0 {
            renderer.surface_scale = scale;
        }
        Ok(renderer)
    }

    /// Construct a headless backend that renders into an offscreen texture.
    ///
    /// Used by tests and the deterministic pixel-hash guard: there is no window
    /// or drawable; [`read_pixels`](Self::read_pixels) returns the rendered
    /// image. Returns `None` when no Metal device is available (e.g. a GPU-less
    /// CI runner), so callers can skip gracefully.
    ///
    /// # Panics
    /// Panics with the [`RenderError`] for any failure other than a missing
    /// device (a bad size, a shader that fails to compile), so a broken build is
    /// never mistaken for a machine without a GPU.
    #[must_use]
    pub fn new_offscreen(width: u32, height: u32) -> Option<Self> {
        match Self::try_new_offscreen(width, height) {
            Ok(renderer) => Some(renderer),
            Err(RenderError::NoDevice) => None,
            Err(e) => panic!("MetalRenderer::new_offscreen: {e}"),
        }
    }

    /// Construct a headless backend that renders into a `width` × `height`
    /// offscreen texture, or say why not.
    ///
    /// # Errors
    /// [`RenderError::InvalidSize`] for a zero or oversized target,
    /// [`RenderError::NoDevice`] without a Metal device, and the shader/pipeline
    /// errors of construction.
    pub fn try_new_offscreen(width: u32, height: u32) -> Result<Self, RenderError> {
        check_size(width, height)?;
        let device = Device::system_default().ok_or(RenderError::NoDevice)?;
        let command_queue = device.new_command_queue();

        let desc = metal::TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(width as u64);
        desc.set_height(height as u64);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        // Managed/shared so the CPU can read it back after the GPU writes it.
        desc.set_storage_mode(metal::MTLStorageMode::Managed);
        let color = device.new_texture(&desc);

        Self::try_build(
            device,
            command_queue,
            RenderTarget::Offscreen {
                color,
                width: width as u64,
                height: height as u64,
            },
        )
    }

    /// Shared construction: compile the pipelines and allocate the rings.
    fn try_build(
        device: Device,
        command_queue: metal::CommandQueue,
        target: RenderTarget,
    ) -> Result<Self, RenderError> {
        // Rasterization shader library. Default: compile the MSL source at runtime
        // (no toolchain needed). With `precompiled-shaders`: load a .metallib that
        // build.rs compiled ahead of time (requires the Metal toolchain).
        #[cfg(not(feature = "precompiled-shaders"))]
        let library = {
            let shader_source = include_str!("../shaders/rasterization.metal");
            device
                .new_library_with_source(shader_source, &metal::CompileOptions::new())
                .map_err(RenderError::ShaderCompile)?
        };
        #[cfg(feature = "precompiled-shaders")]
        let library = {
            // KAMAN_RASTER_METALLIB is set by build.rs to the compiled library path.
            let metallib: &[u8] = include_bytes!(env!("KAMAN_RASTER_METALLIB"));
            device
                .new_library_with_data(metallib)
                .map_err(RenderError::ShaderCompile)?
        };
        let function = |name: &str| {
            library
                .get_function(name, None)
                .map_err(|e| RenderError::PipelineCreation(format!("{name}: {e}")))
        };
        let pipeline = |descriptor: &metal::RenderPipelineDescriptorRef, what: &str| {
            device
                .new_render_pipeline_state(descriptor)
                .map_err(|e| RenderError::PipelineCreation(format!("{what}: {e}")))
        };

        let vertex_function = function("vertex_main")?;
        let fragment_function = function("fragment_main")?;

        // Vertex descriptor built FROM a `VertexLayout` rather than a
        // hardcoded 0/12/24 Float3 triple. The built-in Phong pipeline binds the
        // canonical `[pos,normal,color]` layout; for that layout this yields the
        // exact same descriptor as before (three Float3s at 0/12/24, stride 36),
        // so the pixel hash is preserved.
        let vertex_descriptor = build_vertex_descriptor(&phong_vertex_layout());

        let pipeline_descriptor = metal::RenderPipelineDescriptor::new();
        pipeline_descriptor.set_vertex_function(Some(&vertex_function));
        pipeline_descriptor.set_fragment_function(Some(&fragment_function));
        pipeline_descriptor.set_vertex_descriptor(Some(vertex_descriptor));
        pipeline_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        pipeline_descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
        // MSAA: the scene pipelines render into a multisampled color +
        // depth attachment resolved in-tile. The pipeline's sample count must
        // match the attachments' or pipeline creation fails.
        pipeline_descriptor.set_sample_count(MSAA_SAMPLE_COUNT);

        let pipeline_state = pipeline(&pipeline_descriptor, "untextured pipeline")?;

        // Textured pipeline: a second pipeline state over the
        // `[pos,normal,uv]` layout with the `textured_*` shader functions, which
        // sample the bound base-color texture. Built here at load time; distinct
        // from `pipeline_state` so the untextured path is unchanged.
        let textured_vertex_function = function("textured_vertex_main")?;
        let textured_fragment_function = function("textured_fragment_main")?;
        let textured_descriptor = metal::RenderPipelineDescriptor::new();
        textured_descriptor.set_vertex_function(Some(&textured_vertex_function));
        textured_descriptor.set_fragment_function(Some(&textured_fragment_function));
        textured_descriptor
            .set_vertex_descriptor(Some(build_vertex_descriptor(&textured_vertex_layout())));
        textured_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        textured_descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
        textured_descriptor.set_sample_count(MSAA_SAMPLE_COUNT);
        let textured_pipeline_state = pipeline(&textured_descriptor, "textured pipeline")?;

        // Gradient-sky pipeline: a fullscreen triangle painted before
        // the geometry, sample-count matched to the MSAA scene attachments. No
        // vertex descriptor — `sky_vertex_main` derives its positions from
        // `vertex_id`. Depth is attached (format must match the pass) but the sky
        // uses a no-write/no-test depth state so it never occludes geometry.
        let sky_vertex_function = function("sky_vertex_main")?;
        let sky_fragment_function = function("sky_fragment_main")?;
        let sky_descriptor = metal::RenderPipelineDescriptor::new();
        sky_descriptor.set_vertex_function(Some(&sky_vertex_function));
        sky_descriptor.set_fragment_function(Some(&sky_fragment_function));
        sky_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        sky_descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
        sky_descriptor.set_sample_count(MSAA_SAMPLE_COUNT);
        let sky_pipeline_state = pipeline(&sky_descriptor, "sky pipeline")?;

        // 2D overlay / HUD pipeline: screen-space quads drawn after the
        // scene. Alpha blended (the HUD composites over the tonemapped image), and
        // paired at draw time with the no-write/no-test depth state so it never
        // depth-fights the scene. Vertex layout mirrors `OverlayVertex`:
        // position(float2), uv(float2), color(float4), mode(float).
        let overlay_vertex_function = function("overlay_vertex_main")?;
        let overlay_fragment_function = function("overlay_fragment_main")?;

        let overlay_vertex_descriptor = metal::VertexDescriptor::new();
        let overlay_attrs: [(u64, metal::MTLVertexFormat, u64); 4] = [
            (0, metal::MTLVertexFormat::Float2, 0),  // position (pixels)
            (1, metal::MTLVertexFormat::Float2, 8),  // uv
            (2, metal::MTLVertexFormat::Float4, 16), // color (RGBA)
            (3, metal::MTLVertexFormat::Float, 32),  // fill mode
        ];
        for (index, format, offset) in overlay_attrs {
            let attr = overlay_vertex_descriptor
                .attributes()
                .object_at(index)
                .unwrap();
            attr.set_format(format);
            attr.set_offset(offset);
            attr.set_buffer_index(0);
        }
        let overlay_layout = overlay_vertex_descriptor.layouts().object_at(0).unwrap();
        overlay_layout.set_stride(OVERLAY_VERTEX_STRIDE);

        let overlay_descriptor = metal::RenderPipelineDescriptor::new();
        overlay_descriptor.set_vertex_function(Some(&overlay_vertex_function));
        overlay_descriptor.set_fragment_function(Some(&overlay_fragment_function));
        overlay_descriptor.set_vertex_descriptor(Some(overlay_vertex_descriptor));
        let overlay_color = overlay_descriptor.color_attachments().object_at(0).unwrap();
        overlay_color.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        // Standard source-over alpha blending for the HUD.
        overlay_color.set_blending_enabled(true);
        overlay_color.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
        overlay_color.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
        overlay_color.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
        overlay_color.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
        overlay_color.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
        overlay_color
            .set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
        overlay_descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
        overlay_descriptor.set_sample_count(MSAA_SAMPLE_COUNT);
        let overlay_pipeline_state = pipeline(&overlay_descriptor, "overlay pipeline")?;

        // Clamped linear sampler for the overlay atlas — SDF glyphs must not wrap
        // or they bleed neighbouring atlas cells into each other.
        let overlay_sampler_descriptor = metal::SamplerDescriptor::new();
        overlay_sampler_descriptor.set_min_filter(metal::MTLSamplerMinMagFilter::Linear);
        overlay_sampler_descriptor.set_mag_filter(metal::MTLSamplerMinMagFilter::Linear);
        overlay_sampler_descriptor.set_address_mode_s(metal::MTLSamplerAddressMode::ClampToEdge);
        overlay_sampler_descriptor.set_address_mode_t(metal::MTLSamplerAddressMode::ClampToEdge);
        let overlay_sampler_state = device.new_sampler(&overlay_sampler_descriptor);

        // Trilinear sampler for base-color textures: min/mag linear + mip linear,
        // so the mipmaps generated in `create_texture` are sampled smoothly.
        let sampler_descriptor = metal::SamplerDescriptor::new();
        sampler_descriptor.set_min_filter(metal::MTLSamplerMinMagFilter::Linear);
        sampler_descriptor.set_mag_filter(metal::MTLSamplerMinMagFilter::Linear);
        sampler_descriptor.set_mip_filter(metal::MTLSamplerMipFilter::Linear);
        sampler_descriptor.set_address_mode_s(metal::MTLSamplerAddressMode::Repeat);
        sampler_descriptor.set_address_mode_t(metal::MTLSamplerAddressMode::Repeat);
        let sampler_state = device.new_sampler(&sampler_descriptor);

        let depth_stencil_descriptor = metal::DepthStencilDescriptor::new();
        depth_stencil_descriptor.set_depth_compare_function(metal::MTLCompareFunction::Less);
        depth_stencil_descriptor.set_depth_write_enabled(true);
        let depth_stencil_state = device.new_depth_stencil_state(&depth_stencil_descriptor);

        // Sky depth state: always pass, never write. The sky fills the
        // background first; disabling depth write means geometry drawn afterward
        // still depth-tests normally against a cleared depth buffer.
        let sky_depth_descriptor = metal::DepthStencilDescriptor::new();
        sky_depth_descriptor.set_depth_compare_function(metal::MTLCompareFunction::Always);
        sky_depth_descriptor.set_depth_write_enabled(false);
        let sky_depth_stencil_state = device.new_depth_stencil_state(&sky_depth_descriptor);

        // Shadow caster pipelines: depth-only, single-sample, the minimal
        // `shadow_vertex_main` and **no fragment function** — casters need no
        // fragment work at all. One per mesh layout, differing only in stride.
        let shadow_vertex_function = function("shadow_vertex_main")?;
        let build_shadow_pipeline = |layout: &VertexLayout| {
            let descriptor = metal::RenderPipelineDescriptor::new();
            descriptor.set_vertex_function(Some(&shadow_vertex_function));
            descriptor.set_fragment_function(None);
            descriptor.set_vertex_descriptor(Some(build_vertex_descriptor(&shadow_caster_layout(
                layout,
            ))));
            descriptor.set_depth_attachment_pixel_format(MTLPixelFormat::Depth32Float);
            descriptor.set_sample_count(1);
            pipeline(&descriptor, "shadow caster pipeline")
        };
        let shadow_pipeline_state = build_shadow_pipeline(&phong_vertex_layout())?;
        let shadow_textured_pipeline_state = build_shadow_pipeline(&textured_vertex_layout())?;

        // The shadow map itself: created once, `Private`, render target + shader
        // read. Asserted so a future storage-mode "optimisation" (memoryless, as the
        // MSAA targets are on iOS) fails loudly instead of sampling garbage.
        let shadow_map = new_shadow_map(&device, SHADOW_MAP_SIZE);

        // The light is uploaded per frame, so this buffer is a ring of
        // one slot per in-flight frame rather than a single blob written once at
        // construction. Same 256-byte stride as the uniform ring, which both keeps
        // every `set_fragment_buffer` offset 256-byte aligned (Apple GPU
        // requirement) and leaves room for the struct to grow.
        let light = LightUniforms::default();
        let light_ring = device.new_buffer(
            MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );

        // Persistent uniform ring: one buffer covering all in-flight frames.
        let ring_draws_per_frame = INITIAL_MAX_DRAWS_PER_FRAME;
        let ring_len = ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE;
        let uniform_ring =
            device.new_buffer(ring_len, MTLResourceOptions::CPUCacheModeDefaultCache);

        Ok(Self {
            device,
            command_queue,
            target,
            pipeline_state,
            textured_pipeline_state,
            sampler_state,
            depth_stencil_state,
            sky_depth_stencil_state,
            sky_pipeline_state,
            overlay_pipeline_state,
            overlay_sampler_state,
            overlay_vertices: Vec::new(),
            overlay_textures: Vec::new(),
            overlay_buffers: [None, None, None],
            shadow_map,
            shadow_pipeline_state,
            shadow_textured_pipeline_state,
            shadows_enabled: true,
            shadow_distance_override: None,
            render_settings: RenderSettings::default(),
            shadow_map_size: SHADOW_MAP_SIZE,
            base_fog: (light.fog_start, light.fog_density),
            surface_scale: 1.0,
            draws: Vec::new(),
            light,
            light_ring,
            // No camera pushed yet; identity until the first `set_view_projection`,
            // and the origin until the first `set_camera_position`.
            view_projection: Mat4::IDENTITY,
            camera_position: Vec3::ZERO,
            uniform_ring,
            ring_draws_per_frame,
            ring_cursor: 0,
            ring_region_base: 0,
            // Triple-buffer pacing: start with a full complement of permits.
            frame_semaphore: Arc::new(FrameSemaphore::new(MAX_FRAMES_IN_FLIGHT as u32)),
            frame_index: 0,
            completed_frames: Arc::new(AtomicU64::new(0)),
            depth_texture: None,
            depth_texture_size: (0, 0),
            msaa_color_texture: None,
            msaa_color_size: (0, 0),
            meshes: Registry::new(),
            pipelines: Vec::new(),
            textures: Vec::new(),
            // Two construction-time allocations: the `light_ring` and the
            // persistent `uniform_ring`. Both are load-time, off the hot path.
            alloc_count: AtomicU64::new(2),
            rejected_draws: 0,
            frame: None,
        })
    }

    /// Read back the offscreen color texture as row-major BGRA8 bytes.
    ///
    /// Only valid on an offscreen backend after a completed frame; returns
    /// `None` for a windowed (surface) backend.
    #[must_use]
    pub fn read_pixels(&self) -> Option<Vec<u8>> {
        let RenderTarget::Offscreen {
            color,
            width,
            height,
        } = &self.target
        else {
            return None;
        };
        let bytes_per_row = width * 4;
        let mut data = vec![0u8; (bytes_per_row * height) as usize];
        let region = metal::MTLRegion {
            origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
            size: metal::MTLSize {
                width: *width,
                height: *height,
                depth: 1,
            },
        };
        color.get_bytes(data.as_mut_ptr() as *mut _, bytes_per_row, region, 0);
        Some(data)
    }

    /// Flush this frame's recorded 2D overlay quads onto the scene
    /// pass's `encoder`, then reset the recording buffers for the next frame.
    ///
    /// Runs after every 3D draw and before `end_encoding`, so the HUD composites
    /// on top of the scene. Uses the no-write/no-test depth state (shared with the
    /// sky) so it neither depth-tests against nor writes into the scene's depth,
    /// and the alpha-blended overlay pipeline. Consecutive quads that sample the
    /// same texture are batched into a single draw.
    ///
    /// The staging `Vec`s are cleared but keep their capacity, so a steady-state
    /// HUD performs no per-frame allocation.
    fn flush_overlay(&mut self, encoder: &metal::RenderCommandEncoderRef, size: (u64, u64)) {
        if self.overlay_vertices.is_empty() {
            self.overlay_textures.clear();
            return;
        }

        // The frame's own size, not the live surface's: a resize after
        // `begin_frame` must not squash this frame's HUD.
        let (surface_w, surface_h) = (size.0 as u32, size.1 as u32);
        if surface_w == 0 || surface_h == 0 {
            self.overlay_vertices.clear();
            self.overlay_textures.clear();
            return;
        }

        // This frame's HUD buffer: the region `frame_index % MAX_FRAMES_IN_FLIGHT`
        // (the frame index advances only after this pass is encoded), behind the
        // same frames-in-flight permit that guards the uniform ring. Grow it only
        // when this frame needs more room.
        let needed = self.overlay_vertices.len();
        let region = (self.frame_index % MAX_FRAMES_IN_FLIGHT) as usize;
        if !matches!(&self.overlay_buffers[region], Some((_, capacity)) if *capacity >= needed) {
            let capacity = needed.next_power_of_two().max(256);
            let buffer = self.counted_new_buffer(
                (capacity as u64) * OVERLAY_VERTEX_STRIDE,
                metal::MTLResourceOptions::StorageModeShared,
            );
            self.overlay_buffers[region] = Some((buffer, capacity));
        }
        let Some((buffer, _)) = &self.overlay_buffers[region] else {
            unreachable!("the region's overlay buffer was just ensured");
        };

        // SAFETY: the buffer is shared-storage, sized for at least `needed`
        // vertices above, and `OverlayVertex` is `#[repr(C)]` plain data.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.overlay_vertices.as_ptr(),
                buffer.contents() as *mut OverlayVertex,
                needed,
            );
        }

        encoder.set_render_pipeline_state(&self.overlay_pipeline_state);
        encoder.set_depth_stencil_state(&self.sky_depth_stencil_state);
        encoder.set_vertex_buffer(0, Some(buffer), 0);
        // The ortho mapping only needs the drawable size; pass it inline.
        let viewport = [surface_w as f32, surface_h as f32];
        encoder.set_vertex_bytes(
            1,
            std::mem::size_of_val(&viewport) as u64,
            viewport.as_ptr() as *const _,
        );
        encoder.set_fragment_sampler_state(0, Some(&self.overlay_sampler_state));

        // Batch consecutive quads that share a texture into one draw call.
        let mut start_quad = 0usize;
        while start_quad < self.overlay_textures.len() {
            let texture = self.overlay_textures[start_quad];
            let mut end_quad = start_quad + 1;
            while end_quad < self.overlay_textures.len()
                && self.overlay_textures[end_quad] == texture
            {
                end_quad += 1;
            }

            let bound = texture
                .and_then(|h| self.textures.get(h.0 as usize))
                .and_then(|slot| slot.as_ref());
            encoder.set_fragment_texture(0, bound.map(|t| t.as_ref()));

            const VERTS_PER_QUAD: usize = 6;
            encoder.draw_primitives(
                metal::MTLPrimitiveType::Triangle,
                (start_quad * VERTS_PER_QUAD) as u64,
                ((end_quad - start_quad) * VERTS_PER_QUAD) as u64,
            );
            start_quad = end_quad;
        }

        self.overlay_vertices.clear();
        self.overlay_textures.clear();
    }

    /// Build (or reuse) the **multisampled** depth texture for the current size.
    ///
    /// the depth attachment is now `MSAA_SAMPLE_COUNT`-sample to match
    /// the MSAA color, and uses [`msaa_storage_mode`] (memoryless on iOS, private
    /// on macOS). Preserves the prototype's caching optimization — reallocated
    /// only when the target size changes — so the per-frame path allocates
    /// nothing.
    fn depth_texture_for(&mut self, width: u64, height: u64) -> metal::Texture {
        if let Some(tex) = &self.depth_texture {
            if self.depth_texture_size == (width, height) {
                return tex.clone();
            }
        }
        let desc = metal::TextureDescriptor::new();
        desc.set_texture_type(metal::MTLTextureType::D2Multisample);
        desc.set_sample_count(MSAA_SAMPLE_COUNT);
        desc.set_pixel_format(MTLPixelFormat::Depth32Float);
        desc.set_width(width);
        desc.set_height(height);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget);
        desc.set_storage_mode(msaa_storage_mode());
        let tex = self.device.new_texture(&desc);
        self.depth_texture = Some(tex.clone());
        self.depth_texture_size = (width, height);
        tex
    }

    /// Build (or reuse) the **multisampled color** texture for the current size.
    /// The scene renders into this `MSAA_SAMPLE_COUNT`-sample color
    /// attachment and resolves in-tile into the single-sample target. Uses
    /// [`msaa_storage_mode`] (memoryless on iOS) and, like the depth cache, is
    /// reallocated only on resize so the per-frame path is allocation-free.
    fn msaa_color_for(&mut self, width: u64, height: u64) -> metal::Texture {
        if let Some(tex) = &self.msaa_color_texture {
            if self.msaa_color_size == (width, height) {
                return tex.clone();
            }
        }
        let desc = metal::TextureDescriptor::new();
        desc.set_texture_type(metal::MTLTextureType::D2Multisample);
        desc.set_sample_count(MSAA_SAMPLE_COUNT);
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(width);
        desc.set_height(height);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget);
        desc.set_storage_mode(msaa_storage_mode());
        let tex = self.device.new_texture(&desc);
        self.msaa_color_texture = Some(tex.clone());
        self.msaa_color_size = (width, height);
        tex
    }

    /// Total number of `new_buffer*` allocations the backend has issued since
    /// construction (the allocation instrument).
    ///
    /// Includes the construction-time light-uniform buffer and uniform ring, and
    /// every mesh upload. Tests snapshot this around the per-frame path
    /// (`begin_frame` … `submit`) and assert the delta is `0` for the reference
    /// scene after load: meshes are persistent and per-draw uniforms
    /// come from the ring, so the hot path allocates nothing. Exposed
    /// for reuse by the frames-in-flight tests.
    #[must_use]
    pub fn allocation_count(&self) -> u64 {
        self.alloc_count.load(Ordering::Relaxed)
    }

    /// The Metal device's name (`MTLDevice.name`), e.g. `"Apple M2"`, or
    /// `"Apple Paravirtual device"` inside a macOS virtual machine such as a
    /// GitHub Actions macOS runner. Test/diagnostic aid: the shadow pixel-hash
    /// guard uses it to tell a real Apple GPU (bit-exact baseline) from a
    /// paravirtualized one (tolerance comparison at the shadow's edges).
    #[must_use]
    pub fn device_name(&self) -> String {
        self.device.name().to_string()
    }

    /// Allocate a Metal buffer with initial data, counting the allocation.
    ///
    /// # Safety
    /// `ptr` must point to at least `length` readable bytes; the same contract as
    /// [`metal::Device::new_buffer_with_data`].
    unsafe fn counted_new_buffer_with_data(
        &self,
        ptr: *const std::ffi::c_void,
        length: u64,
        options: MTLResourceOptions,
    ) -> metal::Buffer {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.device.new_buffer_with_data(ptr, length, options)
    }

    /// Allocate an uninitialized Metal buffer, counting the allocation.
    fn counted_new_buffer(&self, length: u64, options: MTLResourceOptions) -> metal::Buffer {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.device.new_buffer(length, options)
    }

    /// Expand an indexed mesh into a deindexed vertex buffer, uploaded **once**
    /// into a persistent `MTLBuffer`. Called only at load time from
    /// [`create_mesh`](RenderDevice::create_mesh); the buffer then lives in the
    /// [`Registry`] and is referenced by handle on the allocation-free draw path.
    fn upload_mesh(&self, data: &MeshData<'_>) -> MeshEntry {
        // Deindex at the byte level using the layout's declared per-vertex stride,
        // so any layout the seam describes is uploaded correctly — not
        // just the 36-byte `Vertex`. For the `[pos,normal,color]` box layout the
        // stride is `size_of::<Vertex>()`, so the resulting bytes are identical to
        // the prior `Vertex`-typed path (pixel hash unchanged).
        let textured = mesh_pipeline_for_layout(&data.layout);
        let stride = data.layout.stride as usize;
        if stride == 0 || data.indices.is_empty() || data.vertices.len() < stride {
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
                textured,
            };
        }
        // Expand the index buffer into a flat, deindexed byte buffer: one
        // `stride`-byte vertex record per index, copied verbatim. A triangle with
        // an out-of-range index is dropped whole, so the triangles after it keep
        // their vertices.
        let expanded = deindex(data.vertices, stride, data.indices);
        if expanded.is_empty() {
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
                textured,
            };
        }
        if expanded.len() as u64 > self.device.max_buffer_length() {
            // A buffer this large cannot be allocated (`newBuffer` would return
            // nil); `try_create_mesh` reports it, the lenient path draws nothing.
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
                textured,
            };
        }

        // SAFETY: `expanded` is a live `Vec<u8>`; the pointer and byte length
        // describe exactly its contents.
        let buffer = unsafe {
            self.counted_new_buffer_with_data(
                expanded.as_ptr() as *const _,
                expanded.len() as u64,
                MTLResourceOptions::CPUCacheModeDefaultCache,
            )
        };
        MeshEntry {
            vertex_buffer: Some(buffer),
            vertex_count: (expanded.len() / stride) as u64,
            textured,
        }
    }

    /// Upload a mesh like [`create_mesh`](RenderDevice::create_mesh), but
    /// **reject** malformed data instead of uploading what it can.
    ///
    /// The seam's `create_mesh` is lenient (an out-of-range index is skipped, a
    /// layout no pipeline can draw yields an undrawable mesh). This is the strict
    /// path for callers that must report bad input, such as the C API.
    ///
    /// # Errors
    /// [`RenderError::InvalidMesh`] if the layout is not exactly one of the two
    /// built-in pipelines' layouts, the vertex bytes are not a whole number of
    /// vertices, the index count is not a multiple of 3, or an index is out of
    /// range; [`RenderError::LimitExceeded`] if the de-indexed vertex buffer
    /// would exceed the device's `maxBufferLength`.
    pub fn try_create_mesh(&mut self, data: &MeshData<'_>) -> Result<MeshHandle, RenderError> {
        if mesh_pipeline_for_layout(&data.layout).is_none() {
            return Err(RenderError::InvalidMesh(
                "the vertex layout matches neither built-in pipeline".into(),
            ));
        }
        let stride = data.layout.stride as usize;
        if !data.vertices.len().is_multiple_of(stride) {
            return Err(RenderError::InvalidMesh(format!(
                "{} vertex bytes is not a whole number of {stride}-byte vertices",
                data.vertices.len()
            )));
        }
        if !data.indices.len().is_multiple_of(3) {
            return Err(RenderError::InvalidMesh(format!(
                "{} indices is not a whole number of triangles",
                data.indices.len()
            )));
        }
        let vertex_count = data.vertices.len() / stride;
        if let Some(bad) = data.indices.iter().find(|&&i| i as usize >= vertex_count) {
            return Err(RenderError::InvalidMesh(format!(
                "index {bad} is out of range for {vertex_count} vertices"
            )));
        }
        let bytes = (data.indices.len() as u64).saturating_mul(stride as u64);
        if bytes > self.device.max_buffer_length() {
            return Err(RenderError::LimitExceeded(format!(
                "a {bytes}-byte vertex buffer exceeds the device's maxBufferLength ({})",
                self.device.max_buffer_length()
            )));
        }
        Ok(self.create_mesh(data))
    }

    /// Upload a texture like [`create_texture`](RenderDevice::create_texture),
    /// returning an error instead of panicking on bad data.
    ///
    /// Empty `rgba8` allocates the texture without uploading contents.
    ///
    /// # Errors
    /// [`RenderError::InvalidSize`] if a side is zero or above
    /// [`MAX_TEXTURE_DIMENSION`](crate::MAX_TEXTURE_DIMENSION);
    /// [`RenderError::InvalidTexture`] if `rgba8` is non-empty but shorter than
    /// `width * height * 4` bytes.
    pub fn try_create_texture(
        &mut self,
        data: &TextureData<'_>,
    ) -> Result<TextureHandle, RenderError> {
        check_size(data.width, data.height)?;
        validate_rgba8(data)?;
        Ok(self.upload_texture(data))
    }

    /// The texture upload behind [`create_texture`](RenderDevice::create_texture)
    /// and [`try_create_texture`](Self::try_create_texture). `data` is already
    /// validated: each side in `1..=MAX_TEXTURE_DIMENSION`, and `rgba8` empty or at
    /// least `4 * width * height` bytes.
    fn upload_texture(&mut self, data: &TextureData<'_>) -> TextureHandle {
        // RGBA8 upload with a full mip chain. ASTC/compressed formats
        // slot in behind a cfg for on-device builds; RGBA8 is the accepted macOS
        // fallback (we build on the Command Line Tools, no ASTC tooling), so the
        // uncompressed path is the one wired here.
        let width = u64::from(data.width);
        let height = u64::from(data.height);
        // Full mip chain: floor(log2(max(w,h))) + 1 levels.
        let mip_levels = (64 - (width.max(height)).leading_zeros()).max(1) as u64;

        let desc = metal::TextureDescriptor::new();
        desc.set_pixel_format(texture_pixel_format(data.color_space));
        desc.set_width(width);
        desc.set_height(height);
        desc.set_mipmap_level_count(mip_levels);
        // ShaderRead for sampling; RenderTarget lets `generate_mipmaps` write the
        // lower levels (blit-based mip generation requires it on some GPUs).
        desc.set_usage(metal::MTLTextureUsage::ShaderRead | metal::MTLTextureUsage::RenderTarget);
        let tex = self.device.new_texture(&desc);

        if !data.rgba8.is_empty() {
            let region = metal::MTLRegion {
                origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
                size: metal::MTLSize {
                    width,
                    height,
                    depth: 1,
                },
            };
            // Upload the base level (level 0); the mip chain is filled by a blit
            // `generate_mipmaps` below. In bounds: `validate_rgba8` checked that
            // `rgba8` holds at least `height` rows of `width * 4` bytes.
            tex.replace_region(region, 0, data.rgba8.as_ptr() as *const _, width * 4);

            // Generate the lower mip levels on the GPU via a blit encoder. No CPU
            // wait: every frame that samples the texture is committed later on the
            // same command queue, and Metal runs one queue's command buffers in
            // commit order, so the mips are always finished before any draw
            // reads them.
            if mip_levels > 1 {
                autoreleasepool(|| {
                    let command_buffer = self.command_queue.new_command_buffer();
                    let blit = command_buffer.new_blit_command_encoder();
                    blit.generate_mipmaps(&tex);
                    blit.end_encoding();
                    command_buffer.commit();
                });
            }
        }

        let id = self.textures.len() as u32;
        self.textures.push(Some(tex));
        TextureHandle(id)
    }

    /// Write `uniforms` into the next slot of the uniform ring and return the
    /// **256-byte-aligned byte offset** to bind at.
    ///
    /// This replaces the prototype's per-draw
    /// `new_buffer`: the ring is a single persistent buffer, so the steady-state
    /// per-frame path performs **no allocation**. Sub-allocation rules:
    ///
    /// - The offset is `(ring_region_base + ring_cursor) * UNIFORM_RING_STRIDE`,
    ///   always a multiple of 256 (the Apple GPU `set_vertex_buffer` offset
    ///   requirement), so the caller can bind the ring at that offset directly.
    /// - `ring_cursor` advances one slot per draw and resets in `begin_frame`;
    ///   `ring_region_base` selects this frame's disjoint region so an in-flight
    ///   frame's slots are never overwritten (frame pacing sets it via
    ///   `frame_index % MAX_FRAMES_IN_FLIGHT`).
    /// - If the frame would exceed its region capacity the ring **grows** here
    ///   (a load/allocation-time event, counted), never a per-draw alloc in
    ///   steady state.
    fn write_uniform_to_ring(&mut self, uniforms: &Uniforms) -> u64 {
        if self.ring_cursor >= self.ring_draws_per_frame {
            self.grow_ring();
        }
        let slot = self.ring_region_base + self.ring_cursor;
        self.ring_cursor += 1;
        let offset = slot * UNIFORM_RING_STRIDE;
        // SAFETY: `offset + size_of::<Uniforms>()` is within the ring (slot is
        // < total slot count) and the ring is CPU-visible; `Uniforms` is POD.
        unsafe {
            let dst = (self.uniform_ring.contents() as *mut u8).add(offset as usize);
            std::ptr::write(dst as *mut Uniforms, *uniforms);
        }
        offset
    }

    /// Write `material` into the next slot of the uniform ring and return its
    /// 256-byte-aligned byte offset.
    ///
    /// A **textured** draw consumes a second ring slot for its per-draw
    /// [`MaterialUniforms`] (base-color factor), bound at the textured fragment
    /// shader's `[[buffer(2)]]`. Giving it its own slot keeps the fragment buffer
    /// offset 256-byte aligned (Apple GPU requirement) and reuses the same
    /// allocation-free ring as the MVP — the untextured path consumes exactly one
    /// slot per draw as before, so the box scene's per-frame allocation count and
    /// pixel hash are unchanged.
    fn write_material_to_ring(&mut self, material: &MaterialUniforms) -> u64 {
        if self.ring_cursor >= self.ring_draws_per_frame {
            self.grow_ring();
        }
        let slot = self.ring_region_base + self.ring_cursor;
        self.ring_cursor += 1;
        let offset = slot * UNIFORM_RING_STRIDE;
        // SAFETY: `offset + size_of::<MaterialUniforms>()` is within the ring
        // (slot < total slot count) and the ring is CPU-visible; the struct is POD.
        unsafe {
            let dst = (self.uniform_ring.contents() as *mut u8).add(offset as usize);
            std::ptr::write(dst as *mut MaterialUniforms, *material);
        }
        offset
    }

    /// Fold this frame's camera into the light block, write it into the light
    /// ring, and return the **256-byte-aligned byte offset** to bind at.
    ///
    /// Called once per frame from [`begin_frame`](FrameRecorder::begin_frame); the
    /// returned offset is bound for the sky pass and every draw in the frame, so
    /// one frame shades with exactly one sun.
    ///
    /// # Why this cannot race a frame the GPU is still reading
    ///
    /// The ring has one slot per in-flight frame and this picks
    /// `frame_index % MAX_FRAMES_IN_FLIGHT` — the same partitioning, and the same
    /// argument, as the per-draw uniform ring. Frame `F` writes slot
    /// `F mod 3`, which is next written by frame `F + 3`; the CPU cannot *begin*
    /// frame `F + 3` until frame `F`'s command-buffer completion handler has
    /// released a semaphore permit, and this write happens after `begin_frame`
    /// acquired one. So the slot is provably retired before it is rewritten.
    ///
    /// Writing straight into the ring (rather than allocating a buffer per frame)
    /// also keeps the per-frame path allocation-free: the buffer is created
    /// once at construction and never grows — a `LightUniforms` is asserted to fit
    /// one slot by `light_uniforms_fits_one_uniform_ring_slot`.
    fn write_light_to_ring(&mut self) -> u64 {
        // The camera half of the block is refreshed here, from the sticky seam
        // values, so it always matches the matrix this frame's draws project with.
        self.light
            .apply_camera(self.camera_position, self.view_projection);
        // Refit the sun's shadow projection to this frame's camera. The
        // light direction is the light block's own — i.e. `SunSky::direction()` as
        // applied by `set_sun_sky` — so shading, sun disc and shadows share one
        // vector. The slab depth follows the fog, which decides what is visible,
        // unless `set_shadow_distance` fixed a shorter one.
        let fit = fit_shadow_frustum(
            self.view_projection,
            Vec3::from(self.light.direction),
            resolve_shadow_distance(
                shadow_distance_for_fog(self.light.fog_start, self.light.fog_density),
                self.shadow_distance_override,
            ),
            self.shadow_map_size,
        );
        self.light.apply_shadow_fit(&fit);

        let offset = (self.frame_index % MAX_FRAMES_IN_FLIGHT) * UNIFORM_RING_STRIDE;
        // SAFETY: the ring is `MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE` bytes and
        // a `LightUniforms` fits one stride (asserted in `vertex.rs`), so the write
        // is in bounds; the buffer is CPU-visible and `LightUniforms` is POD.
        unsafe {
            let dst = (self.light_ring.contents() as *mut u8).add(offset as usize);
            std::ptr::write(dst as *mut LightUniforms, self.light);
        }
        offset
    }

    /// Double the per-frame ring capacity and reallocate the backing buffer.
    ///
    /// Called only from [`write_uniform_to_ring`](Self::write_uniform_to_ring)
    /// when a frame's draw count exceeds the current per-frame capacity — an
    /// amortized, allocation-time event, not a per-draw one. Draws already
    /// recorded this frame keep a retained handle to the *old* buffer their
    /// uniforms were written into (see `DrawRecord`), so the old contents need not
    /// be copied; the cursor is kept.
    fn grow_ring(&mut self) {
        self.ring_draws_per_frame *= 2;
        let ring_len = self.ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE;
        self.uniform_ring =
            self.counted_new_buffer(ring_len, MTLResourceOptions::CPUCacheModeDefaultCache);
        // The per-frame stride just changed, so the region base must be
        // recomputed against the *live* frame index — not hardcoded to 0, which
        // would put this frame's writes in region 0 while the GPU may still be
        // reading region 0 from an earlier in-flight frame.
        self.ring_region_base =
            (self.frame_index % MAX_FRAMES_IN_FLIGHT) * self.ring_draws_per_frame;
    }

    /// Number of live meshes (created minus destroyed). Test/introspection aid.
    #[must_use]
    pub fn live_mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Number of live pipelines. Test/introspection aid.
    #[must_use]
    pub fn live_pipeline_count(&self) -> usize {
        self.pipelines.iter().filter(|p| p.is_some()).count()
    }

    /// The uniform ring's current within-frame slot cursor (0 after
    /// `begin_frame`, advancing one per drawn mesh). Test/introspection aid used
    /// to prove a rejected draw consumes no slot.
    #[must_use]
    pub fn ring_cursor_for_test(&self) -> u64 {
        self.ring_cursor
    }

    /// The byte offset the *next* uniform-ring write will bind at. Test aid: lets
    /// a test assert the ring's per-slot offsets are 256-byte aligned without
    /// issuing a GPU draw.
    #[must_use]
    pub fn ring_next_offset_for_test(&self) -> u64 {
        (self.ring_region_base + self.ring_cursor) * UNIFORM_RING_STRIDE
    }

    /// The uniform-ring byte offset each draw recorded in the open frame binds
    /// its uniforms at, in record order. Test aid: the offsets the draws will
    /// actually use, not a re-derivation of the formula.
    #[must_use]
    pub fn recorded_uniform_offsets_for_test(&self) -> Vec<u64> {
        self.draws.iter().map(|draw| draw.uniforms.1).collect()
    }

    /// The per-slot byte stride of the uniform ring (256-byte aligned). Test aid.
    #[must_use]
    pub fn ring_stride_for_test(&self) -> u64 {
        UNIFORM_RING_STRIDE
    }

    /// The current frame's ring-region **base slot index**. After
    /// `begin_frame` this is `(frame_index % MAX_FRAMES_IN_FLIGHT) *
    /// ring_draws_per_frame`. Test aid: lets the frames-in-flight test assert the
    /// region rotates `0, ring_draws_per_frame, 2*…, 0, …` across frames.
    #[must_use]
    pub fn ring_region_base_for_test(&self) -> u64 {
        self.ring_region_base
    }

    /// The current per-frame ring capacity in slots (`ring_draws_per_frame`).
    /// Test aid: the region-base rotation test divides
    /// [`ring_region_base_for_test`](Self::ring_region_base_for_test) by this to
    /// recover the region index `0,1,2,0,…`.
    #[must_use]
    pub fn ring_draws_per_frame_for_test(&self) -> u64 {
        self.ring_draws_per_frame
    }

    /// The number of frames submitted so far (`frame_index`). Test aid.
    #[must_use]
    pub fn frame_index_for_test(&self) -> u64 {
        self.frame_index
    }

    /// The light block as it stands CPU-side. Test aid: lets a test
    /// assert a [`SunSky`] pushed through the seam actually became the bytes the
    /// shader will read — including that the light direction is the one
    /// [`SunSky::direction`] derives — without a GPU readback.
    #[must_use]
    pub fn light_for_test(&self) -> LightUniforms {
        self.light
    }

    /// Byte offset in the light ring that the **open** frame's light was written
    /// to and is bound at, or `None` with no frame open. Test aid: proves the
    /// light rotates through one slot per in-flight frame, `0, 256, 512, 0, …`,
    /// instead of being overwritten in place while the GPU may still be reading it.
    #[must_use]
    pub fn light_offset_for_test(&self) -> Option<u64> {
        self.frame.as_ref().map(|frame| frame.light_offset)
    }

    /// Enable or disable **shadow casting**. On by default.
    ///
    /// Disabled, the shadow pass still runs and clears the map, but renders no
    /// casters into it, so every lookup comes back fully lit. This is a
    /// diagnostic/test knob below the seam — the shadow pixel-hash test uses it to
    /// prove the occlusion it pins genuinely comes from the pass — not a quality
    /// setting.
    pub fn set_shadows_enabled(&mut self, enabled: bool) {
        self.shadows_enabled = enabled;
    }

    /// Whether the shadow pass renders casters (see
    /// [`set_shadows_enabled`](Self::set_shadows_enabled)).
    #[must_use]
    pub fn shadows_enabled(&self) -> bool {
        self.shadows_enabled
    }

    /// Set the **shadow draw distance**: how far (view depth, world units) from the
    /// camera shadows are drawn before they fade out.
    ///
    /// - `None` (the default) — **match the draw distance**: the shadow map covers
    ///   everything the distance fog leaves visible (`fog_start + 2 / fog_density`,
    ///   165 units with the default fog) and shadows fade out inside the fog.
    /// - `Some(d)` — a fixed, shorter range of `d` units. The same map then covers
    ///   less ground, so its texels and shadow edges are proportionally smaller.
    ///   Each frame `d` is clamped to between
    ///   [`MIN_SHADOW_DISTANCE`](crate::shadow::MIN_SHADOW_DISTANCE) and the fog's
    ///   draw distance (see [`resolve_shadow_distance`]), because shadows past the
    ///   fog could never be seen.
    ///
    /// A backend-level knob below the render seam, like
    /// [`set_shadows_enabled`](Self::set_shadows_enabled). Games reach it through
    /// the seam's [`RenderSettings::shadow_distance`], which
    /// [`set_render_settings`](RenderDevice::set_render_settings) resolves to world
    /// units and writes here, replacing any earlier value. Takes effect from the
    /// next frame.
    pub fn set_shadow_distance(&mut self, distance: Option<f32>) {
        self.shadow_distance_override = distance;
    }

    /// The shadow draw distance override as last set (see
    /// [`set_shadow_distance`](Self::set_shadow_distance)): `None` when shadows
    /// follow the draw distance. This is the value as requested, before it is
    /// clamped to the fog each frame.
    #[must_use]
    pub fn shadow_distance(&self) -> Option<f32> {
        self.shadow_distance_override
    }

    /// The shadow map's current edge length in texels: 4096 for
    /// [`ShadowQuality::High`](kaman_render_api::ShadowQuality::High), 2048 for
    /// [`ShadowQuality::Low`](kaman_render_api::ShadowQuality::Low). Off keeps the last
    /// map (the pass is skipped, so it costs no fill). Test/diagnostic aid.
    #[must_use]
    pub fn shadow_map_size(&self) -> u32 {
        self.shadow_map_size
    }

    /// The render settings last applied through
    /// [`set_render_settings`](RenderDevice::set_render_settings).
    #[must_use]
    pub fn render_settings(&self) -> RenderSettings {
        self.render_settings
    }

    /// Encode the depth-only **shadow pass**: every recorded draw, as a
    /// caster, from the sun into the shadow map.
    ///
    /// Runs first on the frame's command buffer, before the scene pass that samples
    /// the map. The pass **clamps** depth rather than clipping it, so a caster
    /// nearer the sun than the fitted near plane is flattened onto it and still
    /// casts. Faces are not culled: single-sided geometry (the road, the backdrop)
    /// must cast too.
    fn encode_shadow_pass(&self, frame: &FrameState) {
        let descriptor = metal::RenderPassDescriptor::new();
        configure_shadow_depth_attachment(descriptor, &frame.shadow_map);
        let encoder = frame.command_buffer.new_render_command_encoder(descriptor);
        encoder.set_label("Shadow pass");
        encoder.set_depth_stencil_state(&self.depth_stencil_state);
        encoder.set_depth_clip_mode(metal::MTLDepthClipMode::Clamp);
        encoder.set_cull_mode(metal::MTLCullMode::None);
        // The light block carries the fitted light view-projection; the caster
        // vertex function reads it at vertex `[[buffer(2)]]`.
        encoder.set_vertex_buffer(2, Some(&self.light_ring), frame.light_offset);

        if self.shadows_enabled {
            let mut bound_textured: Option<bool> = None;
            for draw in &self.draws {
                if bound_textured != Some(draw.textured) {
                    encoder.set_render_pipeline_state(if draw.textured {
                        &self.shadow_textured_pipeline_state
                    } else {
                        &self.shadow_pipeline_state
                    });
                    bound_textured = Some(draw.textured);
                }
                encoder.set_vertex_buffer(0, Some(&draw.vertex_buffer), 0);
                encoder.set_vertex_buffer(1, Some(&draw.uniforms.0), draw.uniforms.1);
                encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, draw.vertex_count);
            }
        }
        encoder.end_encoding();
    }

    /// Encode the **scene pass** (sky, every recorded draw through the lit
    /// pipelines, then the 2D overlay) into `frame`'s MSAA target, resolved in-tile
    /// into its color target. This is the original single pass, unchanged except
    /// that it binds the shadow map at fragment `[[texture(1)]]` for the lit
    /// shaders and replays the recorded draw list instead of encoding immediately.
    fn encode_scene_pass(&mut self, frame: &FrameState) {
        let render_pass_descriptor = metal::RenderPassDescriptor::new();

        // MSAA: render into a multisampled color attachment and resolve
        // in-tile into `color_texture` (the drawable / offscreen texture). The
        // `MultisampleResolve` store action performs the resolve on the GPU tile,
        // so the multisampled buffer never spills to system memory — and on iOS
        // it can be memoryless. The offscreen pixel-hash path therefore
        // still ends up with a readable, single-sample resolved texture.
        let msaa_color = self.msaa_color_for(frame.width, frame.height);
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_texture(Some(&msaa_color));
        color_attachment.set_resolve_texture(Some(&frame.color_texture));
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(
            CLEAR_COLOR.0,
            CLEAR_COLOR.1,
            CLEAR_COLOR.2,
            CLEAR_COLOR.3,
        ));
        // Resolve the multisampled color into the single-sample target in-tile.
        color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);

        // The scene's MSAA depth is only needed within this pass: discarded
        // (`DontCare`), unlike the shadow map's depth (`Store`).
        let depth_texture = self.depth_texture_for(frame.width, frame.height);
        let depth_attachment = render_pass_descriptor.depth_attachment().unwrap();
        depth_attachment.set_texture(Some(&depth_texture));
        depth_attachment.set_load_action(metal::MTLLoadAction::Clear);
        depth_attachment.set_clear_depth(1.0);
        depth_attachment.set_store_action(metal::MTLStoreAction::DontCare);

        let encoder = frame
            .command_buffer
            .new_render_command_encoder(render_pass_descriptor)
            .to_owned();

        // Gradient sky first: a fullscreen triangle with depth test/
        // write disabled paints the background before any geometry, replacing the
        // flat clear. Geometry drawn afterward depth-tests against the cleared
        // depth buffer normally. Bind the light (fragment(0)) for the sky colors,
        // the sun disc and the camera's inverse view-projection, which all live in
        // the same `LightUniforms` uploaded when the frame opened.
        encoder.set_render_pipeline_state(&self.sky_pipeline_state);
        encoder.set_depth_stencil_state(&self.sky_depth_stencil_state);
        encoder.set_fragment_buffer(0, Some(&self.light_ring), frame.light_offset);
        encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);

        // Geometry: normal depth test, the frame's light (the same bytes the sky
        // drew its sun from, and the shadow pass fitted with) and the shadow map
        // the pass above just stored.
        encoder.set_depth_stencil_state(&self.depth_stencil_state);
        encoder.set_fragment_texture(1, Some(&frame.shadow_map));
        let mut bound_textured: Option<bool> = None;
        for draw in &self.draws {
            if bound_textured != Some(draw.textured) {
                encoder.set_render_pipeline_state(if draw.textured {
                    &self.textured_pipeline_state
                } else {
                    &self.pipeline_state
                });
                bound_textured = Some(draw.textured);
            }
            if let Some(texture) = &draw.texture {
                encoder.set_fragment_texture(0, Some(texture));
                encoder.set_fragment_sampler_state(0, Some(&self.sampler_state));
            }
            encoder.set_vertex_buffer(0, Some(&draw.vertex_buffer), 0);
            encoder.set_vertex_buffer(1, Some(&draw.uniforms.0), draw.uniforms.1);
            if let Some((buffer, offset)) = &draw.material {
                encoder.set_fragment_buffer(2, Some(buffer), *offset);
            }
            encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, draw.vertex_count);
        }

        // 2D overlay / HUD pass: flush everything recorded this frame
        // on the same encoder, after all 3D draws, so it composites on top.
        self.flush_overlay(&encoder, (frame.width, frame.height));

        encoder.end_encoding();
    }

    /// The color target for the next frame: the layer's next drawable (which may
    /// wait for one to free up, and is `None` if none arrives, e.g. while the
    /// window is hidden), or the offscreen texture.
    fn acquire_target(&self) -> Option<FrameTarget> {
        match &self.target {
            RenderTarget::Surface { layer } => layer.next_drawable().map(|drawable| {
                let color_texture = drawable.texture().to_owned();
                FrameTarget {
                    width: color_texture.width(),
                    height: color_texture.height(),
                    color_texture,
                    drawable: Some(drawable.to_owned()),
                }
            }),
            RenderTarget::Offscreen {
                color,
                width,
                height,
            } => Some(FrameTarget {
                color_texture: color.clone(),
                drawable: None,
                width: *width,
                height: *height,
            }),
        }
    }

    /// Open a frame on `target`, behind a frames-in-flight permit the caller
    /// already holds.
    fn open_frame(&mut self, target: FrameTarget) {
        // NOTE: the view-projection is deliberately NOT reset here. It is sticky
        // across frames (seam contract): the engine loop pushes it once
        // per frame via `set_view_projection` *before* the game opens its frame,
        // so clearing it in `begin_frame` would wipe the engine's camera before
        // the first draw. It defaults to identity until the first push.

        let command_buffer = self.command_queue.new_command_buffer().to_owned();

        // Upload this frame's light before anything is recorded: the sky
        // needs the sun's direction to place its disc, the geometry needs the
        // identical bytes, and the shadow pass needs the light
        // view-projection fitted to this frame's camera. The frame's whole shading
        // state is written once here and bound at the same offset everywhere.
        let light_offset = self.write_light_to_ring();

        // Reset the uniform-ring cursor and select this frame's disjoint region:
        // `(frame_index % MAX_FRAMES_IN_FLIGHT) * ring_draws_per_frame`
        // cycles the region base `0,1,2,0,…`. Behind the frames-in-flight permit,
        // so the region the CPU is about to write is guaranteed not to be one the
        // GPU is still reading.
        self.ring_cursor = 0;
        self.ring_region_base =
            (self.frame_index % MAX_FRAMES_IN_FLIGHT) * self.ring_draws_per_frame;

        // Nothing is encoded yet: draws are recorded and replayed by
        // `submit` into the shadow pass and then the scene pass. Clear (never
        // shrink) the lists so their capacity is reused. The overlay lists should
        // already be empty (`draw_overlay_quad` ignores calls outside a frame);
        // clearing them here keeps a stray quad from ever reaching this frame.
        self.draws.clear();
        self.overlay_vertices.clear();
        self.overlay_textures.clear();

        self.frame = Some(FrameState {
            command_buffer,
            drawable: target.drawable,
            color_texture: target.color_texture,
            width: target.width,
            height: target.height,
            // The untextured Phong pipeline until `set_pipeline` selects the
            // textured one; no base-color texture until `bind_texture`.
            textured_pipeline: false,
            bound_texture: None,
            light_offset,
            casts_shadows: self.render_settings.shadows.casts_shadows(),
            shadow_map: self.shadow_map.clone(),
            readback: None,
        });
    }

    /// Open a frame **without waiting** for the GPU: the non-blocking
    /// counterpart of [`begin_frame`](FrameRecorder::begin_frame).
    ///
    /// Returns `Err(FrameSkip::FramesInFlight)` instead of blocking when
    /// [`MAX_FRAMES_IN_FLIGHT`] frames are still on the GPU, and
    /// `Err(FrameSkip::AlreadyOpen)` if a frame is open. On a layer target it then
    /// takes the next drawable, which can still wait for one to free up (Core
    /// Animation gives up after about a second, giving
    /// `Err(FrameSkip::NoDrawable)`); with a free frame slot a drawable is
    /// normally ready at once. On success the frame is open exactly as after
    /// `begin_frame`.
    ///
    /// # Errors
    /// The [`FrameSkip`] reason no frame was opened. Nothing is recorded and no
    /// permit is held.
    pub fn try_begin_frame(&mut self) -> Result<(), FrameSkip> {
        if self.frame.is_some() {
            return Err(FrameSkip::AlreadyOpen);
        }
        if !self.frame_semaphore.try_acquire() {
            return Err(FrameSkip::FramesInFlight);
        }
        autoreleasepool(|| match self.acquire_target() {
            Some(target) => {
                self.open_frame(target);
                Ok(())
            }
            None => {
                self.frame_semaphore.release();
                Err(FrameSkip::NoDrawable)
            }
        })
    }

    /// Frames submitted to the GPU that have not finished yet (`0..=`
    /// [`MAX_FRAMES_IN_FLIGHT`]), counting an open, unsubmitted frame.
    #[must_use]
    pub fn frames_in_flight(&self) -> u64 {
        MAX_FRAMES_IN_FLIGHT - u64::from(self.frame_semaphore.available())
    }

    /// Frames submitted so far. Frame numbers are 1-based: the `n`th submitted
    /// frame is number `n`, as returned by [`submit_async`](Self::submit_async).
    #[must_use]
    pub fn submitted_frame_count(&self) -> u64 {
        self.frame_index
    }

    /// Frames the GPU has finished, counted by each command buffer's completion
    /// handler. Frames on the backend's single command queue complete in
    /// submission order, so frame `n` is done once this reaches `n`.
    #[must_use]
    pub fn completed_frame_count(&self) -> u64 {
        self.completed_frames.load(Ordering::Acquire)
    }

    /// Ask for the open frame's pixels to be copied out when the GPU finishes it.
    ///
    /// Offscreen targets only, and once per frame: returns `None` without an open
    /// frame, on a layer target, or if this frame already has a read-back. The
    /// copy is encoded at submit into a CPU-visible buffer owned by the returned
    /// [`Readback`], so the pixels are this frame's even when later frames have
    /// already drawn over the target by the time they are read. Discarding the
    /// frame instead of submitting it means the read-back never completes.
    pub fn request_readback(&mut self) -> Option<Readback> {
        let frame = self.frame.as_ref()?;
        if frame.readback.is_some() || frame.drawable.is_some() {
            return None;
        }
        let (width, height) = (frame.width, frame.height);
        let buffer =
            self.counted_new_buffer(width * height * 4, MTLResourceOptions::StorageModeShared);
        let done = Arc::new(AtomicBool::new(false));
        let frame = self.frame.as_mut()?;
        frame.readback = Some((buffer.clone(), Arc::clone(&done)));
        Some(Readback {
            buffer,
            done,
            width: width as u32,
            height: height as u32,
        })
    }

    /// Close the open frame **without rendering it**: its recorded draws are
    /// dropped, nothing is committed, and its frames-in-flight permit is
    /// returned. A no-op without an open frame. A read-back requested for the
    /// frame never completes.
    pub fn discard_frame(&mut self) {
        if self.frame.take().is_some() {
            self.draws.clear();
            self.overlay_vertices.clear();
            self.overlay_textures.clear();
            self.frame_semaphore.release();
        }
    }

    /// Close the open frame and hand it to the GPU **without waiting** for it,
    /// on every target (the seam's [`submit`](FrameRecorder::submit) waits on an
    /// offscreen target). Returns the frame's number (see
    /// [`submitted_frame_count`](Self::submitted_frame_count)), or `None` if no
    /// frame was open. Use [`completed_frame_count`](Self::completed_frame_count)
    /// or a [`Readback`] to learn when it is done.
    pub fn submit_async(&mut self) -> Option<u64> {
        self.submit_frame(false)
    }

    /// Encode, commit and (on a layer) present the open frame; on an offscreen
    /// target, wait for the GPU when `wait_offscreen`. Returns the frame number.
    fn submit_frame(&mut self, wait_offscreen: bool) -> Option<u64> {
        let frame = self.frame.take()?;
        autoreleasepool(|| {
            // Pass order: the depth-only shadow pass first (every recorded
            // draw as a caster, into the stored shadow map), then the scene pass
            // (sky, lit draws sampling that map, and the 2D overlay on top).
            // With shadows off the pass is skipped outright: the light block's
            // zero `shadow_strength` makes the lit shaders ignore the map.
            if frame.casts_shadows {
                self.encode_shadow_pass(&frame);
            }
            self.encode_scene_pass(&frame);

            // Offscreen: synchronize the Managed color texture so `read_pixels`
            // sees this frame, and copy it into the read-back buffer if one was
            // requested. Both are blits after the scene pass, in this frame's
            // command buffer, so they see exactly this frame's pixels.
            if let RenderTarget::Offscreen { color, .. } = &self.target {
                let blit = frame.command_buffer.new_blit_command_encoder();
                if let Some((buffer, _)) = &frame.readback {
                    blit.copy_from_texture_to_buffer(
                        color,
                        0,
                        0,
                        metal::MTLOrigin { x: 0, y: 0, z: 0 },
                        metal::MTLSize {
                            width: frame.width,
                            height: frame.height,
                            depth: 1,
                        },
                        buffer,
                        0,
                        frame.width * 4,
                        frame.width * frame.height * 4,
                        metal::MTLBlitOption::empty(),
                    );
                }
                blit.synchronize_resource(color);
                blit.end_encoding();
            }

            // Frames-in-flight release: register a completion handler that
            // signals the semaphore when the GPU finishes this frame, counts the
            // frame complete, and flags its read-back. The handler runs on a
            // Metal-owned thread, so it must be allocation-free: everything it
            // touches is cloned here on the CPU submit path, and it only releases
            // a permit (lock + counter + condvar notify) and stores two atomics.
            // The permit released here is the one taken when this frame opened.
            let semaphore = Arc::clone(&self.frame_semaphore);
            let completed = Arc::clone(&self.completed_frames);
            let readback_done = frame.readback.as_ref().map(|(_, done)| Arc::clone(done));
            let completion = ConcreteBlock::new(move |_cb: &metal::CommandBufferRef| {
                completed.fetch_add(1, Ordering::Release);
                if let Some(done) = &readback_done {
                    done.store(true, Ordering::Release);
                }
                semaphore.release();
            })
            .copy();
            frame.command_buffer.add_completed_handler(&completion);

            // This frame is now fully recorded and about to be committed: advance the
            // frame index so the *next* frame rotates to the next ring region
            // (`0,1,2,0,…`). The frame's number is the new count.
            self.frame_index = self.frame_index.wrapping_add(1);

            match &frame.drawable {
                Some(drawable) => {
                    // Present is tied to the drawable; pacing is the semaphore, so
                    // there is no busy-wait/spin here.
                    frame.command_buffer.present_drawable(drawable);
                    frame.command_buffer.commit();
                }
                None => {
                    frame.command_buffer.commit();
                    if wait_offscreen {
                        // The completion handler (and its `release`) fires before
                        // this returns, keeping the semaphore balanced.
                        frame.command_buffer.wait_until_completed();
                    }
                }
            }
        });
        Some(self.frame_index)
    }

    /// Draws [`draw_mesh_matrix`](FrameRecorder::draw_mesh_matrix) refused because the
    /// mesh's vertex layout did not match the selected pipeline. Test/diagnostic aid.
    #[must_use]
    pub fn rejected_draw_count(&self) -> u64 {
        self.rejected_draws
    }

    /// Record a draw of `mesh` with a model-to-world **matrix**: the body of
    /// [`draw_mesh_matrix`](FrameRecorder::draw_mesh_matrix) (and so of
    /// [`draw_mesh`](FrameRecorder::draw_mesh)).
    ///
    /// Ignored outside an open frame, for an unknown or freed mesh, and for an
    /// empty one. A mesh whose vertex layout does not match the selected pipeline
    /// (or matches no built-in pipeline) is **rejected** and counted in
    /// [`rejected_draw_count`](Self::rejected_draw_count): the pipeline's vertex
    /// descriptor would fetch at the wrong stride, past the end of the buffer.
    fn record_draw(&mut self, mesh: MeshHandle, model: Mat4, material: &MaterialParams) {
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        // Whether the textured pipeline is currently selected — decides if this
        // draw also writes+binds the per-draw material uniform.
        let textured = frame.textured_pipeline;

        // Look the persistent vertex buffer up by handle — no allocation here.
        // A stale/freed or unknown handle is a defined no-op (the registry
        // returns an error), never a silent wrong-buffer draw. Clone the small
        // buffer handle + count so we can drop the immutable `meshes` borrow
        // before taking the `&mut self` uniform-ring write.
        let Ok(entry) = self.meshes.get(mesh) else {
            return;
        };
        if entry.textured != Some(textured) {
            self.rejected_draws += 1;
            return;
        }
        let Some(vertex_buffer) = entry.vertex_buffer.clone() else {
            return;
        };
        let vertex_count = entry.vertex_count;
        if vertex_count == 0 {
            return;
        }

        // Per-draw MVP uniform: written into the persistent uniform ring at a
        // rotating, 256-byte-aligned offset — **no `new_buffer*` on the hot
        // path**. This replaced the prototype's per-draw allocation
        // with this ring sub-allocation.
        let mvp = self.view_projection * model;
        let uniforms = Uniforms {
            model_view_projection: mvp.to_cols_array_2d(),
            // World matrix for the look stack: the shader reconstructs world
            // position + normal for distance fog and shadow-map lookups, and the
            // shadow pass places the caster with it.
            model: model.to_cols_array_2d(),
            // Normals use the inverse-transpose under non-uniform scale.
            normal_matrix: normal_matrix(model).to_cols_array_2d(),
        };
        let uniform_offset = self.write_uniform_to_ring(&uniforms);

        // Textured draws additionally write the base-color factor into a second
        // ring slot and bind it at the textured fragment shader's `[[buffer(2)]]`.
        // Untextured draws skip this and consume exactly one slot, so their
        // per-frame allocation count and pixel output are unchanged.
        let material_offset = if textured {
            Some(self.write_material_to_ring(&MaterialUniforms {
                base_color_factor: material.base_color,
            }))
        } else {
            None
        };

        // Record, don't encode: `submit` replays this draw into the
        // shadow pass (as a caster) and then the scene pass. The buffer handles are
        // retained, so a later ring growth this frame cannot rebind this draw.
        let frame = self.frame.as_ref().expect("frame checked Some above");
        let record = DrawRecord {
            vertex_buffer,
            vertex_count,
            uniforms: (self.uniform_ring.clone(), uniform_offset),
            material: material_offset.map(|offset| (self.uniform_ring.clone(), offset)),
            textured,
            texture: if textured {
                frame.bound_texture.clone()
            } else {
                None
            },
        };
        self.draws.push(record);
    }

    /// Append `quad` to this frame's overlay: the body of
    /// [`draw_overlay_quad`](FrameRecorder::draw_overlay_quad), which checks for
    /// an open frame first.
    fn record_overlay_quad(&mut self, quad: &OverlayQuad) {
        // Record only: overlay quads are flushed as one ortho pass in `submit`,
        // after every 3D draw, so the HUD always composites on top regardless of
        // when the game recorded it. Pushing into a retained Vec keeps this
        // allocation-free in steady state.
        let mode = match quad.fill {
            OverlayFill::Solid => OVERLAY_MODE_SOLID,
            OverlayFill::Textured(t) => match self.textures.get(t.0 as usize) {
                Some(Some(tex)) if is_srgb(tex) => OVERLAY_MODE_TEXTURED_SRGB,
                _ => OVERLAY_MODE_TEXTURED,
            },
            OverlayFill::Sdf(_) => OVERLAY_MODE_SDF,
        };
        let [x, y, w, h] = quad.rect;
        let [u0, v0, u1, v1] = quad.uv;
        let color = quad.color;

        // Two triangles, counter-clockwise, covering the rect.
        let corners = [
            ([x, y], [u0, v0]),
            ([x, y + h], [u0, v1]),
            ([x + w, y + h], [u1, v1]),
            ([x, y], [u0, v0]),
            ([x + w, y + h], [u1, v1]),
            ([x + w, y], [u1, v0]),
        ];
        for (position, uv) in corners {
            self.overlay_vertices.push(OverlayVertex {
                position,
                uv,
                color,
                mode,
            });
        }
        // Remember which texture each quad wants so `submit` can batch runs that
        // share one; six vertices per quad keeps the index arithmetic trivial.
        self.overlay_textures.push(quad.fill.texture());
    }
}

impl RenderDevice for MetalRenderer {
    fn create_mesh(&mut self, data: &MeshData<'_>) -> MeshHandle {
        // Upload the geometry into a persistent buffer once, then register it.
        let entry = self.upload_mesh(data);
        self.meshes.insert(entry)
    }

    fn destroy_mesh(&mut self, handle: MeshHandle) {
        // Free the slot and bump its generation; the (now stale) handle can never
        // resolve to a live mesh again. An unknown/already-freed handle is a
        // defined no-op (per the seam's "caller error, don't corrupt" contract).
        let _ = self.meshes.remove(handle);
    }

    /// Upload a texture (see the seam contract).
    ///
    /// A zero side is treated as 1, as before. Empty `rgba8` allocates the texture
    /// without uploading contents.
    ///
    /// # Panics
    /// Panics if a side exceeds
    /// [`MAX_TEXTURE_DIMENSION`](crate::MAX_TEXTURE_DIMENSION) or `rgba8` is
    /// non-empty but shorter than `4 * width * height` bytes: uploading it would
    /// read past the end of the slice. Use
    /// [`try_create_texture`](MetalRenderer::try_create_texture) to get an error
    /// instead.
    fn create_texture(&mut self, data: &TextureData<'_>) -> TextureHandle {
        let clamped = TextureData {
            width: data.width.max(1),
            height: data.height.max(1),
            rgba8: data.rgba8,
            color_space: data.color_space,
        };
        if let Err(e) = check_size(clamped.width, clamped.height).and(validate_rgba8(&clamped)) {
            panic!("create_texture: {e}");
        }
        self.upload_texture(&clamped)
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        if let Some(slot) = self.textures.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }

    fn create_pipeline(&mut self, desc: &PipelineDescriptor) -> PipelineHandle {
        // Two built-in pipelines: the descriptor's kind selects which one this
        // handle names, the textured one (base-color sampling over
        // `[pos,normal,uv]`) or the untextured Phong one (`[pos,normal,color]`).
        // The actual `RenderPipelineState` is the shared `pipeline_state` /
        // `textured_pipeline_state`.
        let textured = desc.kind == PipelineKind::Textured;
        let id = self.pipelines.len() as u32;
        self.pipelines.push(Some(PipelineHandleData { textured }));
        PipelineHandle(id)
    }

    fn surface_size(&self) -> (u32, u32) {
        match &self.target {
            RenderTarget::Surface { layer } => {
                let size = layer.drawable_size();
                (size.width as u32, size.height as u32)
            }
            RenderTarget::Offscreen { width, height, .. } => (*width as u32, *height as u32),
        }
    }

    fn safe_area_insets(&self) -> [f32; 4] {
        // Zeros on every platform: a macOS window has no notch/home-indicator
        // intrusions, and on a device the host owns the view and knows its
        // `safeAreaInsets`, so it applies them to its own layout.
        [0.0; 4]
    }

    fn resize_surface(&mut self, width: u32, height: u32, pixels_per_point: f32) {
        // the drawable is sized explicitly (a `CAMetalLayer` does not
        // track its view once `drawableSize` has been set), and CoreAnimation
        // scales it to fill the layer, so a drawable smaller than the window's
        // native pixels is a cheap render-scale upscale. The MSAA color and depth
        // attachments follow the drawable's size lazily (`msaa_color_for` /
        // `depth_texture_for`), so nothing else is re-created here. An offscreen
        // target keeps its fixed size.
        if let RenderTarget::Surface { layer } = &self.target {
            layer.set_drawable_size(CGSize::new(
                f64::from(width.max(1)),
                f64::from(height.max(1)),
            ));
        }
        if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            self.surface_scale = pixels_per_point;
        }
    }

    fn surface_scale(&self) -> f32 {
        self.surface_scale
    }

    fn set_render_settings(&mut self, settings: &RenderSettings) {
        // Draw distance: scale the fog from its look defaults. Start and opaque
        // depth (`start + 2 / density`) both scale by exactly `scale`, so the fog
        // keeps hiding a streamed world whose reach the engine scaled alike.
        let scale =
            if settings.draw_distance_scale.is_finite() && settings.draw_distance_scale > 0.0 {
                settings.draw_distance_scale
            } else {
                1.0
            };
        let (base_start, base_density) = self.base_fog;
        self.light.fog_start = base_start * scale;
        self.light.fog_density = base_density / scale;

        // Shadow range: a fraction of that (new) draw distance, resolved to world
        // units for the existing override (which clamps it to the fog each frame).
        let draw_distance = shadow_distance_for_fog(self.light.fog_start, self.light.fog_density);
        self.shadow_distance_override = settings
            .shadow_distance
            .map(|fraction| fraction.clamp(0.0, 1.0) * draw_distance);

        // Shadow tier: off zeroes the strength (the lit shaders then skip the map
        // and `submit` skips the pass); on re-creates the map only if the size
        // changes. Command buffers retain the textures they reference, so dropping
        // the old map here cannot pull it out from under a frame in flight.
        self.light.shadow_strength = if settings.shadows.casts_shadows() {
            1.0
        } else {
            0.0
        };
        if let Some(size) = shadow_map_size_for(settings.shadows) {
            if size != self.shadow_map_size {
                self.shadow_map = new_shadow_map(&self.device, size);
                self.shadow_map_size = size;
            }
        }

        self.render_settings = *settings;
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        if let Some(slot) = self.pipelines.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }
}

impl FrameRecorder for MetalRenderer {
    /// Open a frame (see the seam contract).
    ///
    /// A second call while a frame is already open is a **no-op**: the open frame
    /// continues. Replacing it would drop its command buffer without ever
    /// releasing its frames-in-flight permit, and the third such call would then
    /// block forever. Quads recorded while no frame was open are discarded here.
    fn begin_frame(&mut self) {
        if self.frame.is_some() {
            return;
        }
        // The drawable, command buffer and encoders Metal hands back are
        // autoreleased; drain them here rather than relying on the host's run
        // loop to have a pool in place (a C host's render thread may not).
        autoreleasepool(|| {
            // Secure the target first, then wait for a frame slot, so a skipped
            // frame (no drawable) never takes a permit it would not release.
            let Some(target) = self.acquire_target() else {
                return;
            };
            // Frames-in-flight gate: block until at most
            // `MAX_FRAMES_IN_FLIGHT - 1` frames are still queued on the GPU, so this
            // frame can be prepared without stomping a ring region the GPU is still
            // reading.
            self.frame_semaphore.acquire();
            self.open_frame(target);
        });
    }

    fn set_view_projection(&mut self, view_proj: Mat4) {
        // Store this frame's world → clip matrix; `draw_mesh` multiplies it by
        // each instance's model transform to form the MVP. Per the seam
        // contract this is pushed once per frame before the first `draw_mesh`.
        self.view_projection = view_proj;
    }

    fn set_camera_position(&mut self, position: Vec3) {
        // The eye position for view-dependent shading. Sticky, like the
        // matrix it accompanies; both are folded into the frame's light block when
        // the frame opens (`write_light_to_ring`), so a push that lands *after*
        // `begin_frame` applies from the next frame — which is why the engine loop
        // pushes the camera before the game records.
        self.camera_position = position;
    }

    fn set_sun_sky(&mut self, sun: &SunSky) {
        // Replace only the sun/sky half of the light block (the look params — fog,
        // shadow strength, specular response — have no seam representation yet and are
        // preserved). Sticky: a game with a fixed sun pushes once at load, and this
        // value is re-uploaded for every subsequent frame.
        self.light.apply_sun_sky(sun);
    }

    fn set_pipeline(&mut self, handle: PipelineHandle) {
        // Select the untextured or textured built-in pipeline for the draws that
        // follow. The untextured pipeline is the default each frame; a
        // draw that wants texturing selects the textured one here. Unknown handle:
        // leave the current selection (defined no-op per the seam contract).
        let textured = match self.pipelines.get(handle.0 as usize) {
            Some(Some(p)) => p.textured,
            _ => return,
        };
        if let Some(frame) = &mut self.frame {
            frame.textured_pipeline = textured;
        }
    }

    fn bind_texture(&mut self, handle: TextureHandle) {
        // Select the base-color texture for subsequent textured draws. Recorded
        // into each draw and bound (with the trilinear sampler) at fragment slot 0
        // — the textured shader's `[[texture(0)]]` + `[[sampler(0)]]` — when
        // `submit` replays the draw. An unknown handle keeps the current texture.
        let Some(frame) = &mut self.frame else { return };
        if let Some(Some(tex)) = self.textures.get(handle.0 as usize) {
            frame.bound_texture = Some(tex.clone());
        }
    }

    fn draw_mesh(&mut self, mesh: MeshHandle, transform: &Transform, material: &MaterialParams) {
        self.record_draw(mesh, transform.to_matrix(), material);
    }

    fn draw_mesh_matrix(&mut self, mesh: MeshHandle, model: Mat4, material: &MaterialParams) {
        self.record_draw(mesh, model, material);
    }

    fn draw_overlay_quad(&mut self, quad: &OverlayQuad) {
        // Outside an open frame (before the first `begin_frame`, after `submit`,
        // or while a frame was skipped for want of a drawable) there is nothing
        // to draw into: ignore the quad, as `draw_mesh` ignores its draw. Recording
        // it anyway would stack it into the next real frame.
        if self.frame.is_none() {
            return;
        }
        self.record_overlay_quad(quad);
    }

    /// Close the frame and present it (see the seam contract).
    ///
    /// On an offscreen target this **waits** for the GPU, so
    /// [`read_pixels`](MetalRenderer::read_pixels) sees the frame right after.
    /// [`submit_async`](MetalRenderer::submit_async) never waits.
    fn submit(&mut self) {
        let _ = self.submit_frame(true);
    }
}

/// Extract the AppKit `NSView` pointer from a window handle.
///
/// The pointer is valid for as long as the window is live.
fn ns_view<W: HasWindowHandle>(window: &W) -> Result<cocoa_id, RenderError> {
    let handle = window
        .window_handle()
        .map_err(|e| RenderError::InvalidSurface(format!("window handle unavailable: {e}")))?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(h) => Ok(h.ns_view.as_ptr() as cocoa_id),
        _ => Err(RenderError::InvalidSurface(
            "expected an AppKit window handle".into(),
        )),
    }
}

#[cfg(test)]
mod shadow_pass_tests {
    use super::*;

    #[test]
    fn shadow_map_is_stored_not_discarded() {
        // TBDR: the scene pass samples the map after the shadow pass
        // ends, so its depth must be stored. `DontCare` (what the scene pass's own
        // MSAA depth uses) would compile, run, and sample garbage.
        assert!(matches!(
            SHADOW_MAP_STORE_ACTION,
            metal::MTLStoreAction::Store
        ));
    }

    #[test]
    fn shadow_map_is_never_memoryless() {
        // Unlike the MSAA targets (memoryless on iOS), the shadow map outlives its
        // pass, on every platform.
        assert!(!matches!(
            shadow_map_storage_mode(),
            metal::MTLStorageMode::Memoryless
        ));
    }

    #[test]
    fn colour_textures_use_an_srgb_format_and_data_textures_do_not() {
        let srgb = texture_pixel_format(ColorSpace::Srgb);
        let linear = texture_pixel_format(ColorSpace::Linear);
        assert_ne!(srgb, linear);
        assert!(matches!(
            srgb,
            MTLPixelFormat::RGBA8Unorm_sRGB | MTLPixelFormat::ASTC_4x4_sRGB
        ));
    }

    #[test]
    fn rgba8_validation_rejects_short_pixel_data_but_allows_empty() {
        let data = |width, height, rgba8: &'static [u8]| TextureData {
            width,
            height,
            rgba8,
            color_space: ColorSpace::Srgb,
        };
        assert!(
            validate_rgba8(&data(2, 2, &[])).is_ok(),
            "empty = no upload"
        );
        assert!(validate_rgba8(&data(1, 1, &[0; 4])).is_ok());
        assert!(
            validate_rgba8(&data(1, 1, &[0; 8])).is_ok(),
            "extra bytes are ignored"
        );
        let short = validate_rgba8(&data(1024, 1024, &[0; 4]));
        assert!(
            matches!(short, Err(RenderError::InvalidTexture(_))),
            "{short:?}"
        );
    }

    #[test]
    fn only_the_two_built_in_layouts_map_to_a_pipeline() {
        assert_eq!(
            mesh_pipeline_for_layout(&phong_vertex_layout()),
            Some(false)
        );
        assert_eq!(
            mesh_pipeline_for_layout(&textured_vertex_layout()),
            Some(true)
        );
        assert_eq!(mesh_pipeline_for_layout(&VertexLayout::default()), None);
        let mut wrong_stride = textured_vertex_layout();
        wrong_stride.stride = 36;
        assert_eq!(
            mesh_pipeline_for_layout(&wrong_stride),
            None,
            "a textured layout at the untextured stride must not pass"
        );
    }

    #[test]
    fn caster_layouts_keep_only_the_position_at_the_mesh_stride() {
        for layout in [phong_vertex_layout(), textured_vertex_layout()] {
            let caster = shadow_caster_layout(&layout);
            assert_eq!(caster.stride, layout.stride);
            assert_eq!(caster.attributes.len(), 1);
            assert_eq!(caster.attributes[0].location, 0);
            assert_eq!(caster.attributes[0].offset, 0);
            assert_eq!(caster.attributes[0].format, VertexFormat::Float32x3);
        }
    }
}
