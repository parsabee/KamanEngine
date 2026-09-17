// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Opaque GPU-resource handles.
//!
//! These are the tokens a [`RenderDevice`](crate::RenderDevice) hands back when a
//! resource is created and that a [`FrameRecorder`](crate::FrameRecorder) consumes
//! when recording draws. They are deliberately **opaque**: they carry only an
//! integer identity, never a backend (Metal) object. Code above the render seam
//! stores and passes these handles around without ever learning what they point at
//! on the GPU — that is the whole point of the firewall.
//!
//! Each handle is a plain newtype over a [`u32`]. The wrapped value is the backend's
//! private business: for a real Metal backend it is typically an index into a
//! resource table (a slot in a `Vec` of `metal::Buffer` / `metal::Texture` /
//! `metal::RenderPipelineState`), but this crate does not mandate any particular
//! allocation scheme. All three types are `Copy`, cheap to pass by value, and usable
//! as map keys (`Eq + Hash`).

/// Opaque handle to a mesh resource (vertex + index geometry) owned by a
/// [`RenderDevice`](crate::RenderDevice).
///
/// Returned by [`RenderDevice::create_mesh`](crate::RenderDevice::create_mesh) and
/// consumed by [`FrameRecorder::draw_mesh`](crate::FrameRecorder::draw_mesh). The
/// wrapped [`u32`] is a backend-private identity; do not interpret it above the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

/// Opaque handle to a texture resource owned by a
/// [`RenderDevice`](crate::RenderDevice).
///
/// Returned by [`RenderDevice::create_texture`](crate::RenderDevice::create_texture)
/// and consumed by [`FrameRecorder::bind_texture`](crate::FrameRecorder::bind_texture).
/// The wrapped [`u32`] is a backend-private identity; do not interpret it above the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u32);

/// Opaque handle to a render-pipeline resource (shaders + fixed-function state)
/// owned by a [`RenderDevice`](crate::RenderDevice).
///
/// Returned by
/// [`RenderDevice::create_pipeline`](crate::RenderDevice::create_pipeline) and
/// selected by [`FrameRecorder::set_pipeline`](crate::FrameRecorder::set_pipeline).
/// The wrapped [`u32`] is a backend-private identity; do not interpret it above the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineHandle(pub u32);
