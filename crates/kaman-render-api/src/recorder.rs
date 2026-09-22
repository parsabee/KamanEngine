// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`FrameRecorder`] trait — per-frame command recording.

use kaman_math::glam::Mat4;
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
/// 2. [`set_view_projection`](Self::set_view_projection) to establish the camera for
///    the draws that follow. It is **sticky** (retained across frames), so the
///    engine typically pushes it once per frame before the game records; a frame
///    that draws must have a view-projection in effect (set this frame or a prior
///    one).
/// 3. [`set_pipeline`](Self::set_pipeline) at least once **before the first draw**;
///    it may be called again to switch pipelines between draws.
/// 4. Any number of [`bind_texture`](Self::bind_texture) and
///    [`draw_mesh`](Self::draw_mesh) calls. `bind_texture` sets the texture used by
///    subsequent draws; `draw_mesh` emits geometry with the currently-bound
///    pipeline and texture.
/// 5. [`submit`](Self::submit) exactly once to close and present the frame.
///
/// Recording a draw before `begin_frame`, before any `set_pipeline`, or after
/// `submit` is a caller error. Implementations may panic, debug-assert, or drop the
/// call, but must not silently corrupt an in-flight frame. Pipeline and bound-texture
/// state do **not** carry across a `begin_frame`/`submit` boundary — each frame
/// starts with no pipeline and no texture bound. The **view-projection is the
/// exception: it is sticky**, retaining the last value set until it is replaced, so
/// the engine loop can push it once per frame *before* the game opens its frame (the
/// value survives `begin_frame`). A frame that never sets one draws with the identity
/// matrix (the construction default).
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

    /// Set the world → clip **view-projection** matrix for this frame's draws.
    ///
    /// This is the seam's per-frame camera state: the backend combines it with
    /// each draw's model transform to form the MVP (`mvp = view_proj * model`).
    /// The matrix is a plain [`Mat4`] from [`kaman_math::glam`]; **no GPU or Metal
    /// type crosses the seam**. A game computes it from a
    /// [`kaman_camera::Camera`](https://docs.rs/kaman-camera) (or any source) and
    /// pushes it here.
    ///
    /// # Contract (ordering)
    /// - Must be in effect **before the first [`draw_mesh`](Self::draw_mesh)** of a
    ///   frame — either set this frame or carried over from a prior one.
    /// - The value is **sticky**: it persists across `begin_frame`/`submit` until
    ///   replaced, so a default engine loop pushes the engine camera's
    ///   view-projection here **once per frame before `Game::render`** (before the
    ///   game opens its frame) and every draw that frame uses it.
    /// - May be called again to switch mid-frame; the most recent value applies to
    ///   the draws that follow.
    /// - Until the first call, the implementation uses the identity matrix.
    fn set_view_projection(&mut self, view_proj: Mat4);

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
