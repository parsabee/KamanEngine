// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Frames: recorded CPU-side, re-checked and handed to the backend whole at
//! submit. Acquiring a frame never waits.

use stickshift_math::glam::Mat4;
use stickshift_render_api::{ColorSpace, OverlayFill, OverlayQuad, PipelineKind, SunSky};

use crate::backend::{DrawCall, FrameData};
use crate::context::{frame_of, with_context, with_frame, Inner, OwnedEvent, PendingRead};
use crate::context::{StickShiftContext, StickShiftFrame, MAX_FRAMES_IN_FLIGHT};
use crate::core::{call, input, utf8, Error};
use crate::handles::{kind_of, Kind, Slots};
use crate::models::{ModelEntry, ModelState};
use crate::resources::{MeshEntry, TextureEntry};
use crate::types::*;

/// A frame's recorded contents. Kept on the context and cleared (not freed)
/// between frames.
#[derive(Default)]
pub(crate) struct Recording {
    draws: Vec<DrawCall>,
    /// Per draw: the mesh or model handle and the texture handle (or 0) it
    /// came from, re-checked at submit.
    draw_sources: Vec<(u64, u64)>,
    quads: Vec<OverlayQuad>,
    /// Per quad: the texture handle it samples, or 0.
    quad_sources: Vec<u64>,
    /// The draws and quads still valid at submit.
    kept_draws: Vec<DrawCall>,
    kept_quads: Vec<OverlayQuad>,
    readback: Option<(*mut u8, usize, Option<StickShiftCompletion>)>,
}

impl Recording {
    fn clear(&mut self) {
        self.draws.clear();
        self.draw_sources.clear();
        self.quads.clear();
        self.quad_sources.clear();
        self.kept_draws.clear();
        self.kept_quads.clear();
        self.readback = None;
    }
}

/// Whether a draw's source handles are still live and ready.
fn live(
    meshes: &Slots<MeshEntry>,
    textures: &Slots<TextureEntry>,
    models: &Slots<ModelEntry>,
    (object, texture): (u64, u64),
) -> bool {
    let object_ok = match kind_of(object) {
        Some(Kind::Mesh) => meshes.get(object).is_some(),
        Some(Kind::Model) => models
            .get(object)
            .is_some_and(|m| m.state == ModelState::Ready),
        _ => false,
    };
    object_ok && (texture == 0 || textures.get(texture).is_some())
}

/// A texture handle's backend texture; `INVALID_HANDLE` if it is not live.
fn texture(
    inner: &Inner,
    id: u64,
    what: &str,
) -> Result<stickshift_render_api::TextureHandle, Error> {
    inner
        .textures
        .get(id)
        .map(|t| t.handle)
        .ok_or_else(|| Error::invalid_handle(what))
}

/// Report a requested read-back that will never arrive.
fn fail_readback(inner: &mut Inner, frame: u64, completion: Option<StickShiftCompletion>) {
    let event = OwnedEvent::failed(
        STICKSHIFT_EVENT_READBACK_FAILED,
        0,
        frame,
        &Error::new(STICKSHIFT_FRAME_SKIPPED, "the frame was not rendered"),
    );
    inner.emit(event, completion);
}

/// Open a frame for recording. Never waits: `STICKSHIFT_FRAME_SKIPPED` (and `*out`
/// NULL) when the GPU has the maximum frames in flight or the drawable is empty.
///
/// # Safety
/// `ctx` must be NULL or a live context; `out` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_acquire(
    ctx: *mut StickShiftContext,
    out: *mut *mut StickShiftFrame,
) -> StickShiftResult {
    call(|| {
        let out = crate::core::output(out, "out")?;
        *out = std::ptr::null_mut();
        let code = with_context(ctx, |inner| {
            if inner.frame_open {
                return Err(Error::wrong_state(
                    "a frame is already open: submit or discard it first",
                ));
            }
            let (width, height) = inner.surface_size();
            if width == 0 || height == 0 || inner.backend.frames_in_flight() >= MAX_FRAMES_IN_FLIGHT
            {
                inner.counters.frames_skipped += 1;
                return Ok(STICKSHIFT_FRAME_SKIPPED);
            }
            inner.recording.clear();
            inner.frame_open = true;
            Ok(STICKSHIFT_OK)
        })?;
        if code == STICKSHIFT_OK {
            *out = frame_of(&*ctx);
        }
        Ok(code)
    })
}

/// Set the camera. Sticky across frames; the last value before submit applies
/// to the whole frame.
///
/// # Safety
/// `frame` must be NULL or an open frame; `camera` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_set_camera(
    frame: *mut StickShiftFrame,
    camera: *const StickShiftCamera,
) -> StickShiftResult {
    call(|| {
        let camera = *input(camera, "camera")?;
        with_frame(frame, |inner| {
            inner.camera = (camera.view_projection.into(), camera.position.into());
            Ok(STICKSHIFT_OK)
        })
    })
}

impl From<StickShiftSunSky> for SunSky {
    fn from(s: StickShiftSunSky) -> Self {
        Self {
            sun_elevation_deg: s.sun_elevation_deg,
            sun_azimuth_deg: s.sun_azimuth_deg,
            sun_color: s.sun_color,
            sun_intensity: s.sun_intensity,
            sky_fill: s.sky_fill,
            sky_zenith_color: s.sky_zenith_color,
            sky_horizon_color: s.sky_horizon_color,
        }
    }
}

impl From<SunSky> for StickShiftSunSky {
    fn from(s: SunSky) -> Self {
        Self {
            sun_elevation_deg: s.sun_elevation_deg,
            sun_azimuth_deg: s.sun_azimuth_deg,
            sun_color: s.sun_color,
            sun_intensity: s.sun_intensity,
            sky_fill: s.sky_fill,
            sky_zenith_color: s.sky_zenith_color,
            sky_horizon_color: s.sky_horizon_color,
        }
    }
}

/// Set the sun and sky. Sticky across frames.
///
/// # Safety
/// `frame` must be NULL or an open frame; `sun` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_set_sun_sky(
    frame: *mut StickShiftFrame,
    sun: *const StickShiftSunSky,
) -> StickShiftResult {
    call(|| {
        let sun = SunSky::from(*input(sun, "sun")?);
        with_frame(frame, |inner| {
            inner.sun = sun;
            Ok(STICKSHIFT_OK)
        })
    })
}

/// The default sun and sky: a high, white afternoon sun over a blue gradient.
/// NULL is a no-op. Thread-safe.
///
/// # Safety
/// `out` must be NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_sun_sky_default(out: *mut StickShiftSunSky) {
    if let Some(out) = out.as_mut() {
        *out = SunSky::default().into();
    }
}

const WHITE: StickShiftColor = StickShiftColor {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// Record a draw of `mesh` with model-to-world matrix `*model`. `material` may
/// be NULL (white, no texture).
///
/// # Safety
/// `frame` must be NULL or an open frame; `model` and `material` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_draw_mesh(
    frame: *mut StickShiftFrame,
    mesh: StickShiftMesh,
    model: *const StickShiftMat4,
    material: *const StickShiftMaterial,
) -> StickShiftResult {
    call(|| {
        let model = Mat4::from(*input(model, "model")?);
        let material = material.as_ref().copied().unwrap_or(StickShiftMaterial {
            base_color: WHITE,
            base_color_texture: StickShiftTexture { id: 0 },
        });
        with_frame(frame, |inner| {
            let entry = inner
                .meshes
                .get(mesh.id)
                .ok_or_else(|| Error::invalid_handle("mesh"))?;
            let (kind, handle) = (entry.kind, entry.handle);
            let texture_id = material.base_color_texture.id;
            let texture = match texture_id {
                0 => None,
                id => Some(texture(inner, id, "base_color_texture")?),
            };
            let recording = &mut inner.recording;
            recording.draws.push(DrawCall {
                kind,
                mesh: handle,
                model,
                base_color: material.base_color.to_array(),
                texture: if kind == PipelineKind::Textured {
                    texture
                } else {
                    None
                },
            });
            recording.draw_sources.push((mesh.id, texture_id));
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Record a draw of every part of `model` at `*world`, its base colours
/// multiplied by `*tint` (NULL: white). `STICKSHIFT_NOT_READY` until it loads.
///
/// # Safety
/// `frame` must be NULL or an open frame; `world` and `tint` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_draw_model(
    frame: *mut StickShiftFrame,
    model: StickShiftModel,
    world: *const StickShiftMat4,
    tint: *const StickShiftColor,
) -> StickShiftResult {
    call(|| {
        let world = Mat4::from(*input(world, "world")?);
        let tint = tint.as_ref().copied().unwrap_or(WHITE).to_array();
        with_frame(frame, |inner| {
            let entry = inner
                .models
                .get(model.id)
                .ok_or_else(|| Error::invalid_handle("model"))?;
            if entry.state != ModelState::Ready {
                inner.counters.draws_skipped += 1;
                return Ok(STICKSHIFT_NOT_READY);
            }
            for &(part, transform) in &entry.instances {
                let part = &entry.parts[part];
                let mut base_color = part.base_color;
                for (c, t) in base_color.iter_mut().zip(tint) {
                    *c *= t;
                }
                inner.recording.draws.push(DrawCall {
                    kind: part.kind,
                    mesh: part.mesh,
                    model: world * transform,
                    base_color,
                    texture: part.texture,
                });
                inner.recording.draw_sources.push((model.id, 0));
            }
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Record a 2D overlay quad, composited over the 3D scene in record order.
///
/// # Safety
/// `frame` must be NULL or an open frame; `quad` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_draw_quad(
    frame: *mut StickShiftFrame,
    quad: *const StickShiftQuad,
) -> StickShiftResult {
    call(|| {
        let q = *input(quad, "quad")?;
        with_frame(frame, |inner| {
            let (fill, source) = match q.fill {
                STICKSHIFT_FILL_SOLID => (OverlayFill::Solid, 0),
                STICKSHIFT_FILL_TEXTURE => (
                    OverlayFill::Textured(texture(inner, q.texture.id, "texture")?),
                    q.texture.id,
                ),
                STICKSHIFT_FILL_SDF => {
                    let entry = inner
                        .textures
                        .get(q.texture.id)
                        .ok_or_else(|| Error::invalid_handle("texture"))?;
                    if entry.color_space != ColorSpace::Linear {
                        return Err(Error::invalid_argument(
                            "a STICKSHIFT_FILL_SDF texture holds distances, not colour: create it \
                             with STICKSHIFT_COLOR_SPACE_LINEAR",
                        ));
                    }
                    (OverlayFill::Sdf(entry.handle), q.texture.id)
                }
                other => {
                    return Err(Error::invalid_argument(format!(
                        "{other} is not a STICKSHIFT_FILL_* value"
                    )))
                }
            };
            inner.recording.quads.push(OverlayQuad {
                rect: [q.x, q.y, q.width, q.height],
                uv: [q.u0, q.v0, q.u1, q.v1],
                color: q.color.to_array(),
                fill,
            });
            inner.recording.quad_sources.push(source);
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Record `text` in `font` with the pen starting at `(x, y)` (the left end of
/// the first line's baseline) at `px` pixels per em. `STICKSHIFT_NOT_READY` if the
/// font's atlas texture has been destroyed.
///
/// # Safety
/// `frame` must be NULL or an open frame; `text` NULL or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_draw_text(
    frame: *mut StickShiftFrame,
    font: StickShiftFont,
    text: *const std::ffi::c_char,
    x: f32,
    y: f32,
    px: f32,
    color: StickShiftColor,
) -> StickShiftResult {
    call(|| {
        let text = utf8(text, "text")?;
        with_frame(frame, |inner| {
            let entry = inner
                .fonts
                .get(font.id)
                .ok_or_else(|| Error::invalid_handle("font"))?;
            if inner.textures.get(entry.atlas_id).is_none() {
                inner.counters.draws_skipped += 1;
                return Ok(STICKSHIFT_NOT_READY);
            }
            let atlas_id = entry.atlas_id;
            let recording = &mut inner.recording;
            entry
                .atlas
                .layout(text, [x, y], px, color.to_array(), |quad| {
                    recording.quads.push(quad);
                    recording.quad_sources.push(atlas_id);
                });
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Ask for this frame's pixels (offscreen contexts only): `width * height * 4`
/// bytes of BGRA8, top row first, into `dst`, which must stay valid until the
/// frame's `READBACK_COMPLETE` or `READBACK_FAILED` event.
///
/// # Safety
/// `frame` must be NULL or an open frame; `dst` valid for writes of `len`
/// bytes until the event; `completion` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_read_pixels_async(
    frame: *mut StickShiftFrame,
    dst: *mut std::ffi::c_void,
    len: usize,
    completion: *const StickShiftCompletion,
) -> StickShiftResult {
    call(|| {
        let completion = completion.as_ref().copied();
        if dst.is_null() {
            return Err(Error::null("dst"));
        }
        with_frame(frame, |inner| {
            if !inner.backend.is_offscreen() {
                return Err(Error::new(
                    STICKSHIFT_ERROR_UNSUPPORTED,
                    "read-back needs an offscreen context",
                ));
            }
            let (width, height) = inner.surface_size();
            let need = width as usize * height as usize * 4;
            if len < need {
                return Err(Error::invalid_argument(format!(
                    "dst holds {len} bytes; a {width}x{height} frame needs {need}"
                )));
            }
            if inner.recording.readback.is_some() {
                return Err(Error::wrong_state("this frame already has a read-back"));
            }
            inner.recording.readback = Some((dst.cast(), need, completion));
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Encode, commit and present the frame, then close it. Never waits for the
/// GPU. `STICKSHIFT_FRAME_SKIPPED` if the layer gave no drawable. `completion`
/// receives the frame's `FRAME_COMPLETED`; `*out_frame_number` (optional) is
/// the frame's number, or 0 when skipped.
///
/// # Safety
/// `frame` must be NULL or an open frame; `completion` and `out_frame_number`
/// NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_submit(
    frame: *mut StickShiftFrame,
    completion: *const StickShiftCompletion,
    out_frame_number: *mut u64,
) -> StickShiftResult {
    call(|| {
        let completion = completion.as_ref().copied();
        let mut number = 0;
        let code = with_frame(frame, |inner| {
            // Drop anything destroyed (or a model that stopped being ready)
            // since it was recorded.
            let recording = &mut inner.recording;
            recording.kept_draws.clear();
            recording.kept_quads.clear();
            let mut skipped = 0;
            for (draw, &source) in recording.draws.iter().zip(&recording.draw_sources) {
                if live(&inner.meshes, &inner.textures, &inner.models, source) {
                    recording.kept_draws.push(*draw);
                } else {
                    skipped += 1;
                }
            }
            for (quad, &source) in recording.quads.iter().zip(&recording.quad_sources) {
                if source == 0 || inner.textures.get(source).is_some() {
                    recording.kept_quads.push(*quad);
                } else {
                    skipped += 1;
                }
            }
            inner.counters.draws_skipped += skipped;

            let submitted = inner.backend.submit(&FrameData {
                view_projection: inner.camera.0,
                camera_position: inner.camera.1,
                sun: inner.sun,
                draws: &inner.recording.kept_draws,
                quads: &inner.recording.kept_quads,
                readback: inner.recording.readback.is_some(),
            });
            let readback = inner.recording.readback.take();
            let drawn = inner.recording.kept_draws.len() as u64;
            inner.recording.clear();
            inner.frame_open = false;

            let Some(submitted) = submitted else {
                inner.counters.frames_skipped += 1;
                if let Some((_, _, read_completion)) = readback {
                    fail_readback(inner, 0, read_completion);
                }
                return Ok(STICKSHIFT_FRAME_SKIPPED);
            };
            inner.counters.draws_submitted += drawn;
            inner
                .pending_frames
                .push_back((submitted.frame, completion));
            match (readback, submitted.readback) {
                (Some((dst, len, read_completion)), Some(copy)) => {
                    inner.pending_reads.push(PendingRead {
                        frame: submitted.frame,
                        copy,
                        dst,
                        len,
                        completion: read_completion,
                    });
                }
                (Some((_, _, read_completion)), None) => {
                    fail_readback(inner, submitted.frame, read_completion);
                }
                (None, _) => {}
            }
            number = submitted.frame;
            Ok(STICKSHIFT_OK)
        })?;
        if let Some(out) = out_frame_number.as_mut() {
            *out = number;
        }
        Ok(code)
    })
}

/// Close the frame without rendering it. A requested read-back fails.
///
/// # Safety
/// `frame` must be NULL or an open frame.
#[no_mangle]
pub unsafe extern "C" fn stickshift_frame_discard(frame: *mut StickShiftFrame) -> StickShiftResult {
    call(|| {
        with_frame(frame, |inner| {
            let readback = inner.recording.readback.take();
            inner.recording.clear();
            inner.frame_open = false;
            if let Some((_, _, completion)) = readback {
                fail_readback(inner, 0, completion);
            }
            Ok(STICKSHIFT_OK)
        })
    })
}
