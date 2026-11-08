// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The KamanEngine **C API**: `libkaman.a` / `libkaman.dylib` and the generated
//! `include/kaman.h`.
//!
//! The contract is `docs/C_API.md`; this crate implements it. In short:
//!
//! - One opaque [`KamanContext`] owns everything; meshes, textures, models and
//!   fonts are typed 64-bit handles with 32-bit generations, never reissued.
//! - The API is **asynchronous in shape**: calls return at once with a handle or
//!   a result code, resources go `PENDING` → `READY` / `FAILED`, and completions
//!   arrive as events, through `kaman_poll_events` or per-call callbacks that
//!   always run on a host thread.
//! - In this beta, mesh and texture uploads still happen inside their create
//!   calls; glTF models are parsed on worker threads and uploaded during
//!   housekeeping; frames are recorded CPU-side and submitted through the Metal
//!   backend's non-blocking path, so `kaman_frame_acquire` never waits.
//! - A context is used by one thread at a time; an overlapping call returns
//!   `KAMAN_ERROR_BUSY` instead of waiting. No panic crosses the boundary:
//!   every function catches one and returns `KAMAN_ERROR_INTERNAL`.
//!
//! The crate is graphics-only: it links `kaman-render`, `kaman-render-api`,
//! `kaman-assets` (without its ECS feature) and `kaman-math`, and none of the
//! windowing, audio, ECS or physics crates. On platforms without Metal only the
//! null backend exists, so the crate and its tests build everywhere.
//!
//! # Safety
//!
//! Every exported function is `unsafe extern "C"`: pointer arguments must be
//! NULL or valid as `kaman.h` documents (a context from `kaman_context_create`
//! that has not been destroyed, a frame from `kaman_frame_acquire`, readable
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
    kaman_context_create, kaman_context_destroy, kaman_context_device_name, kaman_context_flush,
    kaman_context_get_settings, kaman_context_get_size, kaman_context_get_stats,
    kaman_context_resize, kaman_context_set_settings, kaman_poll_events,
    kaman_render_settings_default, kaman_resource_state, KamanContext, KamanFrame,
};
pub use core::{
    kaman_api_version, kaman_last_error_message, kaman_result_name, kaman_version_string,
};
pub use fonts::{kaman_font_create, kaman_font_destroy, kaman_font_measure};
pub use frame::{
    kaman_frame_acquire, kaman_frame_discard, kaman_frame_draw_mesh, kaman_frame_draw_model,
    kaman_frame_draw_quad, kaman_frame_draw_text, kaman_frame_read_pixels_async,
    kaman_frame_set_camera, kaman_frame_set_sun_sky, kaman_frame_submit, kaman_sun_sky_default,
};
pub use math::{
    kaman_mat4_from_trs, kaman_mat4_identity, kaman_mat4_look_at, kaman_mat4_mul,
    kaman_mat4_perspective, kaman_quat_from_axis_angle,
};
pub use models::{
    kaman_model_destroy, kaman_model_get_bounds, kaman_model_load_file, kaman_model_load_memory,
};
pub use resources::{
    kaman_mesh_create, kaman_mesh_destroy, kaman_texture_create, kaman_texture_destroy,
};
pub use types::*;
