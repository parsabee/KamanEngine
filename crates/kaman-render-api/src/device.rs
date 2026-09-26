// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`RenderDevice`] trait — load-time resource creation and destruction.

use crate::descriptor::VertexLayout;
use crate::handles::{MeshHandle, PipelineHandle, TextureHandle};

/// Plain-data description of a mesh to upload.
///
/// Vertex bytes are supplied opaquely (`vertices`) alongside the [`VertexLayout`]
/// that interprets them; indices are 32-bit. Keeping the vertex payload as raw
/// bytes lets a caller pack any layout without this crate knowing the shape of a
/// vertex. `vertices.len()` should be a whole multiple of `layout.stride`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MeshData<'a> {
    /// Tightly-packed vertex bytes, interpreted according to `layout`.
    pub vertices: &'a [u8],
    /// 32-bit triangle indices into the vertex array.
    pub indices: &'a [u32],
    /// Memory layout of a single vertex within `vertices`.
    pub layout: VertexLayout,
}

/// Plain-data description of a 2D RGBA texture to upload.
///
/// Pixels are 8-bit RGBA (`4 * width * height` bytes, row-major, top-left origin).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureData<'a> {
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Row-major 8-bit RGBA pixel bytes (`4 * width * height` bytes).
    pub rgba8: &'a [u8],
}

/// Plain-data description of a render pipeline to compile.
///
/// Names the vertex/fragment shader functions (backend resolves them from its
/// compiled shader library) and the [`VertexLayout`] the pipeline expects. No
/// backend objects appear here — the device turns this into its native pipeline
/// state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PipelineDescriptor {
    /// Name of the vertex shader entry point in the backend's shader library.
    pub vertex_shader: String,
    /// Name of the fragment shader entry point in the backend's shader library.
    pub fragment_shader: String,
    /// Vertex layout the pipeline binds against.
    pub vertex_layout: VertexLayout,
}

/// Load-time GPU-resource ownership: create and destroy meshes, textures and
/// pipelines.
///
/// A `RenderDevice` is the resource *owner* below the render seam. Callers hand it
/// plain-data descriptors and receive opaque handles ([`MeshHandle`],
/// [`TextureHandle`], [`PipelineHandle`]) in return; the underlying GPU objects
/// (Metal buffers, textures, pipeline states) never cross the seam. This trait is
/// the "no Metal above this line" firewall for *resource* operations; per-frame
/// recording lives in [`FrameRecorder`](crate::FrameRecorder).
///
/// # General contract
///
/// - **Handle validity.** A handle returned by a `create_*` method is valid until
///   the matching `destroy_*` is called for it. Using a destroyed handle (in a
///   later create returning the same integer, or in a draw) is a caller error;
///   backends may reuse the underlying slot, so a stale handle must never be
///   assumed to still name its original resource.
/// - **Uniqueness.** While a resource is live, its handle is unique among live
///   resources of the same kind. Handles are *not* guaranteed unique across kinds
///   (a `MeshHandle(0)` and a `TextureHandle(0)` may coexist).
/// - **Ownership.** The device owns the GPU resources; handles are non-owning
///   references. Dropping the device releases everything it owns.
pub trait RenderDevice {
    /// Upload a mesh and return an opaque [`MeshHandle`] naming it.
    ///
    /// # Contract
    /// - The returned handle is valid until [`destroy_mesh`](Self::destroy_mesh) is
    ///   called with it.
    /// - The device copies whatever it needs from `data`; the borrowed slices need
    ///   not outlive the call.
    /// - `data.vertices.len()` is expected to be a whole multiple of
    ///   `data.layout.stride`; a backend may reject a mismatch.
    fn create_mesh(&mut self, data: &MeshData<'_>) -> MeshHandle;

    /// Release the mesh named by `handle`.
    ///
    /// # Contract
    /// - After this returns, `handle` is invalid and must not be passed to
    ///   [`draw_mesh`](crate::FrameRecorder::draw_mesh).
    /// - Destroying an unknown or already-destroyed handle is a caller error;
    ///   backends may ignore it or panic but must not corrupt live resources.
    fn destroy_mesh(&mut self, handle: MeshHandle);

    /// Upload a texture and return an opaque [`TextureHandle`] naming it.
    ///
    /// # Contract
    /// - The returned handle is valid until
    ///   [`destroy_texture`](Self::destroy_texture) is called with it.
    /// - The device copies whatever it needs from `data`; the borrowed pixels need
    ///   not outlive the call.
    fn create_texture(&mut self, data: &TextureData<'_>) -> TextureHandle;

    /// Release the texture named by `handle`.
    ///
    /// # Contract
    /// - After this returns, `handle` is invalid and must not be passed to
    ///   [`bind_texture`](crate::FrameRecorder::bind_texture).
    /// - Destroying an unknown or already-destroyed handle is a caller error.
    fn destroy_texture(&mut self, handle: TextureHandle);

    /// Compile a render pipeline and return an opaque [`PipelineHandle`] naming it.
    ///
    /// # Contract
    /// - The returned handle is valid until
    ///   [`destroy_pipeline`](Self::destroy_pipeline) is called with it.
    /// - The named shader functions must exist in the backend's shader library;
    ///   a backend may panic or otherwise surface an error if they do not.
    fn create_pipeline(&mut self, desc: &PipelineDescriptor) -> PipelineHandle;

    /// Release the pipeline named by `handle`.
    ///
    /// # Contract
    /// - After this returns, `handle` is invalid and must not be passed to
    ///   [`set_pipeline`](crate::FrameRecorder::set_pipeline).
    /// - Destroying an unknown or already-destroyed handle is a caller error.
    fn destroy_pipeline(&mut self, handle: PipelineHandle);

    /// The drawable's current size in pixels, `(width, height)`.
    ///
    /// This is the coordinate space the 2D overlay is laid out in (see the
    /// [`overlay`](crate::overlay) module) — a HUD positions itself against it, so
    /// it must reflect the live drawable, including resizes.
    fn surface_size(&self) -> (u32, u32);

    /// Insets, in pixels, of the region that is safe from device intrusions —
    /// notches, rounded corners, home indicators — as `[top, right, bottom, left]`.
    ///
    /// A HUD should keep anything it wants guaranteed-visible inside
    /// [`surface_size`](Self::surface_size) shrunk by these. Backends without
    /// intrusions (a plain desktop window) report zeros; the iOS path reports the
    /// real insets.
    fn safe_area_insets(&self) -> [f32; 4];
}
