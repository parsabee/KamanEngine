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
//! # Behavior preservation (KE-0102 / KE-0103)
//!
//! KE-0103 makes **mesh** buffers persistent: geometry is deindexed and uploaded
//! into an `MTLBuffer` **once** in [`create_mesh`](RenderDevice::create_mesh) and
//! kept in a generational [`Registry`], so the per-frame draw path performs **no
//! mesh allocation** — it looks the buffer up by [`MeshHandle`]. KE-0104 removes
//! the last per-frame allocation, the per-draw uniform buffer, by writing each
//! draw's MVP into a persistent **uniform ring** at a rotating, 256-byte-aligned
//! offset (see [the ring section](#uniform-ring-ke-0104)). The depth texture is
//! the one texture the prototype already cached, and that caching is preserved.
//! None of this changes any rendered pixel, so the KE-0102 pixel-hash guard
//! stays valid.
//!
//! # Uniform ring (KE-0104)
//!
//! A single persistent uniform `MTLBuffer` is allocated once in
//! [`build`](MetalRenderer). Each `draw_mesh` writes its [`Uniforms`] into the
//! next ring slot at a **256-byte-aligned offset** (the Apple GPU
//! `set_vertex_buffer` offset requirement) and binds the ring at that offset —
//! no `new_buffer*` on the hot path. Invariants:
//!
//! - **Stride/alignment:** each slot is [`UNIFORM_RING_STRIDE`] (256) bytes so
//!   every per-draw offset is a multiple of 256; a 64-byte [`Uniforms`] fits
//!   with padding.
//! - **Capacity:** the ring holds `ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT`
//!   slots; [`MAX_FRAMES_IN_FLIGHT`] is `3`. A frame that exceeds its per-frame
//!   capacity grows the ring (doubling, an allocation-time event), never a
//!   per-draw alloc in steady state.
//! - **Don't stomp an in-flight slot:** the ring is partitioned into
//!   `MAX_FRAMES_IN_FLIGHT` disjoint regions. **KE-0105** selects the region with
//!   `frame_index % MAX_FRAMES_IN_FLIGHT` (behind the frames-in-flight
//!   semaphore) so the CPU never overwrites a region the GPU is still reading.
//!   The cursor resets each `begin_frame`.
//!
//! # Frames-in-flight pacing (KE-0105)
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
//! # Allocation instrument (KR1.2)
//!
//! Every `new_buffer*` the backend issues goes through
//! [`count_new_buffer`](MetalRenderer) helpers that bump an
//! [`allocation_count`](MetalRenderer::allocation_count). Tests snapshot the
//! count around the per-frame path and assert it stays `0` for the reference
//! scene after load — with meshes persistent (KE-0103) and uniforms
//! ring-allocated (KE-0104) the per-frame delta is now **0**. The counter is
//! public so KE-0105 can reuse it for the frames-in-flight work.
//!
//! # Vertex color vs. material
//!
//! The raster shader reads color from the per-vertex data (as the prototype
//! did), so [`MaterialParams`](kaman_render_api::MaterialParams) is accepted at
//! the seam but does not currently drive the raster color; the mesh bytes carry
//! the color. This matches prototype behavior and is intentional for KE-0102.

use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use block::ConcreteBlock;
use cocoa::base::id as cocoa_id;
use cocoa::foundation::NSRect;
use core_graphics_types::geometry::CGSize;
use metal::{Device, MTLPixelFormat, MTLResourceOptions, MetalLayer};
use objc::runtime::YES;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use kaman_math::glam::Mat4;
use kaman_math::Transform;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, MeshHandle, PipelineDescriptor, PipelineHandle,
    RenderDevice, TextureData, TextureHandle, VertexFormat, VertexLayout,
};

use crate::frame_sync::FrameSemaphore;
use crate::registry::Registry;
use crate::vertex::{LightUniforms, Uniforms, Vertex, UNIFORM_RING_STRIDE};

/// The clear color of the reference scene (dark blue-grey), matching the
/// prototype. Load-bearing for the pixel hash.
const CLEAR_COLOR: (f64, f64, f64, f64) = (0.1, 0.1, 0.15, 1.0);

/// The canonical `[position_xyz, normal_xyz, color_rgb]` vertex layout the
/// built-in Phong pipeline binds against (36-byte stride, attributes at
/// 0/12/24). Meshes packed onto this layout (the ECS boxes, and imported glTF
/// geometry via `kaman-assets`) all feed the single shared pipeline.
///
/// The vertex descriptor is now built *from* a [`VertexLayout`] (KE-0402) rather
/// than a hardcoded 0/12/24 triple; for this layout the mapping is byte-identical
/// to the old hardcoded descriptor, so the pixel hash is preserved.
fn phong_vertex_layout() -> VertexLayout {
    use kaman_render_api::VertexAttribute;
    VertexLayout::new(
        mem::size_of::<Vertex>() as u32,
        vec![
            VertexAttribute {
                location: 0,
                offset: 0,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 1,
                offset: 12,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 2,
                offset: 24,
                format: VertexFormat::Float32x3,
            },
        ],
    )
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

/// Number of frames the CPU may have in flight before it must wait on the GPU.
///
/// The uniform ring is sized so each of `MAX_FRAMES_IN_FLIGHT` frames owns a
/// disjoint slice: while the GPU reads frame *N*'s slice the CPU writes frame
/// *N+1*'s. **KE-0105** uses this as the frames-in-flight semaphore's permit
/// count and to select each frame's ring region with
/// `frame_index % MAX_FRAMES_IN_FLIGHT` (see [`MetalRenderer::begin_frame`]).
/// Defined here so the ring sizing (KE-0104) and the pacing (KE-0105) share one
/// source of truth.
pub const MAX_FRAMES_IN_FLIGHT: u64 = 3;

/// Initial per-frame uniform-slot capacity of the ring, per in-flight frame.
///
/// The ring starts sized for `INITIAL_MAX_DRAWS_PER_FRAME * MAX_FRAMES_IN_FLIGHT`
/// uniform slots. If a frame issues more draws than the current capacity, the
/// ring **grows** at that draw (an allocation-time event, counted by the KR1.2
/// instrument) rather than allocating per draw — steady state stays allocation
/// free. Growth doubles the per-frame capacity so it amortizes.
const INITIAL_MAX_DRAWS_PER_FRAME: u64 = 256;

/// A persistent mesh resource: a deindexed vertex buffer, uploaded once, and its
/// vertex count. Held in the [`Registry`] for the mesh's whole lifetime.
struct MeshEntry {
    /// Deindexed vertex buffer (one [`Vertex`] per index), or `None` if empty.
    vertex_buffer: Option<metal::Buffer>,
    /// Number of vertices to draw (== index count of the source mesh).
    vertex_count: u64,
}

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
/// window with [`MetalRenderer::new`], or for headless/offscreen rendering with
/// [`MetalRenderer::new_offscreen`].
pub struct MetalRenderer {
    device: Device,
    command_queue: metal::CommandQueue,
    target: RenderTarget,

    pipeline_state: metal::RenderPipelineState,
    depth_stencil_state: metal::DepthStencilState,
    light_buffer: metal::Buffer,

    // The world → clip view-projection, pushed through the seam via
    // `set_view_projection` (KE-0205). The backend owns no camera; the
    // game/engine computes this from a `kaman_camera::Camera` and hands it across
    // the seam. Combined with each draw's model transform to form the MVP. It is
    // **sticky** — retained across frames until replaced (the engine pushes it
    // once per frame before the game records) — and defaults to identity.
    view_projection: Mat4,

    // Persistent per-draw uniform ring (KE-0104). One `MTLBuffer` allocated at
    // build time and re-written every frame at rotating, 256-byte-aligned
    // offsets, so the per-frame draw path issues **no** `new_buffer*` (KR1.2).
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
    // in-flight frame's slots are never stomped (KE-0105).
    ring_region_base: u64,

    // Frames-in-flight pacing (KE-0105). The semaphore starts with
    // `MAX_FRAMES_IN_FLIGHT` permits: `begin_frame` acquires one (blocking if 3
    // frames are queued) and each command buffer's completion handler releases
    // one. `Arc` so a completion handler running on a Metal-owned thread can hold
    // a clone. `frame_index` advances once per submitted frame and drives the
    // per-frame ring region.
    frame_semaphore: Arc<FrameSemaphore>,
    frame_index: u64,

    // Cached depth texture (preserved from the prototype's one optimization).
    depth_texture: Option<metal::Texture>,
    depth_texture_size: (u64, u64),

    // Mesh resources live in a generational registry keyed by `MeshHandle`
    // (index + generation); a freed handle is a defined error, never a silent
    // wrong-buffer draw. Textures/pipelines keep the simple index-table scheme.
    meshes: Registry<MeshEntry>,
    pipelines: Vec<Option<PipelineHandleData>>,
    textures: Vec<Option<metal::Texture>>,

    // KR1.2 allocation instrument: bumped on every `new_buffer*` the backend
    // issues. `AtomicU64` so counting works through a `&self` draw path.
    alloc_count: AtomicU64,

    // Per-frame recording state (valid between `begin_frame` and `submit`).
    frame: Option<FrameState>,
}

/// Backend-private pipeline record. In this Phase-1 port there is a single
/// built-in Phong pipeline, so pipeline handles are name-only placeholders that
/// select the shared `pipeline_state`.
struct PipelineHandleData;

/// Live state for an in-flight frame.
struct FrameState {
    command_buffer: metal::CommandBuffer,
    encoder: metal::RenderCommandEncoder,
    /// A drawable to present at submit (windowed path only).
    drawable: Option<metal::MetalDrawable>,
}

impl MetalRenderer {
    /// Construct a backend attached to a `winit` window's surface.
    ///
    /// Attaches a `CAMetalLayer` to the window's `NSView`, sizes it to the view
    /// bounds, and compiles the Phong raster pipeline. Panics if no Metal device
    /// is available or the window handle is not AppKit.
    ///
    /// # Panics
    /// Panics if `MTLCreateSystemDefaultDevice` returns nil, if the shader fails
    /// to compile, or if the window is not an AppKit window.
    #[must_use]
    pub fn new<W: HasWindowHandle>(window: &W, width: u32, height: u32) -> Self {
        let device = Device::system_default().expect("no Metal device found");
        let command_queue = device.new_command_queue();

        let layer = MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_presents_with_transaction(false);

        // Attach the layer to the window's NSView.
        unsafe {
            let view = ns_view(window);
            let bounds: NSRect = msg_send![view, bounds];
            layer.set_drawable_size(CGSize::new(bounds.size.width, bounds.size.height));
            let _: () = msg_send![view, setWantsLayer: YES];
            let layer_ptr = layer.as_ref() as *const metal::MetalLayerRef as cocoa_id;
            let _: () = msg_send![view, setLayer: layer_ptr];
        }

        let _ = (width, height);
        Self::build(device, command_queue, RenderTarget::Surface { layer })
    }

    /// Construct a headless backend that renders into an offscreen texture.
    ///
    /// Used by tests and the deterministic pixel-hash guard: there is no window
    /// or drawable; [`read_pixels`](Self::read_pixels) returns the rendered
    /// image. Returns `None` when no Metal device is available (e.g. a GPU-less
    /// CI runner), so callers can skip gracefully.
    #[must_use]
    pub fn new_offscreen(width: u32, height: u32) -> Option<Self> {
        let device = Device::system_default()?;
        let command_queue = device.new_command_queue();

        let desc = metal::TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(width as u64);
        desc.set_height(height as u64);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        // Managed/shared so the CPU can read it back after the GPU writes it.
        desc.set_storage_mode(metal::MTLStorageMode::Managed);
        let color = device.new_texture(&desc);

        Some(Self::build(
            device,
            command_queue,
            RenderTarget::Offscreen {
                color,
                width: width as u64,
                height: height as u64,
            },
        ))
    }

    /// Shared construction: compile the pipeline and the light buffer.
    fn build(device: Device, command_queue: metal::CommandQueue, target: RenderTarget) -> Self {
        // Rasterization shader library. Default: compile the MSL source at runtime
        // (no toolchain needed). With `precompiled-shaders`: load a .metallib that
        // build.rs compiled ahead of time (requires the Metal toolchain). See KE-0107.
        #[cfg(not(feature = "precompiled-shaders"))]
        let library = {
            let shader_source = include_str!("../shaders/rasterization.metal");
            device
                .new_library_with_source(shader_source, &metal::CompileOptions::new())
                .expect("failed to compile rasterization shader")
        };
        #[cfg(feature = "precompiled-shaders")]
        let library = {
            // KAMAN_RASTER_METALLIB is set by build.rs to the compiled library path.
            let metallib: &[u8] = include_bytes!(env!("KAMAN_RASTER_METALLIB"));
            device
                .new_library_with_data(metallib)
                .expect("failed to load precompiled rasterization.metallib")
        };

        let vertex_function = library
            .get_function("vertex_main", None)
            .expect("vertex_main not found");
        let fragment_function = library
            .get_function("fragment_main", None)
            .expect("fragment_main not found");

        // Vertex descriptor built FROM a `VertexLayout` (KE-0402) rather than a
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

        let pipeline_state = device
            .new_render_pipeline_state(&pipeline_descriptor)
            .expect("failed to create render pipeline state");

        let depth_stencil_descriptor = metal::DepthStencilDescriptor::new();
        depth_stencil_descriptor.set_depth_compare_function(metal::MTLCompareFunction::Less);
        depth_stencil_descriptor.set_depth_write_enabled(true);
        let depth_stencil_state = device.new_depth_stencil_state(&depth_stencil_descriptor);

        let light = LightUniforms::default();
        let light_buffer = device.new_buffer_with_data(
            &light as *const LightUniforms as *const _,
            mem::size_of::<LightUniforms>() as u64,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );

        // Persistent uniform ring: one buffer covering all in-flight frames.
        let ring_draws_per_frame = INITIAL_MAX_DRAWS_PER_FRAME;
        let ring_len = ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE;
        let uniform_ring =
            device.new_buffer(ring_len, MTLResourceOptions::CPUCacheModeDefaultCache);

        Self {
            device,
            command_queue,
            target,
            pipeline_state,
            depth_stencil_state,
            light_buffer,
            // No camera pushed yet; identity until the first `set_view_projection`.
            view_projection: Mat4::IDENTITY,
            uniform_ring,
            ring_draws_per_frame,
            ring_cursor: 0,
            ring_region_base: 0,
            // Triple-buffer pacing: start with a full complement of permits.
            frame_semaphore: Arc::new(FrameSemaphore::new(MAX_FRAMES_IN_FLIGHT as u32)),
            frame_index: 0,
            depth_texture: None,
            depth_texture_size: (0, 0),
            meshes: Registry::new(),
            pipelines: Vec::new(),
            textures: Vec::new(),
            // Two construction-time allocations: the `light_buffer` and the
            // persistent `uniform_ring`. Both are load-time, off the hot path.
            alloc_count: AtomicU64::new(2),
            frame: None,
        }
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
        color.get_bytes(
            data.as_mut_ptr() as *mut _,
            bytes_per_row,
            region,
            0,
        );
        Some(data)
    }

    /// Build (or reuse) the depth texture for the current attachment size.
    ///
    /// Preserves the prototype's single caching optimization: the depth texture
    /// is only reallocated when the target size changes.
    fn depth_texture_for(&mut self, width: u64, height: u64) -> metal::Texture {
        if let Some(tex) = &self.depth_texture {
            if self.depth_texture_size == (width, height) {
                return tex.clone();
            }
        }
        let desc = metal::TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::Depth32Float);
        desc.set_width(width);
        desc.set_height(height);
        desc.set_usage(metal::MTLTextureUsage::RenderTarget);
        desc.set_storage_mode(metal::MTLStorageMode::Private);
        let tex = self.device.new_texture(&desc);
        self.depth_texture = Some(tex.clone());
        self.depth_texture_size = (width, height);
        tex
    }

    /// Total number of `new_buffer*` allocations the backend has issued since
    /// construction (KR1.2 instrument).
    ///
    /// Includes the construction-time light-uniform buffer and uniform ring, and
    /// every mesh upload. Tests snapshot this around the per-frame path
    /// (`begin_frame` … `submit`) and assert the delta is `0` for the reference
    /// scene after load: meshes are persistent (KE-0103) and per-draw uniforms
    /// come from the ring (KE-0104), so the hot path allocates nothing. Exposed
    /// for reuse by KE-0105 (frames-in-flight).
    #[must_use]
    pub fn allocation_count(&self) -> u64 {
        self.alloc_count.load(Ordering::Relaxed)
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
        // Deindex at the byte level using the layout's declared per-vertex stride
        // (KE-0402), so any layout the seam describes is uploaded correctly — not
        // just the 36-byte `Vertex`. For the `[pos,normal,color]` box layout the
        // stride is `size_of::<Vertex>()`, so the resulting bytes are identical to
        // the prior `Vertex`-typed path (pixel hash unchanged).
        let stride = data.layout.stride as usize;
        if stride == 0 || data.indices.is_empty() || data.vertices.len() < stride {
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
            };
        }
        let vertex_count_in = data.vertices.len() / stride;

        // Expand the index buffer into a flat, deindexed byte buffer: one
        // `stride`-byte vertex record per index, copied verbatim.
        let mut expanded: Vec<u8> = Vec::with_capacity(data.indices.len() * stride);
        for &index in data.indices {
            let start = index as usize * stride;
            if index as usize >= vertex_count_in {
                // A malformed index would read out of bounds; skip defensively
                // rather than panic on the load path.
                continue;
            }
            expanded.extend_from_slice(&data.vertices[start..start + stride]);
        }
        if expanded.is_empty() {
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
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
        }
    }

    /// Write `uniforms` into the next slot of the uniform ring and return the
    /// **256-byte-aligned byte offset** to bind at.
    ///
    /// This is the KE-0104 replacement for the prototype's per-draw
    /// `new_buffer`: the ring is a single persistent buffer, so the steady-state
    /// per-frame path performs **no allocation** (KR1.2). Sub-allocation rules:
    ///
    /// - The offset is `(ring_region_base + ring_cursor) * UNIFORM_RING_STRIDE`,
    ///   always a multiple of 256 (the Apple GPU `set_vertex_buffer` offset
    ///   requirement), so the caller can bind the ring at that offset directly.
    /// - `ring_cursor` advances one slot per draw and resets in `begin_frame`;
    ///   `ring_region_base` selects this frame's disjoint region so an in-flight
    ///   frame's slots are never overwritten (KE-0105 sets it via
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

    /// Double the per-frame ring capacity and reallocate the backing buffer.
    ///
    /// Called only from [`write_uniform_to_ring`](Self::write_uniform_to_ring)
    /// when a frame's draw count exceeds the current per-frame capacity — an
    /// amortized, allocation-time event, not a per-draw one. Uniforms already
    /// written this frame are re-issued by the caller on the next draw path, so
    /// the old contents need not be copied; the cursor is kept.
    fn grow_ring(&mut self) {
        self.ring_draws_per_frame *= 2;
        let ring_len = self.ring_draws_per_frame * MAX_FRAMES_IN_FLIGHT * UNIFORM_RING_STRIDE;
        self.uniform_ring =
            self.counted_new_buffer(ring_len, MTLResourceOptions::CPUCacheModeDefaultCache);
        // The per-frame stride just changed, so the region base must be
        // recomputed against the *live* frame index — not hardcoded to 0, which
        // would put this frame's writes in region 0 while the GPU may still be
        // reading region 0 from an earlier in-flight frame (KE-0105).
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

    /// The per-slot byte stride of the uniform ring (256-byte aligned). Test aid.
    #[must_use]
    pub fn ring_stride_for_test(&self) -> u64 {
        UNIFORM_RING_STRIDE
    }

    /// The current frame's ring-region **base slot index** (KE-0105). After
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

    /// The number of frames submitted so far (KE-0105 `frame_index`). Test aid.
    #[must_use]
    pub fn frame_index_for_test(&self) -> u64 {
        self.frame_index
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

    fn create_texture(&mut self, data: &TextureData<'_>) -> TextureHandle {
        let desc = metal::TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
        desc.set_width(data.width as u64);
        desc.set_height(data.height as u64);
        let tex = self.device.new_texture(&desc);
        if !data.rgba8.is_empty() {
            let region = metal::MTLRegion {
                origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
                size: metal::MTLSize {
                    width: data.width as u64,
                    height: data.height as u64,
                    depth: 1,
                },
            };
            tex.replace_region(
                region,
                0,
                data.rgba8.as_ptr() as *const _,
                (data.width * 4) as u64,
            );
        }
        let id = self.textures.len() as u32;
        self.textures.push(Some(tex));
        TextureHandle(id)
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        if let Some(slot) = self.textures.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }

    fn create_pipeline(&mut self, _desc: &PipelineDescriptor) -> PipelineHandle {
        // Phase-1 port: a single built-in Phong pipeline. The handle names it;
        // the actual `RenderPipelineState` is the shared `self.pipeline_state`.
        let id = self.pipelines.len() as u32;
        self.pipelines.push(Some(PipelineHandleData));
        PipelineHandle(id)
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        if let Some(slot) = self.pipelines.get_mut(handle.0 as usize) {
            *slot = None;
        }
    }
}

impl FrameRecorder for MetalRenderer {
    fn begin_frame(&mut self) {
        // Determine the color attachment texture and (optionally) a drawable.
        let (color_texture, drawable, width, height) = match &self.target {
            RenderTarget::Surface { layer } => match layer.next_drawable() {
                Some(drawable) => {
                    let tex = drawable.texture().to_owned();
                    let w = tex.width();
                    let h = tex.height();
                    (tex, Some(drawable.to_owned()), w, h)
                }
                // No drawable available (e.g. minimized): skip this frame.
                None => return,
            },
            RenderTarget::Offscreen {
                color,
                width,
                height,
            } => (color.clone(), None, *width, *height),
        };

        // Frames-in-flight gate (KE-0105): block until at most
        // `MAX_FRAMES_IN_FLIGHT - 1` frames are still queued on the GPU, so this
        // frame can be prepared without stomping a ring region the GPU is still
        // reading. Acquired here — *after* the drawable/target is secured — so a
        // skipped frame (no drawable) never leaks a permit (it also never
        // registers a completion handler to release one).
        self.frame_semaphore.acquire();

        // NOTE: the view-projection is deliberately NOT reset here. It is sticky
        // across frames (seam contract, KE-0205): the engine loop pushes it once
        // per frame via `set_view_projection` *before* the game opens its frame,
        // so clearing it in `begin_frame` would wipe the engine's camera before
        // the first draw. It defaults to identity until the first push.

        let command_buffer = self.command_queue.new_command_buffer().to_owned();
        let render_pass_descriptor = metal::RenderPassDescriptor::new();

        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_texture(Some(&color_texture));
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(
            CLEAR_COLOR.0,
            CLEAR_COLOR.1,
            CLEAR_COLOR.2,
            CLEAR_COLOR.3,
        ));
        color_attachment.set_store_action(metal::MTLStoreAction::Store);

        let depth_texture = self.depth_texture_for(width, height);
        let depth_attachment = render_pass_descriptor.depth_attachment().unwrap();
        depth_attachment.set_texture(Some(&depth_texture));
        depth_attachment.set_load_action(metal::MTLLoadAction::Clear);
        depth_attachment.set_clear_depth(1.0);
        depth_attachment.set_store_action(metal::MTLStoreAction::DontCare);

        let encoder = command_buffer
            .new_render_command_encoder(render_pass_descriptor)
            .to_owned();
        encoder.set_render_pipeline_state(&self.pipeline_state);
        encoder.set_depth_stencil_state(&self.depth_stencil_state);

        // Reset the uniform-ring cursor and select this frame's disjoint region
        // (KE-0105): `(frame_index % MAX_FRAMES_IN_FLIGHT) * ring_draws_per_frame`
        // cycles the region base `0,1,2,0,…`. Behind the semaphore acquired
        // above, so the region the CPU is about to write is guaranteed not to be
        // one the GPU is still reading.
        self.ring_cursor = 0;
        self.ring_region_base =
            (self.frame_index % MAX_FRAMES_IN_FLIGHT) * self.ring_draws_per_frame;

        self.frame = Some(FrameState {
            command_buffer,
            encoder,
            drawable,
        });
    }

    fn set_view_projection(&mut self, view_proj: Mat4) {
        // Store this frame's world → clip matrix; `draw_mesh` multiplies it by
        // each instance's model transform to form the MVP (KE-0205). Per the seam
        // contract this is pushed once per frame before the first `draw_mesh`.
        self.view_projection = view_proj;
    }

    fn set_pipeline(&mut self, _handle: PipelineHandle) {
        // Single built-in pipeline; already bound in `begin_frame`. This exists
        // to honor the seam protocol (a pipeline must be selected before draws).
    }

    fn bind_texture(&mut self, handle: TextureHandle) {
        if let Some(frame) = &self.frame {
            if let Some(Some(tex)) = self.textures.get(handle.0 as usize) {
                frame.encoder.set_fragment_texture(0, Some(tex));
            }
        }
    }

    fn draw_mesh(&mut self, mesh: MeshHandle, transform: &Transform, _material: &MaterialParams) {
        if self.frame.is_none() {
            return;
        }
        // Look the persistent vertex buffer up by handle — no allocation here.
        // A stale/freed or unknown handle is a defined no-op (the registry
        // returns an error), never a silent wrong-buffer draw. Clone the small
        // buffer handle + count so we can drop the immutable `meshes` borrow
        // before taking the `&mut self` uniform-ring write.
        let Ok(entry) = self.meshes.get(mesh) else {
            return;
        };
        let Some(vertex_buffer) = entry.vertex_buffer.clone() else {
            return;
        };
        let vertex_count = entry.vertex_count;
        if vertex_count == 0 {
            return;
        }

        // Per-draw MVP uniform: written into the persistent uniform ring at a
        // rotating, 256-byte-aligned offset — **no `new_buffer*` on the hot
        // path** (KR1.2). KE-0104 replaced the prototype's per-draw allocation
        // with this ring sub-allocation.
        let model: Mat4 = transform.to_matrix();
        let mvp = self.view_projection * model;
        let uniforms = Uniforms {
            model_view_projection: mvp.to_cols_array_2d(),
        };
        let uniform_offset = self.write_uniform_to_ring(&uniforms);

        let frame = self.frame.as_ref().expect("frame checked Some above");
        let encoder = &frame.encoder;
        encoder.set_vertex_buffer(0, Some(&vertex_buffer), 0);
        encoder.set_vertex_buffer(1, Some(&self.uniform_ring), uniform_offset);
        encoder.set_fragment_buffer(0, Some(&self.light_buffer), 0);
        encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, vertex_count);
    }

    fn submit(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        frame.encoder.end_encoding();

        // Frames-in-flight release (KE-0105): register a completion handler that
        // signals the semaphore when the GPU finishes this frame. The handler
        // runs on a Metal-owned thread, so it must be allocation-free — it holds
        // an `Arc<FrameSemaphore>` (cloned here on the CPU submit path, not in
        // the handler) and does nothing but `release()`, which is a lock +
        // counter bump + condvar notify (no allocation). The permit released
        // here is the one `begin_frame` acquired for this frame.
        let semaphore = Arc::clone(&self.frame_semaphore);
        let completion = ConcreteBlock::new(move |_cb: &metal::CommandBufferRef| {
            semaphore.release();
        })
        .copy();
        frame.command_buffer.add_completed_handler(&completion);

        // This frame is now fully recorded and about to be committed: advance the
        // frame index so the *next* `begin_frame` rotates to the next ring region
        // (`0,1,2,0,…`).
        self.frame_index = self.frame_index.wrapping_add(1);

        match &frame.drawable {
            Some(drawable) => {
                // Present is tied to the drawable; pacing is the semaphore, so
                // there is no busy-wait/spin here.
                frame.command_buffer.present_drawable(drawable);
                frame.command_buffer.commit();
            }
            None => {
                // Offscreen: for a Managed texture, synchronize to CPU so the
                // readback sees the rendered pixels, then wait for completion.
                // The completion handler (and its `release`) still fires on the
                // wait, keeping the semaphore balanced on the offscreen path.
                if let RenderTarget::Offscreen { color, .. } = &self.target {
                    let blit = frame.command_buffer.new_blit_command_encoder();
                    blit.synchronize_resource(color);
                    blit.end_encoding();
                }
                frame.command_buffer.commit();
                frame.command_buffer.wait_until_completed();
            }
        }
    }
}

/// Extract the AppKit `NSView` pointer from a window handle.
///
/// # Safety
/// The window must be a live AppKit window for the returned pointer to be valid.
unsafe fn ns_view<W: HasWindowHandle>(window: &W) -> cocoa_id {
    let handle = window.window_handle().expect("window handle unavailable");
    match handle.as_raw() {
        RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as cocoa_id,
        _ => panic!("expected an AppKit window handle"),
    }
}
