// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! What the C layer needs from a renderer, and the GPU-less [`NullBackend`].
//!
//! The C layer records a frame CPU-side and hands it over whole at submit
//! ([`FrameData`]), so a backend only has to replay it. Inputs reaching a
//! backend are already validated by the C layer.

use stickshift_math::glam::{Mat4, Vec3};
use stickshift_render_api::{
    ColorSpace, MeshHandle, OverlayQuad, PipelineKind, RenderSettings, SunSky, TextureHandle,
};

use crate::core::Error;

/// One 3D draw, in backend handles.
// Only the Metal backend reads a frame's contents; the null backend counts it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawCall {
    pub(crate) kind: PipelineKind,
    pub(crate) mesh: MeshHandle,
    pub(crate) model: Mat4,
    pub(crate) base_color: [f32; 4],
    /// The base-colour texture for a textured draw; `None` draws it white.
    pub(crate) texture: Option<TextureHandle>,
}

/// A whole frame, ready to encode.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct FrameData<'a> {
    pub(crate) view_projection: Mat4,
    pub(crate) camera_position: Vec3,
    pub(crate) sun: SunSky,
    pub(crate) draws: &'a [DrawCall],
    pub(crate) quads: &'a [OverlayQuad],
    /// Copy the frame's pixels out for the host (offscreen targets only).
    pub(crate) readback: bool,
}

/// A frame the GPU has been given.
pub(crate) struct Submitted {
    /// The frame's 1-based number.
    pub(crate) frame: u64,
    /// The pending pixel copy, if one was asked for.
    pub(crate) readback: Option<Box<dyn PendingReadback>>,
}

/// A pixel copy that completes when the GPU finishes its frame.
pub(crate) trait PendingReadback {
    fn is_complete(&self) -> bool;
    /// Copy into `dst` (at least the frame's `width * height * 4` bytes).
    fn copy_to(&self, dst: &mut [u8]) -> bool;
}

/// A renderer the C layer can drive.
pub(crate) trait Backend {
    fn device_name(&self) -> String;
    /// Whether the target is an offscreen texture (fixed size, readable).
    fn is_offscreen(&self) -> bool;
    fn create_mesh(
        &mut self,
        kind: PipelineKind,
        vertices: &[u8],
        indices: &[u32],
    ) -> Result<MeshHandle, Error>;
    fn destroy_mesh(&mut self, mesh: MeshHandle);
    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba8: &[u8],
        color_space: ColorSpace,
    ) -> Result<TextureHandle, Error>;
    fn destroy_texture(&mut self, texture: TextureHandle);
    fn surface_size(&self) -> (u32, u32);
    fn resize(&mut self, width: u32, height: u32, scale: f32);
    fn settings(&self) -> RenderSettings;
    fn set_settings(&mut self, settings: &RenderSettings);
    /// Frames handed to the GPU and not finished yet.
    fn frames_in_flight(&self) -> u64;
    fn submitted_frames(&self) -> u64;
    fn completed_frames(&self) -> u64;
    /// Encode and commit `frame` without waiting for the GPU, or `None` if the
    /// frame had to be skipped (no free frame slot, no drawable).
    fn submit(&mut self, frame: &FrameData<'_>) -> Option<Submitted>;
}

/// No GPU: handles are counters, frames complete as they are submitted, and
/// read-backs are zeros.
pub(crate) struct NullBackend {
    size: (u32, u32),
    settings: RenderSettings,
    next_mesh: u32,
    next_texture: u32,
    submitted: u64,
}

impl NullBackend {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            size: (width, height),
            settings: RenderSettings::default(),
            next_mesh: 0,
            next_texture: 0,
            submitted: 0,
        }
    }
}

/// The null backend's read-back: complete at once, all zeros.
struct ZeroReadback;

impl PendingReadback for ZeroReadback {
    fn is_complete(&self) -> bool {
        true
    }

    fn copy_to(&self, dst: &mut [u8]) -> bool {
        dst.fill(0);
        true
    }
}

impl Backend for NullBackend {
    fn device_name(&self) -> String {
        "Stick Shift Engine null backend".into()
    }

    fn is_offscreen(&self) -> bool {
        true
    }

    fn create_mesh(&mut self, _: PipelineKind, _: &[u8], _: &[u32]) -> Result<MeshHandle, Error> {
        self.next_mesh += 1;
        Ok(MeshHandle(self.next_mesh))
    }

    fn destroy_mesh(&mut self, _: MeshHandle) {}

    fn create_texture(
        &mut self,
        _: u32,
        _: u32,
        _: &[u8],
        _: ColorSpace,
    ) -> Result<TextureHandle, Error> {
        self.next_texture += 1;
        Ok(TextureHandle(self.next_texture))
    }

    fn destroy_texture(&mut self, _: TextureHandle) {}

    fn surface_size(&self) -> (u32, u32) {
        self.size
    }

    fn resize(&mut self, width: u32, height: u32, _: f32) {
        self.size = (width, height);
    }

    fn settings(&self) -> RenderSettings {
        self.settings
    }

    fn set_settings(&mut self, settings: &RenderSettings) {
        self.settings = *settings;
    }

    fn frames_in_flight(&self) -> u64 {
        0
    }

    fn submitted_frames(&self) -> u64 {
        self.submitted
    }

    fn completed_frames(&self) -> u64 {
        self.submitted
    }

    fn submit(&mut self, frame: &FrameData<'_>) -> Option<Submitted> {
        self.submitted += 1;
        Some(Submitted {
            frame: self.submitted,
            readback: frame
                .readback
                .then(|| Box::new(ZeroReadback) as Box<dyn PendingReadback>),
        })
    }
}
