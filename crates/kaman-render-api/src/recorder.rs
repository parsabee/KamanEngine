// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`FrameRecorder`] trait — per-frame command recording.

use kaman_math::Transform;

use crate::descriptor::MaterialParams;
use crate::handles::{MeshHandle, PipelineHandle, TextureHandle};

/// Per-frame command recording: begin a frame, bind pipeline/texture state, record
/// draws, and submit.
///
/// A `FrameRecorder` is the seam's per-frame half; resource ownership lives in
/// [`RenderDevice`](crate::RenderDevice). Callers describe a frame as an ordered
/// sequence of calls — the backend translates them into a native command encoder
/// (e.g. Metal render command encoding) without exposing any GPU type above the
/// seam.
///
/// # Frame protocol (ordering invariants)
///
/// A well-formed frame follows this order:
///
/// 1. [`begin_frame`](Self::begin_frame) exactly once to open the frame.
/// 2. [`set_pipeline`](Self::set_pipeline) at least once **before the first draw**;
///    it may be called again to switch pipelines between draws.
/// 3. Any number of [`bind_texture`](Self::bind_texture) and
///    [`draw_mesh`](Self::draw_mesh) calls. `bind_texture` sets the texture used by
///    subsequent draws; `draw_mesh` emits geometry with the currently-bound
///    pipeline and texture.
/// 4. [`submit`](Self::submit) exactly once to close and present the frame.
///
/// Recording a draw before `begin_frame`, before any `set_pipeline`, or after
/// `submit` is a caller error. Implementations may panic, debug-assert, or drop the
/// call, but must not silently corrupt an in-flight frame. State set inside a frame
/// (pipeline, bound texture) does **not** carry across a `begin_frame`/`submit`
/// boundary — each frame starts with no pipeline and no texture bound.
pub trait FrameRecorder {
    /// Open a new frame for recording.
    ///
    /// # Contract
    /// - Must be called before any `set_pipeline`, `bind_texture`, or `draw_mesh`.
    /// - Resets per-frame state: after this call no pipeline and no texture are
    ///   bound.
    /// - Calling it twice without an intervening [`submit`](Self::submit) is a
    ///   caller error.
    fn begin_frame(&mut self);

    /// Select the render pipeline used by subsequent draws.
    ///
    /// # Contract
    /// - Requires an open frame (after [`begin_frame`](Self::begin_frame), before
    ///   [`submit`](Self::submit)).
    /// - Must be called at least once before the first [`draw_mesh`](Self::draw_mesh).
    /// - May be called repeatedly to switch pipelines; the most recent selection
    ///   applies to following draws.
    /// - `handle` must name a live pipeline from
    ///   [`RenderDevice::create_pipeline`](crate::RenderDevice::create_pipeline).
    fn set_pipeline(&mut self, handle: PipelineHandle);

    /// Bind a texture for subsequent draws.
    ///
    /// # Contract
    /// - Requires an open frame.
    /// - The bound texture applies to every [`draw_mesh`](Self::draw_mesh) that
    ///   follows, until replaced by another `bind_texture` or the frame ends.
    /// - `handle` must name a live texture from
    ///   [`RenderDevice::create_texture`](crate::RenderDevice::create_texture).
    fn bind_texture(&mut self, handle: TextureHandle);

    /// Record a draw of `mesh` at `transform` with `material`.
    ///
    /// # Contract
    /// - Requires an open frame with a pipeline already selected via
    ///   [`set_pipeline`](Self::set_pipeline).
    /// - `mesh` must name a live mesh from
    ///   [`RenderDevice::create_mesh`](crate::RenderDevice::create_mesh).
    /// - `transform` is the model-to-world transform for this instance; `material`
    ///   is the plain-data shading block. The currently-bound pipeline and texture
    ///   (if any) apply.
    fn draw_mesh(&mut self, mesh: MeshHandle, transform: &Transform, material: &MaterialParams);

    /// Close the current frame and present it.
    ///
    /// # Contract
    /// - Requires an open frame; ends it.
    /// - After this call, no frame is open — a new frame requires
    ///   [`begin_frame`](Self::begin_frame) again.
    /// - Calling it without an open frame is a caller error.
    fn submit(&mut self);
}
