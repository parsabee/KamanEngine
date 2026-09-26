// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`FrameRecorder`] trait — per-frame command recording.

use kaman_math::glam::{Mat4, Vec3};
use kaman_math::Transform;

use crate::descriptor::MaterialParams;
use crate::handles::{MeshHandle, PipelineHandle, TextureHandle};
use crate::overlay::OverlayQuad;
use crate::sun::SunSky;

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
/// 2. [`set_view_projection`](Self::set_view_projection) and
///    [`set_camera_position`](Self::set_camera_position) to establish the camera for
///    the draws that follow. Both are **sticky** (retained across frames), so the
///    engine typically pushes them once per frame before the game records; a frame
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
/// starts with no pipeline and no texture bound. The **camera and the sun/sky are
/// the exceptions: they are sticky**, retaining the last value set until it is
/// replaced, so the engine loop can push them once per frame *before* the game opens
/// its frame (the values survive `begin_frame`). A frame that never sets a
/// view-projection draws with the identity matrix (the construction default); one
/// that never sets a sun/sky is lit by the backend's default [`SunSky`].
///
/// # Frame-wide state vs. resources
///
/// The sticky setters — [`set_view_projection`](Self::set_view_projection),
/// [`set_camera_position`](Self::set_camera_position) and
/// [`set_sun_sky`](Self::set_sun_sky) — live here rather than on
/// [`RenderDevice`](crate::RenderDevice) because none of them creates or owns
/// anything: they are *per-frame values* a backend folds into the uniforms it
/// uploads for the frame it is recording. `RenderDevice` is for resources whose
/// lifetime a caller manages with a handle.
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

    /// Set the camera's **world-space position** for this frame's draws (KE-0406).
    ///
    /// The view-projection matrix alone is not enough to shade a frame: any
    /// view-dependent term — specular highlights, and anything else that needs to
    /// know where the eye is — needs the camera's *position* in world space, and
    /// recovering it by inverting the view-projection is both wasteful and
    /// numerically fragile. So the caller that already owns the camera pushes it
    /// here, as a plain [`Vec3`] from [`kaman_math::glam`].
    ///
    /// # Contract (ordering)
    /// - **Describes the same camera** as the most recent
    ///   [`set_view_projection`](Self::set_view_projection). A backend may light the
    ///   frame with one and project it with the other, so pushing a position that
    ///   disagrees with the matrix produces highlights that come from the wrong
    ///   place. The engine loop pushes both together, once per frame.
    /// - The value is **sticky**, exactly like the view-projection: it persists
    ///   across `begin_frame`/`submit` until replaced.
    /// - Until the first call, the implementation uses the world origin.
    fn set_camera_position(&mut self, position: Vec3);

    /// Set the **sun and sky** the frame is lit by (KE-0406).
    ///
    /// [`SunSky`] is the whole lighting description the seam accepts: the sun's
    /// elevation/azimuth in degrees, its colour and intensity, the ambient sky-fill
    /// level, and the sky gradient's zenith/horizon colours. The backend derives the
    /// light direction from the angles ([`SunSky::direction`]) and uses that one
    /// vector for **both** the shading and the sun disc it draws in the sky, so
    /// turning the sun moves the light and the disc together.
    ///
    /// The seam carries no time of day: a game that wants "summer, 4pm" owns that
    /// policy and expresses it as angles.
    ///
    /// # Contract (ordering)
    /// - The value is **sticky** — it persists across `begin_frame`/`submit` until
    ///   replaced — so a game with a fixed sun pushes it once at load, and a game
    ///   with a day/night cycle pushes it every frame *before* it opens the frame.
    /// - Backends honour it **per frame**: whatever is in effect when a frame opens
    ///   lights that frame, including its sky pass.
    /// - Until the first call, the implementation uses the default [`SunSky`].
    fn set_sun_sky(&mut self, sun: &SunSky);

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

    /// Record a screen-space quad into this frame's **2D overlay** (KE-0404).
    ///
    /// Overlay quads are not part of the 3D scene: the backend collects them
    /// during the frame and flushes them in a single orthographic pass at
    /// [`submit`](Self::submit) — **after** all 3D draws, with no depth test or
    /// write and alpha blending on — so the HUD composites on top of the scene
    /// regardless of when it was recorded.
    ///
    /// Coordinates are pixels with the origin at the drawable's top-left; see the
    /// [`overlay`](crate::overlay) module. Use
    /// [`FontAtlas::layout`](crate::overlay::FontAtlas::layout) to turn a string
    /// into a run of these without allocating.
    ///
    /// # Contract
    /// - Requires an open frame (after [`begin_frame`](Self::begin_frame), before
    ///   [`submit`](Self::submit)).
    /// - Independent of [`set_pipeline`](Self::set_pipeline) /
    ///   [`bind_texture`](Self::bind_texture): the overlay pass owns its own
    ///   pipeline, and each quad names the texture it samples via its
    ///   [`fill`](crate::overlay::OverlayQuad::fill).
    /// - Quads composite in **record order**, so later quads draw over earlier
    ///   ones.
    fn draw_overlay_quad(&mut self, quad: &OverlayQuad);

    /// Close the current frame and present it.
    ///
    /// # Contract
    /// - Requires an open frame; ends it.
    /// - After this call, no frame is open — a new frame requires
    ///   [`begin_frame`](Self::begin_frame) again.
    /// - Calling it without an open frame is a caller error.
    fn submit(&mut self);
}
