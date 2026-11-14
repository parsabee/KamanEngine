// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`StickShiftContext`]: the busy guard, the event queue and callback dispatch, the
//! per-call housekeeping, and the context and event functions.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use stickshift_math::glam::{Mat4, Vec3};
use stickshift_render_api::{RenderSettings, ShadowQuality, SunSky};

use crate::backend::{Backend, NullBackend, PendingReadback};
use crate::core::{call, call_or, input, output, Error};
use crate::fonts::FontEntry;
use crate::frame::Recording;
use crate::handles::{Kind, Slots};
use crate::models::{Loader, ModelEntry};
use crate::resources::{MeshEntry, TextureEntry};
use crate::types::*;

/// The most frames the GPU may have in flight.
pub(crate) const MAX_FRAMES_IN_FLIGHT: u64 = 3;

/// The largest drawable or texture side, in pixels.
pub(crate) const MAX_DIMENSION: u32 = 16384;

/// An opaque rendering context. See `docs/C_API.md`.
pub struct StickShiftContext {
    /// Set while a call is inside the context; a second, overlapping call gets
    /// `STICKSHIFT_ERROR_BUSY`.
    busy: AtomicBool,
    /// Callbacks being delivered (re-entrant calls are allowed; destroy is not).
    dispatching: AtomicU32,
    device_name: CString,
    inner: UnsafeCell<Inner>,
    /// The one frame object, handed out by `stickshift_frame_acquire`. Boxed so its
    /// address is stable for the context's lifetime.
    frame: Box<StickShiftFrame>,
}

/// The frame being recorded on a context. See `docs/C_API.md`.
pub struct StickShiftFrame {
    ctx: *const StickShiftContext,
}

/// An event owned by the context until it is delivered.
pub(crate) struct OwnedEvent {
    pub(crate) kind: StickShiftEventType,
    pub(crate) result: StickShiftResult,
    pub(crate) object: u64,
    pub(crate) frame: u64,
    pub(crate) message: Option<CString>,
}

impl OwnedEvent {
    pub(crate) fn new(kind: StickShiftEventType, object: u64, frame: u64) -> Self {
        Self {
            kind,
            result: STICKSHIFT_OK,
            object,
            frame,
            message: None,
        }
    }

    pub(crate) fn failed(
        kind: StickShiftEventType,
        object: u64,
        frame: u64,
        error: &Error,
    ) -> Self {
        Self {
            kind,
            result: error.code,
            object,
            frame,
            message: CString::new(error.message.replace('\0', " ")).ok(),
        }
    }

    fn as_c(&self) -> StickShiftEvent {
        StickShiftEvent {
            kind: self.kind,
            result: self.result,
            object: self.object,
            frame: self.frame,
            message: self
                .message
                .as_ref()
                .map_or(std::ptr::null(), |m| m.as_ptr()),
        }
    }
}

/// Callbacks and releases to run once the busy guard is dropped.
#[derive(Default)]
pub(crate) struct Due {
    callbacks: Vec<(StickShiftCompletion, OwnedEvent)>,
    releases: Vec<StickShiftData>,
}

impl Due {
    fn is_empty(&self) -> bool {
        self.callbacks.is_empty() && self.releases.is_empty()
    }

    /// Run everything due on this thread.
    fn run(self) {
        for (completion, event) in self.callbacks {
            if let Some(callback) = completion.callback {
                let event = event.as_c();
                // SAFETY: the host supplied this callback for exactly this call;
                // `event` (and its message) live until it returns.
                unsafe { callback(completion.user, &event) };
            }
        }
        for data in self.releases {
            if let Some(release) = data.release {
                // SAFETY: the host lent these bytes with this release function,
                // and the engine is done with them; called exactly once.
                unsafe { release(data.user, data.ptr, data.len) };
            }
        }
    }
}

/// A requested pixel copy waiting for its frame to finish.
pub(crate) struct PendingRead {
    pub(crate) frame: u64,
    pub(crate) copy: Box<dyn PendingReadback>,
    pub(crate) dst: *mut u8,
    pub(crate) len: usize,
    pub(crate) completion: Option<StickShiftCompletion>,
}

/// Counters not derived from the tables.
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) frames_skipped: u64,
    pub(crate) draws_submitted: u64,
    pub(crate) draws_skipped: u64,
}

/// Everything behind the busy guard.
pub(crate) struct Inner {
    pub(crate) backend: Box<dyn Backend>,
    pub(crate) meshes: Slots<MeshEntry>,
    pub(crate) textures: Slots<TextureEntry>,
    pub(crate) models: Slots<ModelEntry>,
    pub(crate) fonts: Slots<FontEntry>,
    /// Events waiting for `stickshift_poll_events`.
    queue: VecDeque<OwnedEvent>,
    /// Messages of the last poll batch; valid until the next poll.
    delivered: Vec<CString>,
    pub(crate) due: Due,
    pub(crate) loader: Option<Loader>,
    pub(crate) recording: Recording,
    pub(crate) frame_open: bool,
    /// Sticky camera: view-projection and eye position.
    pub(crate) camera: (Mat4, Vec3),
    /// Sticky sun and sky.
    pub(crate) sun: SunSky,
    /// Submitted frames not yet reported complete, oldest first.
    pub(crate) pending_frames: VecDeque<(u64, Option<StickShiftCompletion>)>,
    pub(crate) pending_reads: Vec<PendingRead>,
    pub(crate) counters: Counters,
}

impl Inner {
    /// Queue `event` for its completion callback, or for polling.
    pub(crate) fn emit(&mut self, event: OwnedEvent, completion: Option<StickShiftCompletion>) {
        match completion.filter(|c| c.callback.is_some()) {
            Some(completion) => self.due.callbacks.push((completion, event)),
            None => self.queue.push_back(event),
        }
    }

    /// Release borrowed `data` at the end of the current call (a no-op for
    /// copied data).
    pub(crate) fn release(&mut self, data: StickShiftData) {
        if data.release.is_some() {
            self.due.releases.push(data);
        }
    }

    /// Housekeeping: finish model loads, report finished frames, and copy out
    /// finished read-backs.
    fn pump(&mut self) {
        crate::models::pump(self);

        let completed = self.backend.completed_frames();
        while let Some(&(frame, completion)) = self.pending_frames.front() {
            if frame > completed {
                break;
            }
            self.pending_frames.pop_front();
            self.emit(
                OwnedEvent::new(STICKSHIFT_EVENT_FRAME_COMPLETED, 0, frame),
                completion,
            );
        }

        let mut i = 0;
        while i < self.pending_reads.len() {
            if !self.pending_reads[i].copy.is_complete() {
                i += 1;
                continue;
            }
            let read = self.pending_reads.swap_remove(i);
            // SAFETY: the host promised `dst` stays valid for `len` bytes until
            // this frame's read-back event, which is delivered after this copy.
            let dst = unsafe { std::slice::from_raw_parts_mut(read.dst, read.len) };
            let event = if read.copy.copy_to(dst) {
                OwnedEvent::new(STICKSHIFT_EVENT_READBACK_COMPLETE, 0, read.frame)
            } else {
                OwnedEvent::failed(
                    STICKSHIFT_EVENT_READBACK_FAILED,
                    0,
                    read.frame,
                    &Error::new(STICKSHIFT_ERROR_INTERNAL, "the read-back copy failed"),
                )
            };
            self.emit(event, read.completion);
        }
    }

    /// Whether nothing is in progress: no loads, no frames or read-backs on the GPU.
    fn idle(&self) -> bool {
        self.loader.as_ref().is_none_or(|l| l.in_flight() == 0)
            && self.pending_frames.is_empty()
            && self.pending_reads.is_empty()
    }

    pub(crate) fn surface_size(&self) -> (u32, u32) {
        self.backend.surface_size()
    }
}

/// Holds the busy flag for the duration of a call.
pub(crate) struct Guard<'a> {
    ctx: &'a StickShiftContext,
}

impl Guard<'_> {
    pub(crate) fn inner(&mut self) -> &mut Inner {
        // SAFETY: the busy flag gives this guard exclusive access to `inner`.
        unsafe { &mut *self.ctx.inner.get() }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.ctx.busy.store(false, Ordering::Release);
    }
}

impl StickShiftContext {
    pub(crate) fn enter(&self) -> Result<Guard<'_>, Error> {
        self.busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Guard { ctx: self })
            .map_err(|_| {
                Error::new(
                    STICKSHIFT_ERROR_BUSY,
                    "another call on this context is in progress",
                )
            })
    }

    fn dispatch(&self, due: Due) {
        if due.is_empty() {
            return;
        }
        self.dispatching.fetch_add(1, Ordering::SeqCst);
        due.run();
        self.dispatching.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Run `body` on the context behind its busy guard, then deliver the callbacks
/// and releases it queued, after the guard is dropped (so they may call back in).
///
/// # Safety
/// `ctx` must be NULL or a live context.
pub(crate) unsafe fn with_context<T>(
    ctx: *mut StickShiftContext,
    body: impl FnOnce(&mut Inner) -> Result<T, Error>,
) -> Result<T, Error> {
    let ctx = input(ctx.cast_const(), "ctx")?;
    let (result, due) = {
        let mut guard = ctx.enter()?;
        let inner = guard.inner();
        let result = body(inner);
        (result, std::mem::take(&mut inner.due))
    };
    ctx.dispatch(due);
    result
}

/// Like [`with_context`] for a frame: resolves its context and requires the
/// frame to be open.
///
/// # Safety
/// `frame` must be NULL or a frame from a live context.
pub(crate) unsafe fn with_frame<T>(
    frame: *mut StickShiftFrame,
    body: impl FnOnce(&mut Inner) -> Result<T, Error>,
) -> Result<T, Error> {
    let frame = input(frame.cast_const(), "frame")?;
    with_context(frame.ctx.cast_mut(), |inner| {
        if !inner.frame_open {
            return Err(Error::wrong_state(
                "the frame is not open: acquire one with stickshift_frame_acquire",
            ));
        }
        body(inner)
    })
}

/// The frame object of `ctx`, for `stickshift_frame_acquire`.
pub(crate) fn frame_of(ctx: &StickShiftContext) -> *mut StickShiftFrame {
    std::ptr::from_ref::<StickShiftFrame>(&ctx.frame).cast_mut()
}

fn shadow_quality(value: StickShiftShadowQuality) -> Result<ShadowQuality, Error> {
    match value {
        STICKSHIFT_SHADOW_QUALITY_OFF => Ok(ShadowQuality::Off),
        STICKSHIFT_SHADOW_QUALITY_LOW => Ok(ShadowQuality::Low),
        STICKSHIFT_SHADOW_QUALITY_HIGH => Ok(ShadowQuality::High),
        other => Err(Error::invalid_argument(format!(
            "{other} is not a STICKSHIFT_SHADOW_QUALITY_* value"
        ))),
    }
}

impl From<RenderSettings> for StickShiftRenderSettings {
    fn from(s: RenderSettings) -> Self {
        Self {
            shadows: match s.shadows {
                ShadowQuality::Off => STICKSHIFT_SHADOW_QUALITY_OFF,
                ShadowQuality::Low => STICKSHIFT_SHADOW_QUALITY_LOW,
                ShadowQuality::High => STICKSHIFT_SHADOW_QUALITY_HIGH,
            },
            shadow_distance: s.shadow_distance.unwrap_or(0.0),
            draw_distance_scale: s.draw_distance_scale,
        }
    }
}

fn render_settings(s: &StickShiftRenderSettings) -> Result<RenderSettings, Error> {
    let shadows = shadow_quality(s.shadows)?;
    if !(s.draw_distance_scale.is_finite() && s.draw_distance_scale > 0.0) {
        return Err(Error::invalid_argument(
            "draw_distance_scale must be finite and positive",
        ));
    }
    let shadow_distance = match s.shadow_distance {
        d if d.is_nan() || d > 1.0 => {
            return Err(Error::invalid_argument(
                "shadow_distance must be <= 0 (match the draw distance) or in (0, 1]",
            ))
        }
        d if d <= 0.0 => None,
        d => Some(d),
    };
    Ok(RenderSettings {
        shadows,
        shadow_distance,
        draw_distance_scale: s.draw_distance_scale,
    })
}

fn check_dimensions(width: u32, height: u32) -> Result<(), Error> {
    if (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height) {
        Ok(())
    } else {
        Err(Error::invalid_argument(format!(
            "{width}x{height}: each side must be 1..={MAX_DIMENSION}"
        )))
    }
}

/// The backend `desc` asks for.
///
/// # Safety
/// `desc.metal_layer` must be NULL or a live Objective-C object.
unsafe fn create_backend(desc: &StickShiftContextDesc) -> Result<Box<dyn Backend>, Error> {
    match desc.backend {
        STICKSHIFT_BACKEND_NULL => {
            if !desc.metal_layer.is_null() {
                return Err(Error::invalid_argument(
                    "metal_layer must be NULL with STICKSHIFT_BACKEND_NULL",
                ));
            }
            Ok(Box::new(NullBackend::new(desc.width, desc.height)))
        }
        STICKSHIFT_BACKEND_METAL => metal_backend(desc),
        other => Err(Error::invalid_argument(format!(
            "{other} is not a STICKSHIFT_BACKEND_* value"
        ))),
    }
}

#[cfg(target_os = "macos")]
unsafe fn metal_backend(desc: &StickShiftContextDesc) -> Result<Box<dyn Backend>, Error> {
    use crate::metal::MetalBackend;
    let backend = if desc.metal_layer.is_null() {
        MetalBackend::offscreen(desc.width, desc.height)?
    } else {
        MetalBackend::layer(desc.metal_layer, desc.width, desc.height, desc.scale)?
    };
    Ok(Box::new(backend))
}

#[cfg(not(target_os = "macos"))]
unsafe fn metal_backend(_: &StickShiftContextDesc) -> Result<Box<dyn Backend>, Error> {
    Err(Error::new(
        STICKSHIFT_ERROR_NO_DEVICE,
        "this build has no Metal backend (macOS only); use STICKSHIFT_BACKEND_NULL",
    ))
}

/// Create a context. On success `*out` is the new context, with
/// `STICKSHIFT_EVENT_CONTEXT_READY` already queued.
///
/// # Safety
/// `desc` and `out` must be NULL or valid; `desc->metal_layer` must be NULL or
/// a live `CAMetalLayer *`.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_create(
    desc: *const StickShiftContextDesc,
    out: *mut *mut StickShiftContext,
) -> StickShiftResult {
    call(|| {
        let desc = input(desc, "desc")?;
        let out = output(out, "out")?;
        if desc.api_version != STICKSHIFT_API_VERSION {
            return Err(Error::new(
                STICKSHIFT_ERROR_UNSUPPORTED,
                format!(
                    "the header is API version {} but the library is {STICKSHIFT_API_VERSION}",
                    desc.api_version
                ),
            ));
        }
        check_dimensions(desc.width, desc.height)?;
        let backend = create_backend(desc)?;
        let device_name =
            CString::new(backend.device_name().replace('\0', " ")).unwrap_or_default();
        let mut inner = Inner {
            backend,
            meshes: Slots::new(Kind::Mesh),
            textures: Slots::new(Kind::Texture),
            models: Slots::new(Kind::Model),
            fonts: Slots::new(Kind::Font),
            queue: VecDeque::new(),
            delivered: Vec::new(),
            due: Due::default(),
            loader: None,
            recording: Recording::default(),
            frame_open: false,
            camera: (Mat4::IDENTITY, Vec3::ZERO),
            sun: SunSky::default(),
            pending_frames: VecDeque::new(),
            pending_reads: Vec::new(),
            counters: Counters::default(),
        };
        inner.emit(OwnedEvent::new(STICKSHIFT_EVENT_CONTEXT_READY, 0, 0), None);
        let ctx = Box::into_raw(Box::new(StickShiftContext {
            busy: AtomicBool::new(false),
            dispatching: AtomicU32::new(0),
            device_name,
            inner: UnsafeCell::new(inner),
            frame: Box::new(StickShiftFrame {
                ctx: std::ptr::null(),
            }),
        }));
        // SAFETY: just allocated; the frame points back at its context.
        (*ctx).frame.ctx = ctx;
        *out = ctx;
        Ok(STICKSHIFT_OK)
    })
}

/// Destroy a context and everything it owns. Waits for submitted frames and a
/// model parse in progress; undelivered events are dropped and borrowed data is
/// released. NULL is a no-op.
///
/// # Safety
/// `ctx` must be NULL or a live context; it is invalid after this returns
/// `STICKSHIFT_OK`.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_destroy(
    ctx: *mut StickShiftContext,
) -> StickShiftResult {
    call(|| {
        let Some(context) = ctx.as_ref() else {
            return Ok(STICKSHIFT_OK);
        };
        if context.dispatching.load(Ordering::SeqCst) > 0 {
            return Err(Error::wrong_state(
                "stickshift_context_destroy cannot be called from a callback",
            ));
        }
        {
            let mut guard = context.enter()?;
            let inner = guard.inner();
            if let Some(loader) = inner.loader.take() {
                for borrowed in loader.shutdown() {
                    inner.release(borrowed);
                }
            }
            // Let the GPU finish what it was given before its resources go.
            for _ in 0..10_000 {
                if inner.backend.completed_frames() >= inner.backend.submitted_frames() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            // Run releases while still holding the guard, so a release callback
            // that calls back into the dying context gets STICKSHIFT_ERROR_BUSY.
            // Undelivered callbacks are dropped with the context.
            Due {
                callbacks: Vec::new(),
                releases: std::mem::take(&mut inner.due.releases),
            }
            .run();
        }
        drop(Box::from_raw(ctx));
        Ok(STICKSHIFT_OK)
    })
}

/// Resize a layer context's drawable to `width` × `height` pixels with `scale`
/// pixels per point (`<= 0`: 1). Offscreen contexts keep their size
/// (`STICKSHIFT_ERROR_UNSUPPORTED`).
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_resize(
    ctx: *mut StickShiftContext,
    width: u32,
    height: u32,
    scale: f32,
) -> StickShiftResult {
    call(|| {
        with_context(ctx, |inner| {
            if inner.backend.is_offscreen() {
                return Err(Error::new(
                    STICKSHIFT_ERROR_UNSUPPORTED,
                    "an offscreen context has a fixed size",
                ));
            }
            check_dimensions(width, height)?;
            let scale = if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            };
            inner.backend.resize(width, height, scale);
            Ok(STICKSHIFT_OK)
        })
    })
}

/// The drawable size in pixels.
///
/// # Safety
/// `ctx` must be NULL or a live context; `width` and `height` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_get_size(
    ctx: *mut StickShiftContext,
    width: *mut u32,
    height: *mut u32,
) -> StickShiftResult {
    call(|| {
        let width = output(width, "width")?;
        let height = output(height, "height")?;
        with_context(ctx, |inner| {
            (*width, *height) = inner.surface_size();
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Apply render settings from the next frame.
///
/// # Safety
/// `ctx` must be NULL or a live context; `settings` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_set_settings(
    ctx: *mut StickShiftContext,
    settings: *const StickShiftRenderSettings,
) -> StickShiftResult {
    call(|| {
        let settings = render_settings(input(settings, "settings")?)?;
        with_context(ctx, |inner| {
            inner.backend.set_settings(&settings);
            Ok(STICKSHIFT_OK)
        })
    })
}

/// The current render settings.
///
/// # Safety
/// `ctx` must be NULL or a live context; `out` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_get_settings(
    ctx: *mut StickShiftContext,
    out: *mut StickShiftRenderSettings,
) -> StickShiftResult {
    call(|| {
        let out = output(out, "out")?;
        with_context(ctx, |inner| {
            *out = inner.backend.settings().into();
            Ok(STICKSHIFT_OK)
        })
    })
}

/// The default render settings: high shadows matching the draw distance, draw
/// distance scale 1. NULL is a no-op. Thread-safe.
///
/// # Safety
/// `out` must be NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_render_settings_default(out: *mut StickShiftRenderSettings) {
    if let Some(out) = out.as_mut() {
        *out = RenderSettings::default().into();
    }
}

/// Counters for diagnostics and tests.
///
/// # Safety
/// `ctx` must be NULL or a live context; `out` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_get_stats(
    ctx: *mut StickShiftContext,
    out: *mut StickShiftStats,
) -> StickShiftResult {
    call(|| {
        let out = output(out, "out")?;
        with_context(ctx, |inner| {
            *out = StickShiftStats {
                frames_submitted: inner.backend.submitted_frames(),
                frames_completed: inner.backend.completed_frames(),
                frames_skipped: inner.counters.frames_skipped,
                draws_submitted: inner.counters.draws_submitted,
                draws_skipped: inner.counters.draws_skipped,
                meshes: inner.meshes.len(),
                textures: inner.textures.len(),
                models: inner.models.len(),
                fonts: inner.fonts.len(),
                loads_pending: inner.loader.as_ref().map_or(0, Loader::in_flight),
            };
            Ok(STICKSHIFT_OK)
        })
    })
}

/// The GPU's name, valid for the context's lifetime; `""` for NULL. Thread-safe.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_device_name(
    ctx: *mut StickShiftContext,
) -> *const c_char {
    match ctx.as_ref() {
        Some(ctx) => ctx.device_name.as_ptr(),
        None => c"".as_ptr(),
    }
}

/// Do the context's housekeeping, deliver callbacks, then copy up to `capacity`
/// queued events into `events` and their count into `*out_count`. Events that
/// don't fit stay queued.
///
/// Callbacks run before anything is copied, so a callback may poll too: its
/// poll finishes before this one hands out any events, and so it can't free
/// their messages.
///
/// # Safety
/// `ctx` must be NULL or a live context; `events` NULL (with `capacity` 0) or
/// valid for `capacity` events; `out_count` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn stickshift_poll_events(
    ctx: *mut StickShiftContext,
    events: *mut StickShiftEvent,
    capacity: u32,
    out_count: *mut u32,
) -> StickShiftResult {
    call(|| {
        let out_count = output(out_count, "out_count")?;
        if events.is_null() && capacity > 0 {
            return Err(Error::null("events"));
        }
        *out_count = 0;
        // 1. Housekeeping. The messages handed out by the previous poll are
        //    freed here. `with_context` delivers the callbacks it makes due as
        //    it returns, with the busy guard dropped, so a callback may call
        //    back in, even into this function.
        with_context(ctx, |inner| {
            inner.delivered.clear();
            inner.pump();
            Ok(())
        })?;
        // 2. Copy. This runs after every callback has returned, so a poll made
        //    from a callback can't free the messages handed out below: it has
        //    already finished. They stay valid until the next poll.
        with_context(ctx, |inner| {
            let n = (capacity as usize).min(inner.queue.len());
            for i in 0..n {
                let event = inner.queue.pop_front().expect("n <= queue length");
                events.add(i).write(event.as_c());
                if let Some(message) = event.message {
                    // Moving the CString keeps its heap buffer (and the pointer
                    // just copied out) in place until the next poll.
                    inner.delivered.push(message);
                }
            }
            *out_count = n as u32;
            Ok(STICKSHIFT_OK)
        })
    })
}

/// **Wait** for every model load and submitted frame to finish, delivering
/// their callbacks; their poll events are queued. Tests and tools only.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_context_flush(ctx: *mut StickShiftContext) -> StickShiftResult {
    call(|| loop {
        let idle = with_context(ctx, |inner| {
            inner.pump();
            Ok(inner.idle())
        })?;
        if idle {
            return Ok(STICKSHIFT_OK);
        }
        std::thread::sleep(Duration::from_millis(1));
    })
}

/// The state of any handle id: `INVALID` for an unknown handle, a NULL
/// context, or a context busy with another call.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_resource_state(
    ctx: *mut StickShiftContext,
    handle_id: u64,
) -> StickShiftResourceState {
    call_or(STICKSHIFT_RESOURCE_STATE_INVALID, || {
        with_context(ctx, |inner| Ok(crate::resources::state(inner, handle_id)))
            .unwrap_or(STICKSHIFT_RESOURCE_STATE_INVALID)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_enter_is_busy_until_the_first_guard_drops() {
        let desc = StickShiftContextDesc {
            api_version: STICKSHIFT_API_VERSION,
            backend: STICKSHIFT_BACKEND_NULL,
            metal_layer: std::ptr::null_mut(),
            width: 4,
            height: 4,
            scale: 1.0,
        };
        let mut ctx = std::ptr::null_mut();
        // SAFETY: valid pointers to locals.
        assert_eq!(
            unsafe { stickshift_context_create(&desc, &mut ctx) },
            STICKSHIFT_OK
        );
        // SAFETY: just created.
        let context = unsafe { &*ctx };
        let guard = context.enter().expect("first entry");
        assert_eq!(
            context.enter().err().map(|e| e.code),
            Some(STICKSHIFT_ERROR_BUSY)
        );
        drop(guard);
        assert!(context.enter().is_ok());
        // SAFETY: no guard is held.
        assert_eq!(unsafe { stickshift_context_destroy(ctx) }, STICKSHIFT_OK);
    }

    #[test]
    fn settings_are_validated() {
        let ok = StickShiftRenderSettings {
            shadows: STICKSHIFT_SHADOW_QUALITY_LOW,
            shadow_distance: 0.5,
            draw_distance_scale: 2.0,
        };
        let s = render_settings(&ok).unwrap();
        assert_eq!(s.shadows, ShadowQuality::Low);
        assert_eq!(s.shadow_distance, Some(0.5));
        let matching = StickShiftRenderSettings {
            shadow_distance: -1.0,
            ..ok
        };
        assert_eq!(render_settings(&matching).unwrap().shadow_distance, None);
        for bad in [
            StickShiftRenderSettings { shadows: 9, ..ok },
            StickShiftRenderSettings {
                shadow_distance: 1.5,
                ..ok
            },
            StickShiftRenderSettings {
                shadow_distance: f32::NAN,
                ..ok
            },
            StickShiftRenderSettings {
                draw_distance_scale: 0.0,
                ..ok
            },
        ] {
            assert!(render_settings(&bad).is_err(), "{bad:?}");
        }
    }
}
