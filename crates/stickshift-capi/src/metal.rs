// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The Metal [`Backend`]: replays a recorded frame into a [`MetalRenderer`]
//! through its non-blocking path (`try_begin_frame` / `submit_async`).

use std::ffi::c_void;

use kaman_render::{MetalRenderer, Readback, RenderError};
use kaman_render_api::{
    ColorSpace, FrameRecorder, MaterialParams, MeshData, MeshHandle, PipelineDescriptor,
    PipelineHandle, PipelineKind, RenderDevice, RenderSettings, TextureData, TextureHandle,
};

use crate::backend::{Backend, FrameData, PendingReadback, Submitted};
use crate::core::Error;
use crate::types::*;

/// Map a backend error onto a result code.
fn error(e: RenderError) -> Error {
    let code = match &e {
        RenderError::NoDevice => KAMAN_ERROR_NO_DEVICE,
        RenderError::LimitExceeded(_) => KAMAN_ERROR_LIMIT_EXCEEDED,
        RenderError::ShaderCompile(_) | RenderError::PipelineCreation(_) => KAMAN_ERROR_INTERNAL,
        _ => KAMAN_ERROR_INVALID_ARGUMENT,
    };
    Error::new(code, e.to_string())
}

pub(crate) struct MetalBackend {
    renderer: MetalRenderer,
    offscreen: bool,
    /// The two built-in pipelines, by `PipelineKind`: `[VertexColor, Textured]`.
    pipelines: [PipelineHandle; 2],
    /// 1×1 white, bound for textured draws that name no texture.
    white: TextureHandle,
}

impl MetalBackend {
    fn finish(mut renderer: MetalRenderer, offscreen: bool) -> Result<Self, Error> {
        let pipelines = [
            renderer.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor)),
            renderer.create_pipeline(&PipelineDescriptor::new(PipelineKind::Textured)),
        ];
        let white = renderer
            .try_create_texture(&TextureData {
                width: 1,
                height: 1,
                rgba8: &[255; 4],
                color_space: ColorSpace::Srgb,
            })
            .map_err(error)?;
        Ok(Self {
            renderer,
            offscreen,
            pipelines,
            white,
        })
    }

    pub(crate) fn offscreen(width: u32, height: u32) -> Result<Self, Error> {
        Self::finish(
            MetalRenderer::try_new_offscreen(width, height).map_err(error)?,
            true,
        )
    }

    /// # Safety
    /// `layer` must be a valid pointer to a live Objective-C object.
    pub(crate) unsafe fn layer(
        layer: *mut c_void,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, Error> {
        Self::finish(
            MetalRenderer::try_from_layer(layer, width, height, scale).map_err(error)?,
            false,
        )
    }
}

impl PendingReadback for Readback {
    fn is_complete(&self) -> bool {
        Readback::is_complete(self)
    }

    fn copy_to(&self, dst: &mut [u8]) -> bool {
        Readback::copy_to(self, dst)
    }
}

impl Backend for MetalBackend {
    fn device_name(&self) -> String {
        self.renderer.device_name()
    }

    fn is_offscreen(&self) -> bool {
        self.offscreen
    }

    fn create_mesh(
        &mut self,
        kind: PipelineKind,
        vertices: &[u8],
        indices: &[u32],
    ) -> Result<MeshHandle, Error> {
        self.renderer
            .try_create_mesh(&MeshData {
                vertices,
                indices,
                layout: kind.vertex_layout(),
            })
            .map_err(error)
    }

    fn destroy_mesh(&mut self, mesh: MeshHandle) {
        self.renderer.destroy_mesh(mesh);
    }

    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba8: &[u8],
        color_space: ColorSpace,
    ) -> Result<TextureHandle, Error> {
        self.renderer
            .try_create_texture(&TextureData {
                width,
                height,
                rgba8,
                color_space,
            })
            .map_err(error)
    }

    fn destroy_texture(&mut self, texture: TextureHandle) {
        self.renderer.destroy_texture(texture);
    }

    fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }

    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.renderer.resize_surface(width, height, scale);
    }

    fn settings(&self) -> RenderSettings {
        self.renderer.render_settings()
    }

    fn set_settings(&mut self, settings: &RenderSettings) {
        self.renderer.set_render_settings(settings);
    }

    fn frames_in_flight(&self) -> u64 {
        self.renderer.frames_in_flight()
    }

    fn submitted_frames(&self) -> u64 {
        self.renderer.submitted_frame_count()
    }

    fn completed_frames(&self) -> u64 {
        self.renderer.completed_frame_count()
    }

    fn submit(&mut self, frame: &FrameData<'_>) -> Option<Submitted> {
        let r = &mut self.renderer;
        // The camera and sun are sticky in the renderer and folded into the
        // frame's light block when the frame opens, so set them first.
        r.set_view_projection(frame.view_projection);
        r.set_camera_position(frame.camera_position);
        r.set_sun_sky(&frame.sun);
        r.try_begin_frame().ok()?;

        let readback = if frame.readback {
            r.request_readback()
        } else {
            None
        };
        let mut current: Option<PipelineKind> = None;
        for draw in frame.draws {
            if current != Some(draw.kind) {
                let pipeline = match draw.kind {
                    PipelineKind::VertexColor => self.pipelines[0],
                    PipelineKind::Textured => self.pipelines[1],
                };
                r.set_pipeline(pipeline);
                current = Some(draw.kind);
            }
            if draw.kind == PipelineKind::Textured {
                r.bind_texture(draw.texture.unwrap_or(self.white));
            }
            r.draw_mesh_matrix(
                draw.mesh,
                draw.model,
                &MaterialParams {
                    base_color: draw.base_color,
                },
            );
        }
        for quad in frame.quads {
            r.draw_overlay_quad(quad);
        }
        let number = r.submit_async()?;
        Some(Submitted {
            frame: number,
            readback: readback.map(|rb| Box::new(rb) as Box<dyn PendingReadback>),
        })
    }
}
