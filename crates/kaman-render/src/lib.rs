// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Raw-Metal implementation of the `kaman-render-api` seam for KamanEngine.
//!
//! This crate is the concrete render backend that lives **below** the render
//! seam (`docs/ARCHITECTURE.md` §2). It is the *only* engine crate that depends
//! on `metal`; everything above the seam talks to the `kaman-render-api` traits
//! and therefore never imports a Metal type. It migrates the prototype's
//! rasterization renderer (KE-0102).
//!
//! # The backend and the seam
//!
//! [`MetalRenderer`] implements both halves of the seam —
//! [`RenderDevice`](kaman_render_api::RenderDevice) (load-time resource
//! create/destroy) and [`FrameRecorder`](kaman_render_api::FrameRecorder)
//! (per-frame `begin_frame` / `draw_mesh` / `submit`) — so it automatically
//! satisfies `kaman-core`'s `Renderer` marker trait via that crate's blanket
//! impl. `kaman-core` does **not** depend on this crate: the game binary
//! constructs a [`MetalRenderer`] and hands it to the engine as a
//! `Box<dyn Renderer>`, keeping `metal` out of `kaman-core`'s dependency tree.
//!
//! ## Mesh / uniform / draw mapping
//!
//! - `create_mesh` uploads (deindexes) geometry into a **persistent** Metal
//!   vertex buffer **once**, at load time, and stores it in a generational
//!   [`Registry`] keyed by the returned [`MeshHandle`](kaman_render_api::MeshHandle)
//!   (KE-0103). This is the "upload once, reference by handle" rule.
//! - `create_pipeline` names the built-in Phong pipeline (Phase-1 port has one).
//! - `draw_mesh(handle, transform, material)` looks the persistent vertex buffer
//!   up by handle (**no per-frame mesh allocation**), computes an MVP from the
//!   seam-provided view-projection and this instance's transform, writes a per-draw uniform
//!   buffer (the one remaining per-frame allocation, removed by KE-0104), and
//!   records a triangle draw. A stale/freed handle is a defined no-op, never a
//!   silent wrong-buffer draw (see [`RegistryError`]).
//! - `begin_frame` acquires the color attachment (drawable or offscreen
//!   texture) and uploads the frame's light; draws are *recorded* and `submit`
//!   encodes the frame's two passes (shadow, then scene — see below), then
//!   presents (windowed) or synchronizes for readback (offscreen).
//!
//! # Camera (via the seam)
//!
//! The backend does **not** own a camera. The view-projection matrix arrives
//! through the seam once per frame via
//! [`FrameRecorder::set_view_projection`](kaman_render_api::FrameRecorder::set_view_projection):
//! the game (or engine loop) computes it from a
//! [`kaman_camera::Camera`](https://docs.rs/kaman-camera) and pushes it before
//! the first `draw_mesh`. `draw_mesh` combines the stored view-projection with
//! each draw's model transform to form the MVP (KE-0205). The previously-inlined
//! minimal camera was deleted with this migration.
//!
//! The camera's **world position** arrives the same way, via
//! [`FrameRecorder::set_camera_position`](kaman_render_api::FrameRecorder::set_camera_position)
//! (KE-0406), because a matrix alone cannot say where the eye is and specular
//! needs to know. Both are sticky and describe one camera; the backend folds them
//! into the frame's light block as it opens each frame.
//!
//! # Sun and sky (via the seam)
//!
//! Lighting is likewise not the backend's choice. A
//! [`SunSky`](kaman_render_api::SunSky) — sun elevation/azimuth, colour,
//! intensity, sky fill, and the sky gradient — is pushed through
//! [`FrameRecorder::set_sun_sky`](kaman_render_api::FrameRecorder::set_sun_sky)
//! and uploaded per frame (KE-0406). The backend derives the light direction from
//! the angles once and uses that single vector for both the shading and the sun
//! disc the sky pass draws, so they cannot disagree. Until something pushes one,
//! the default is the `Default` impl of [`SunSky`](kaman_render_api::SunSky).
//!
//! # Shadows (KE-0407)
//!
//! The sun casts real shadows through a single fitted shadow map. Each frame is
//! two render passes: a **depth-only shadow pass** from the sun (every recorded
//! draw is a caster) into a stored `Depth32Float` map, then the **scene pass**,
//! whose untextured and textured lit shaders both sample it. The light's
//! orthographic projection is refitted every frame to the camera's visible slab
//! and snapped to whole texels so it never shimmers, and it is derived from the
//! same [`SunSky`](kaman_render_api::SunSky) direction the shading uses. The
//! fitting, bias and filtering live (and are unit-tested, GPU-free) in
//! [`shadow`]; the pass wiring and its TBDR store-action reasoning in
//! [`backend`]. Nothing about shadows crosses the seam: a game gets them by
//! drawing and setting a sun.
//!
//! # Ray tracer (feature-gated)
//!
//! The ray-tracing code that was entangled in the prototype renderer is behind
//! the off-by-default `raytracer` feature. The **default** build is
//! rasterization-only and does not compile `raytracing.metal`. The feature is
//! additionally excluded on iOS (`not(target_os = "ios")`): the raytracer is a
//! desktop-only, non-shipping path, so enabling the feature has no effect in an
//! iOS build.
//!
//! # Shader compilation
//!
//! By default shaders compile at runtime via `include_str!` + `new_library_with_source`,
//! which needs no toolchain beyond the Command Line Tools. The off-by-default
//! `precompiled-shaders` feature instead loads a `.metallib` compiled ahead of time by
//! `build.rs` (via `new_library_with_data`); it requires the Metal toolchain (full Xcode)
//! and is the KE-0107 path. See `scripts/preflight.sh --ios`.

#![deny(missing_docs)]

pub mod backend;
pub mod frame_sync;
pub mod registry;
pub mod shadow;
pub mod vertex;

#[cfg(all(feature = "raytracer", not(target_os = "ios")))]
pub mod raytracer;

pub use backend::MetalRenderer;
pub use registry::{Registry, RegistryError};
pub use vertex::{LightUniforms, MaterialUniforms, Uniforms, Vertex};

#[cfg(test)]
mod feature_gate_tests {
    /// The raytracer is a desktop-only, non-shipping path: it must be off in the
    /// default build so the mobile rasterization path stays lean (KR1.4). This
    /// test runs in the default `cargo test` (no `--features raytracer`) and
    /// fails if the feature ever becomes a default. The module itself is
    /// `#[cfg(all(feature = "raytracer", not(target_os = "ios")))]`, so a default
    /// build compiles zero raytracer symbols and an iOS build never does.
    #[test]
    #[cfg(not(feature = "raytracer"))]
    fn raytracer_is_off_by_default() {
        assert!(
            !cfg!(feature = "raytracer"),
            "the `raytracer` feature must remain off by default"
        );
    }
}
