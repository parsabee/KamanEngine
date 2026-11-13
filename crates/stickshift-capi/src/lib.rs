// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The Stick Shift Engine **C API**: `libstickshift.a` / `libstickshift.dylib` and the generated
//! `include/stickshift.h`.
//!
//! The contract is `docs/C_API.md`; this crate implements it. In short:
//!
//! - One opaque [`StickShiftContext`] owns everything; meshes, textures, models and
//!   fonts are typed 64-bit handles with 32-bit generations, never reissued.
//! - The API is **asynchronous in shape**: calls return at once with a handle or
//!   a result code, resources go `PENDING` → `READY` / `FAILED`, and completions
//!   arrive as events, through `stickshift_poll_events` or per-call callbacks that
//!   always run on a host thread.
//! - In this beta, mesh and texture uploads still happen inside their create
//!   calls; glTF models are parsed on worker threads and uploaded during
//!   housekeeping; frames are recorded CPU-side and submitted through the Metal
//!   backend's non-blocking path, so `stickshift_frame_acquire` never waits.
//! - A context is used by one thread at a time; an overlapping call returns
//!   `STICKSHIFT_ERROR_BUSY` instead of waiting. No panic crosses the boundary:
//!   every function catches one and returns `STICKSHIFT_ERROR_INTERNAL`.
//!
//! The crate is graphics-only: it links `stickshift-render`, `stickshift-render-api`,
//! `stickshift-assets` (without its ECS feature) and `stickshift-math`, and none of the
//! windowing, audio, ECS or physics crates. On platforms without Metal only the
//! null backend exists, so the crate and its tests build everywhere.
//!
//! # Safety
//!
//! Every exported function is `unsafe extern "C"`: pointer arguments must be
//! NULL or valid as `stickshift.h` documents (a context from `stickshift_context_create`
//! that has not been destroyed, a frame from `stickshift_frame_acquire`, readable
//! inputs, writable outputs, NUL-terminated strings). Everything else (NULL,
//! wrong handles, bad enum values, mismatched sizes, overlapping calls) is
//! checked and reported as a result code.

#![deny(missing_docs)]
// Each function documents its pointer requirements; the crate-level `# Safety`
// section above states the shared contract once instead of in every function.
#![allow(clippy::missing_safety_doc)]

mod backend;
mod context;
mod core;
mod fonts;
mod frame;
mod handles;
mod math;
#[cfg(target_os = "macos")]
mod metal;
mod models;
mod resources;
pub mod types;

pub use context::{
    stickshift_context_create, stickshift_context_destroy, stickshift_context_device_name,
    stickshift_context_flush, stickshift_context_get_settings, stickshift_context_get_size,
    stickshift_context_get_stats, stickshift_context_resize, stickshift_context_set_settings,
    stickshift_poll_events, stickshift_render_settings_default, stickshift_resource_state,
    StickShiftContext, StickShiftFrame,
};
pub use core::{
    stickshift_api_version, stickshift_last_error_message, stickshift_result_name,
    stickshift_version_string,
};
pub use fonts::{stickshift_font_create, stickshift_font_destroy, stickshift_font_measure};
pub use frame::{
    stickshift_frame_acquire, stickshift_frame_discard, stickshift_frame_draw_mesh,
    stickshift_frame_draw_model, stickshift_frame_draw_quad, stickshift_frame_draw_text,
    stickshift_frame_read_pixels_async, stickshift_frame_set_camera, stickshift_frame_set_sun_sky,
    stickshift_frame_submit, stickshift_sun_sky_default,
};
pub use math::{
    stickshift_mat4_from_trs, stickshift_mat4_identity, stickshift_mat4_look_at,
    stickshift_mat4_mul, stickshift_mat4_perspective, stickshift_quat_from_axis_angle,
};
pub use models::{
    stickshift_model_destroy, stickshift_model_get_bounds, stickshift_model_load_file,
    stickshift_model_load_memory,
};
pub use resources::{
    stickshift_mesh_create, stickshift_mesh_destroy, stickshift_texture_create,
    stickshift_texture_destroy,
};
pub use types::*;
