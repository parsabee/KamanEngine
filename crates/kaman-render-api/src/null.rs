// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`NullRenderer`] — a headless, GPU-free test double for the render seam.

use kaman_math::glam::{Mat4, Vec3};
use kaman_math::Transform;

use crate::descriptor::MaterialParams;
use crate::device::{MeshData, PipelineDescriptor, RenderDevice, TextureData};
use crate::handles::{MeshHandle, PipelineHandle, TextureHandle};
use crate::overlay::OverlayQuad;
use crate::recorder::FrameRecorder;
use crate::sun::SunSky;

/// A single recorded draw, captured by [`NullRenderer`] for later assertion.
///
/// Not `PartialEq`: [`kaman_math::Transform`] does not implement `PartialEq`, so tests
/// assert on individual fields (`mesh`, `pipeline`, `texture`, `material`,
/// `transform.position`, …) rather than comparing whole draws.
#[derive(Debug, Clone)]
pub struct RecordedDraw {
    /// The mesh that was drawn.
    pub mesh: MeshHandle,
    /// The pipeline bound when the draw was recorded, if any had been set.
    pub pipeline: Option<PipelineHandle>,
    /// The texture bound when the draw was recorded, if any.
    pub texture: Option<TextureHandle>,
    /// The instance transform passed to the draw.
    pub transform: Transform,
    /// The material parameters passed to the draw.
    pub material: MaterialParams,
}

/// A headless implementation of both [`RenderDevice`] and [`FrameRecorder`] that
/// **records** every call instead of touching a GPU.
///
/// `NullRenderer` lets scene and game code be unit-tested without a Metal device:
/// resource creation hands out sequential handles, and frame recording captures the
/// full ordered stream of draws (each with the pipeline/texture bound at the time).
/// Tests introspect the recorder through its accessors — [`draw_count`](Self::draw_count),
/// [`draws`](Self::draws), [`created_meshes`](Self::created_meshes), and friends — to
/// assert what the code under test asked the renderer to do.
///
/// The sticky frame-wide state is recorded the same way:
/// [`view_projection`](Self::view_projection),
/// [`camera_position`](Self::camera_position) and [`sun_sky`](Self::sun_sky) each
/// hold the last value pushed, so a test can assert *which camera and which sun* a
/// game asked for — a headless way to check lighting intent that would otherwise
/// need a GPU and a pixel hash.
///
/// It also tracks live vs. destroyed resources so tests can assert correct cleanup,
/// and records the frame protocol (frames begun/submitted, whether a frame is open)
/// so tests can catch ordering mistakes. It never panics on protocol violations —
/// it records them faithfully so a test can assert the *observed* behaviour.
#[derive(Debug, Default)]
pub struct NullRenderer {
    // --- resource bookkeeping (RenderDevice side) ---
    next_mesh: u32,
    next_texture: u32,
    next_pipeline: u32,
    created_meshes: Vec<MeshHandle>,
    created_textures: Vec<TextureHandle>,
    created_pipelines: Vec<PipelineHandle>,
    destroyed_meshes: Vec<MeshHandle>,
    destroyed_textures: Vec<TextureHandle>,
    destroyed_pipelines: Vec<PipelineHandle>,

    // --- frame bookkeeping (FrameRecorder side) ---
    frame_open: bool,
    frames_begun: u32,
    frames_submitted: u32,
    current_pipeline: Option<PipelineHandle>,
    current_texture: Option<TextureHandle>,
    draws: Vec<RecordedDraw>,
    /// Overlay quads recorded this run, in record order (KE-0404), so headless
    /// tests can assert what a HUD drew without a GPU.
    overlay_quads: Vec<OverlayQuad>,
    /// The most recent view-projection pushed via
    /// [`set_view_projection`](FrameRecorder::set_view_projection), if any. `None`
    /// until the first push, then **sticky** (retained across frames) — mirroring
    /// the seam contract that a real backend keeps the last view-projection until
    /// it is replaced.
    view_projection: Option<Mat4>,
    /// The most recent camera world position pushed via
    /// [`set_camera_position`](FrameRecorder::set_camera_position) (KE-0406).
    /// `None` until the first push, then sticky — like `view_projection`, which it
    /// must agree with.
    camera_position: Option<Vec3>,
    /// The most recent sun/sky pushed via
    /// [`set_sun_sky`](FrameRecorder::set_sun_sky) (KE-0406). `None` until the first
    /// push, then sticky, so a headless test can assert *which sun* a game asked
    /// for without a GPU.
    sun_sky: Option<SunSky>,
}

impl NullRenderer {
    /// Create an empty recorder with no resources and no recorded frames.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // --- resource introspection ---

    /// Handles of every mesh created, in creation order (including any later destroyed).
    #[must_use]
    pub fn created_meshes(&self) -> &[MeshHandle] {
        &self.created_meshes
    }

    /// Handles of every texture created, in creation order.
    #[must_use]
    pub fn created_textures(&self) -> &[TextureHandle] {
        &self.created_textures
    }

    /// Handles of every pipeline created, in creation order.
    #[must_use]
    pub fn created_pipelines(&self) -> &[PipelineHandle] {
        &self.created_pipelines
    }

    /// Handles passed to [`destroy_mesh`](RenderDevice::destroy_mesh), in call order.
    #[must_use]
    pub fn destroyed_meshes(&self) -> &[MeshHandle] {
        &self.destroyed_meshes
    }

    /// Handles passed to [`destroy_texture`](RenderDevice::destroy_texture), in call order.
    #[must_use]
    pub fn destroyed_textures(&self) -> &[TextureHandle] {
        &self.destroyed_textures
    }

    /// Handles passed to [`destroy_pipeline`](RenderDevice::destroy_pipeline), in call order.
    #[must_use]
    pub fn destroyed_pipelines(&self) -> &[PipelineHandle] {
        &self.destroyed_pipelines
    }

    /// Number of meshes still live (created minus destroyed).
    #[must_use]
    pub fn live_mesh_count(&self) -> usize {
        self.created_meshes.len() - self.destroyed_meshes.len()
    }

    /// Number of textures still live (created minus destroyed).
    #[must_use]
    pub fn live_texture_count(&self) -> usize {
        self.created_textures.len() - self.destroyed_textures.len()
    }

    /// Number of pipelines still live (created minus destroyed).
    #[must_use]
    pub fn live_pipeline_count(&self) -> usize {
        self.created_pipelines.len() - self.destroyed_pipelines.len()
    }

    // --- frame introspection ---

    /// Every draw recorded across every frame, in order.
    #[must_use]
    pub fn draws(&self) -> &[RecordedDraw] {
        &self.draws
    }

    /// Every overlay quad recorded across all frames, in record order (KE-0404).
    #[must_use]
    pub fn overlay_quads(&self) -> &[OverlayQuad] {
        &self.overlay_quads
    }

    /// Total number of overlay quads recorded across all frames.
    #[must_use]
    pub fn overlay_quad_count(&self) -> usize {
        self.overlay_quads.len()
    }

    /// Total number of draws recorded across all frames.
    #[must_use]
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }

    /// Number of times [`begin_frame`](FrameRecorder::begin_frame) was called.
    #[must_use]
    pub fn frames_begun(&self) -> u32 {
        self.frames_begun
    }

    /// Number of times [`submit`](FrameRecorder::submit) was called.
    #[must_use]
    pub fn frames_submitted(&self) -> u32 {
        self.frames_submitted
    }

    /// Whether a frame is currently open (begun but not yet submitted).
    #[must_use]
    pub fn frame_open(&self) -> bool {
        self.frame_open
    }

    /// The most recent view-projection matrix recorded via
    /// [`set_view_projection`](FrameRecorder::set_view_projection).
    ///
    /// `None` before the first push, then sticky (the last value pushed). Lets
    /// tests assert the engine/game pushed the expected camera before drawing.
    #[must_use]
    pub fn view_projection(&self) -> Option<Mat4> {
        self.view_projection
    }

    /// The most recent camera world position recorded via
    /// [`set_camera_position`](FrameRecorder::set_camera_position) (KE-0406).
    ///
    /// `None` before the first push, then sticky. Pairs with
    /// [`view_projection`](Self::view_projection): a test can assert the engine
    /// pushed a position that matches the matrix it pushed.
    #[must_use]
    pub fn camera_position(&self) -> Option<Vec3> {
        self.camera_position
    }

    /// The most recent [`SunSky`] recorded via
    /// [`set_sun_sky`](FrameRecorder::set_sun_sky) (KE-0406).
    ///
    /// `None` before the first push, then sticky. Lets a headless test assert the
    /// *sun a game asked for* — its elevation, azimuth, colour and sky — and, via
    /// [`SunSky::direction`], the light vector that follows from it.
    #[must_use]
    pub fn sun_sky(&self) -> Option<SunSky> {
        self.sun_sky
    }
}

impl RenderDevice for NullRenderer {
    fn create_mesh(&mut self, _data: &MeshData<'_>) -> MeshHandle {
        let handle = MeshHandle(self.next_mesh);
        self.next_mesh += 1;
        self.created_meshes.push(handle);
        handle
    }

    fn destroy_mesh(&mut self, handle: MeshHandle) {
        self.destroyed_meshes.push(handle);
    }

    fn create_texture(&mut self, _data: &TextureData<'_>) -> TextureHandle {
        let handle = TextureHandle(self.next_texture);
        self.next_texture += 1;
        self.created_textures.push(handle);
        handle
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        self.destroyed_textures.push(handle);
    }

    fn create_pipeline(&mut self, _desc: &PipelineDescriptor) -> PipelineHandle {
        let handle = PipelineHandle(self.next_pipeline);
        self.next_pipeline += 1;
        self.created_pipelines.push(handle);
        handle
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        self.destroyed_pipelines.push(handle);
    }

    fn surface_size(&self) -> (u32, u32) {
        // A fixed, GPU-less drawable size so headless HUD layout is deterministic.
        (NULL_SURFACE_WIDTH, NULL_SURFACE_HEIGHT)
    }

    fn safe_area_insets(&self) -> [f32; 4] {
        // No notches or rounded corners off-device.
        [0.0; 4]
    }
}

/// Drawable width the [`NullRenderer`] reports from
/// [`surface_size`](RenderDevice::surface_size), so headless HUD layout is
/// deterministic.
pub const NULL_SURFACE_WIDTH: u32 = 1280;
/// Drawable height the [`NullRenderer`] reports from
/// [`surface_size`](RenderDevice::surface_size).
pub const NULL_SURFACE_HEIGHT: u32 = 720;

impl FrameRecorder for NullRenderer {
    fn begin_frame(&mut self) {
        self.frame_open = true;
        self.frames_begun += 1;
        // Per-frame state resets at the start of each frame. The camera and the
        // sun/sky are deliberately NOT reset: they are sticky across frames (seam
        // contract), so an engine loop can push them once per frame before the game
        // records — or, for a fixed sun, just once at load.
        self.current_pipeline = None;
        self.current_texture = None;
    }

    fn set_view_projection(&mut self, view_proj: Mat4) {
        self.view_projection = Some(view_proj);
    }

    fn set_camera_position(&mut self, position: Vec3) {
        self.camera_position = Some(position);
    }

    fn set_sun_sky(&mut self, sun: &SunSky) {
        self.sun_sky = Some(*sun);
    }

    fn set_pipeline(&mut self, handle: PipelineHandle) {
        self.current_pipeline = Some(handle);
    }

    fn bind_texture(&mut self, handle: TextureHandle) {
        self.current_texture = Some(handle);
    }

    fn draw_mesh(&mut self, mesh: MeshHandle, transform: &Transform, material: &MaterialParams) {
        self.draws.push(RecordedDraw {
            mesh,
            pipeline: self.current_pipeline,
            texture: self.current_texture,
            transform: *transform,
            material: *material,
        });
    }

    fn draw_overlay_quad(&mut self, quad: &OverlayQuad) {
        self.overlay_quads.push(*quad);
    }

    fn submit(&mut self) {
        self.frame_open = false;
        self.frames_submitted += 1;
    }
}
