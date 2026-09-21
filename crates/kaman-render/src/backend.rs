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
//! mesh allocation** — it looks the buffer up by [`MeshHandle`]. The one
//! remaining per-frame allocation is the per-draw uniform buffer, kept
//! deliberately here exactly as the prototype did (KE-0104 replaces it with a
//! ring). The depth texture is the one texture the prototype already cached, and
//! that caching is preserved. Making mesh upload persistent does not change any
//! rendered pixel, so the KE-0102 pixel-hash guard stays valid.
//!
//! # Allocation instrument (KR1.2)
//!
//! Every `new_buffer*` the backend issues goes through
//! [`count_new_buffer`](MetalRenderer) helpers that bump an
//! [`allocation_count`](MetalRenderer::allocation_count). Tests snapshot the
//! count around the per-frame path and assert it stays `0` for the reference
//! scene after load, proving the mesh-allocation removal. The counter is public
//! so KE-0104/0105 can reuse it for the uniform-ring and frames-in-flight work.
//!
//! # Vertex color vs. material
//!
//! The raster shader reads color from the per-vertex data (as the prototype
//! did), so [`MaterialParams`](kaman_render_api::MaterialParams) is accepted at
//! the seam but does not currently drive the raster color; the mesh bytes carry
//! the color. This matches prototype behavior and is intentional for KE-0102.

use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};

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
    RenderDevice, TextureData, TextureHandle,
};

use crate::camera::Camera;
use crate::registry::Registry;
use crate::vertex::{LightUniforms, Uniforms, Vertex};

/// The clear color of the reference scene (dark blue-grey), matching the
/// prototype. Load-bearing for the pixel hash.
const CLEAR_COLOR: (f64, f64, f64, f64) = (0.1, 0.1, 0.15, 1.0);

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
    camera: Camera,

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

        let aspect = width as f32 / height.max(1) as f32;
        Self::build(device, command_queue, RenderTarget::Surface { layer }, aspect)
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

        let aspect = width as f32 / height.max(1) as f32;
        Some(Self::build(
            device,
            command_queue,
            RenderTarget::Offscreen {
                color,
                width: width as u64,
                height: height as u64,
            },
            aspect,
        ))
    }

    /// Shared construction: compile the pipeline, light buffer, and camera.
    fn build(
        device: Device,
        command_queue: metal::CommandQueue,
        target: RenderTarget,
        aspect_ratio: f32,
    ) -> Self {
        // Compile the rasterization shader at runtime (the .metallib precompile
        // is KE-0107).
        let shader_source = include_str!("../shaders/rasterization.metal");
        let library = device
            .new_library_with_source(shader_source, &metal::CompileOptions::new())
            .expect("failed to compile rasterization shader");

        let vertex_function = library
            .get_function("vertex_main", None)
            .expect("vertex_main not found");
        let fragment_function = library
            .get_function("fragment_main", None)
            .expect("fragment_main not found");

        // Vertex descriptor: interleaved position/normal/color at 0/12/24.
        let vertex_descriptor = metal::VertexDescriptor::new();
        for (index, offset) in [(0u64, 0u64), (1, 12), (2, 24)] {
            let attr = vertex_descriptor.attributes().object_at(index).unwrap();
            attr.set_format(metal::MTLVertexFormat::Float3);
            attr.set_offset(offset);
            attr.set_buffer_index(0);
        }
        let layout = vertex_descriptor.layouts().object_at(0).unwrap();
        layout.set_stride(mem::size_of::<Vertex>() as u64);
        layout.set_step_function(metal::MTLVertexStepFunction::PerVertex);

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

        let camera = Camera::new(aspect_ratio);

        Self {
            device,
            command_queue,
            target,
            pipeline_state,
            depth_stencil_state,
            light_buffer,
            camera,
            depth_texture: None,
            depth_texture_size: (0, 0),
            meshes: Registry::new(),
            pipelines: Vec::new(),
            textures: Vec::new(),
            // The `light_buffer` above is the one construction-time allocation.
            alloc_count: AtomicU64::new(1),
            frame: None,
        }
    }

    /// Mutable access to the backend camera (view/projection).
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
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
    /// Includes the one construction-time light-uniform buffer and every mesh
    /// upload and per-draw uniform buffer. Tests snapshot this around the
    /// per-frame path (`begin_frame` … `submit`) and assert the delta is `0` for
    /// the reference scene after load, proving no mesh allocation happens on the
    /// hot path. Exposed for reuse by KE-0104 (uniform ring) and KE-0105
    /// (frames-in-flight).
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
        // Interpret the seam's raw vertex bytes as `Vertex` records.
        let stride = mem::size_of::<Vertex>();
        if data.indices.is_empty() || data.vertices.len() < stride {
            return MeshEntry {
                vertex_buffer: None,
                vertex_count: 0,
            };
        }
        let vertex_count_in = data.vertices.len() / stride;
        // SAFETY: `Vertex` is `#[repr(C)]` plain-old-data of `3 * [f32;3]`; the
        // seam contract says `vertices.len()` is a whole multiple of the layout
        // stride, which for this backend is `size_of::<Vertex>()`.
        let src: &[Vertex] = unsafe {
            std::slice::from_raw_parts(data.vertices.as_ptr() as *const Vertex, vertex_count_in)
        };

        let mut expanded: Vec<Vertex> = Vec::with_capacity(data.indices.len());
        for &index in data.indices {
            expanded.push(src[index as usize]);
        }

        // SAFETY: `expanded` is a live `Vec<Vertex>`; the pointer and byte length
        // describe exactly its contents.
        let buffer = unsafe {
            self.counted_new_buffer_with_data(
                expanded.as_ptr() as *const _,
                (expanded.len() * mem::size_of::<Vertex>()) as u64,
                MTLResourceOptions::CPUCacheModeDefaultCache,
            )
        };
        MeshEntry {
            vertex_buffer: Some(buffer),
            vertex_count: expanded.len() as u64,
        }
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

        self.camera.set_aspect_ratio(width as f32 / height.max(1) as f32);

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

        self.frame = Some(FrameState {
            command_buffer,
            encoder,
            drawable,
        });
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
        let Some(frame) = &self.frame else {
            return;
        };
        // Look the persistent vertex buffer up by handle — no allocation here.
        // A stale/freed or unknown handle is a defined no-op (the registry
        // returns an error), never a silent wrong-buffer draw.
        let Ok(entry) = self.meshes.get(mesh) else {
            return;
        };
        let Some(vertex_buffer) = &entry.vertex_buffer else {
            return;
        };
        if entry.vertex_count == 0 {
            return;
        }

        // Per-draw MVP uniform buffer. This is the one remaining per-frame
        // allocation, kept as the prototype had it; KE-0104 replaces it with a
        // ring. It is counted by the KR1.2 instrument.
        let model: Mat4 = transform.to_matrix();
        let mvp = self.camera.view_projection_matrix() * model;
        let uniforms = Uniforms {
            model_view_projection: mvp.to_cols_array_2d(),
        };
        let uniform_buffer = self.counted_new_buffer(
            mem::size_of::<Uniforms>() as u64,
            MTLResourceOptions::CPUCacheModeDefaultCache,
        );
        // SAFETY: buffer is exactly `size_of::<Uniforms>()` and CPU-visible.
        unsafe {
            std::ptr::write(uniform_buffer.contents() as *mut Uniforms, uniforms);
        }

        let encoder = &frame.encoder;
        encoder.set_vertex_buffer(0, Some(vertex_buffer), 0);
        encoder.set_vertex_buffer(1, Some(&uniform_buffer), 0);
        encoder.set_fragment_buffer(0, Some(&self.light_buffer), 0);
        encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, entry.vertex_count);
    }

    fn submit(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        frame.encoder.end_encoding();
        match &frame.drawable {
            Some(drawable) => {
                frame.command_buffer.present_drawable(drawable);
                frame.command_buffer.commit();
            }
            None => {
                // Offscreen: for a Managed texture, synchronize to CPU so the
                // readback sees the rendered pixels, then wait for completion.
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
