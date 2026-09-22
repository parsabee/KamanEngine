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
//!   backend camera and this instance's transform, writes a per-draw uniform
//!   buffer (the one remaining per-frame allocation, removed by KE-0104), and
//!   records a triangle draw. A stale/freed handle is a defined no-op, never a
//!   silent wrong-buffer draw (see [`RegistryError`](registry::RegistryError)).
//! - `begin_frame` acquires the color attachment (drawable or offscreen
//!   texture) and opens a render encoder; `submit` ends encoding and presents
//!   (windowed) or synchronizes for readback (offscreen).
//!
//! # Camera (temporary inline)
//!
//! `kaman-camera` is still a stub, so a **minimal** view/projection [`Camera`]
//! is inlined here (see [`camera`]) to place the reference scene. A later
//! camera-migration ticket should replace it with the real crate.
//!
//! # Ray tracer (feature-gated)
//!
//! The ray-tracing code that was entangled in the prototype renderer is behind
//! the off-by-default [`raytracer`](crate::raytracer) feature. The **default**
//! build is rasterization-only and does not compile `raytracing.metal`. KE-0106
//! formalizes the gating and the iOS exclusion.
//!
//! # Runtime shader compilation
//!
//! Shaders still compile at runtime via `include_str!` + `new_library_with_source`
//! (the `.metallib` precompile is KE-0107).

#![deny(missing_docs)]

pub mod backend;
pub mod camera;
pub mod frame_sync;
pub mod registry;
pub mod vertex;

#[cfg(feature = "raytracer")]
pub mod raytracer;

pub use backend::MetalRenderer;
pub use camera::Camera;
pub use registry::{Registry, RegistryError};
pub use vertex::{LightUniforms, Uniforms, Vertex};
