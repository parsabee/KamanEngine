// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Backend-agnostic render seam for KamanEngine: [`RenderDevice`] / [`FrameRecorder`] traits.
//!
//! This crate is the **blast-radius firewall** described in `docs/ARCHITECTURE.md` §2. It
//! defines *only* a contract — opaque resource handles plus two traits — and never depends on
//! `metal`. A concrete renderer (`kaman-render`, KE-0102) implements these traits below the
//! seam; every crate above the seam (ECS, scene, physics, scripting, game code) talks to the
//! contract and therefore never imports a Metal type. A full renderer rewrite is provably
//! contained to the implementing crate.
//!
//! # The "no Metal above this line" rule
//!
//! `kaman-render-api` depends on `std` and [`kaman_math`] (for [`kaman_math::Transform`]) and
//! nothing else. It must never list `metal` in its `Cargo.toml`; a CI check asserts `metal` is
//! absent from this crate's dependency tree.
//!
//! # Public surface
//!
//! - Opaque handles: [`MeshHandle`], [`TextureHandle`], [`PipelineHandle`] — newtypes that
//!   name GPU resources without exposing what they point at.
//! - [`RenderDevice`] — load-time resource create/destroy.
//! - [`FrameRecorder`] — per-frame `begin_frame` / `set_pipeline` / `bind_texture` /
//!   `draw_mesh` / `submit`.
//! - Plain-data descriptors: [`VertexLayout`], [`VertexAttribute`], [`VertexFormat`],
//!   [`MaterialParams`], [`MeshData`], [`TextureData`], [`PipelineDescriptor`].
//! - [`SunSky`] — the engine-generic sun + sky description (KE-0406), pushed per
//!   frame through [`FrameRecorder::set_sun_sky`]. Physical parameters only: the
//!   sun's elevation/azimuth, colour and intensity, the sky-fill level, and the
//!   sky gradient. No time-of-day policy lives below the seam.
//! - [`NullRenderer`] — a headless, GPU-free test double implementing both traits so scene
//!   and game code can be unit-tested without a device.
//!
//! # Example
//!
//! ```rust
//! use kaman_render_api::{
//!     FrameRecorder, MaterialParams, MeshData, NullRenderer, PipelineDescriptor,
//!     RenderDevice, VertexLayout,
//! };
//! use kaman_math::Transform;
//!
//! let mut r = NullRenderer::new();
//!
//! // Load time: create resources, keep the opaque handles.
//! let mesh = r.create_mesh(&MeshData {
//!     vertices: &[],
//!     indices: &[],
//!     layout: VertexLayout::default(),
//! });
//! let pipeline = r.create_pipeline(&PipelineDescriptor {
//!     vertex_shader: "vs_main".into(),
//!     fragment_shader: "fs_main".into(),
//!     vertex_layout: VertexLayout::default(),
//! });
//!
//! // Per frame: record an ordered command stream.
//! r.begin_frame();
//! r.set_pipeline(pipeline);
//! r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
//! r.submit();
//!
//! assert_eq!(r.draw_count(), 1);
//! ```

#![deny(missing_docs)]

pub mod descriptor;
pub mod device;
pub mod handles;
pub mod null;
pub mod overlay;
pub mod recorder;
pub mod sun;

pub use descriptor::{MaterialParams, VertexAttribute, VertexFormat, VertexLayout};
pub use device::{MeshData, PipelineDescriptor, RenderDevice, TextureData};
pub use handles::{MeshHandle, PipelineHandle, TextureHandle};
pub use null::{NullRenderer, RecordedDraw};
pub use overlay::{FontAtlas, Glyph, OverlayFill, OverlayQuad};
pub use recorder::FrameRecorder;
pub use sun::SunSky;
