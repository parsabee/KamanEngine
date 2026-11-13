// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! End-to-end tests of the C API through its exported functions.
//!
//! Most run on the null backend, so they run on any machine, GPU or not. The
//! last test renders through Metal (macOS, skipped without a device) and checks
//! the C API produces the same pixels as the Rust API for the same scene.

use std::ffi::{c_void, CStr, CString};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};

use kaman::*;

const CUBE: &[u8] = include_bytes!("../../kaman-assets/tests/fixtures/cube.gltf");
const TEXTURED_CUBE: &[u8] = include_bytes!("../../kaman-assets/tests/fixtures/textured_cube.gltf");

fn desc(backend: KamanBackend, width: u32, height: u32) -> KamanContextDesc {
    KamanContextDesc {
        api_version: KAMAN_API_VERSION,
        backend,
        metal_layer: null_mut(),
        width,
        height,
        scale: 1.0,
    }
}

/// A null-backend context, destroyed when dropped.
struct Ctx(*mut KamanContext);

impl Ctx {
    fn null(width: u32, height: u32) -> Self {
        let mut ctx = null_mut();
        // SAFETY: valid pointers to locals.
        let r = unsafe { kaman_context_create(&desc(KAMAN_BACKEND_NULL, width, height), &mut ctx) };
        assert_eq!(r, KAMAN_OK, "{}", last_error());
        Ctx(ctx)
    }

    fn poll(&self) -> Vec<Event> {
        let mut events = [KamanEvent {
            kind: 0,
            result: 0,
            object: 0,
            frame: 0,
            message: null(),
        }; 64];
        let mut n = 0;
        // SAFETY: a live context and a 64-event buffer.
        assert_eq!(
            unsafe { kaman_poll_events(self.0, events.as_mut_ptr(), 64, &mut n) },
            KAMAN_OK
        );
        events[..n as usize].iter().map(Event::from).collect()
    }

    fn state(&self, id: u64) -> KamanResourceState {
        // SAFETY: a live context.
        unsafe { kaman_resource_state(self.0, id) }
    }

    fn stats(&self) -> KamanStats {
        let mut stats = KamanStats::default();
        // SAFETY: a live context.
        assert_eq!(
            unsafe { kaman_context_get_stats(self.0, &mut stats) },
            KAMAN_OK
        );
        stats
    }

    fn flush(&self) {
        // SAFETY: a live context.
        assert_eq!(unsafe { kaman_context_flush(self.0) }, KAMAN_OK);
    }

    fn acquire(&self) -> *mut KamanFrame {
        let mut frame = null_mut();
        // SAFETY: a live context.
        assert_eq!(unsafe { kaman_frame_acquire(self.0, &mut frame) }, KAMAN_OK);
        assert!(!frame.is_null());
        frame
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: created by `Ctx::null`, not used after this.
        let r = unsafe { kaman_context_destroy(self.0) };
        if !std::thread::panicking() {
            assert_eq!(r, KAMAN_OK);
        }
    }
}

/// An owned copy of a delivered event.
#[derive(Debug, Clone, PartialEq)]
struct Event {
    kind: KamanEventType,
    result: KamanResult,
    object: u64,
    frame: u64,
    message: Option<String>,
}

impl From<&KamanEvent> for Event {
    fn from(e: &KamanEvent) -> Self {
        Self {
            kind: e.kind,
            result: e.result,
            object: e.object,
            frame: e.frame,
            // SAFETY: the message is valid until the next poll.
            message: (!e.message.is_null()).then(|| {
                unsafe { CStr::from_ptr(e.message) }
                    .to_string_lossy()
                    .into_owned()
            }),
        }
    }
}

fn last_error() -> String {
    // SAFETY: never NULL, valid until the next error on this thread.
    unsafe { CStr::from_ptr(kaman_last_error_message()) }
        .to_string_lossy()
        .into_owned()
}

fn copied(bytes: &[u8]) -> KamanData {
    KamanData {
        ptr: bytes.as_ptr().cast(),
        len: bytes.len(),
        release: None,
        user: null_mut(),
    }
}

unsafe extern "C" fn count_release(user: *mut c_void, _: *const c_void, _: usize) {
    (*user.cast::<AtomicUsize>()).fetch_add(1, Ordering::SeqCst);
}

fn borrowed(bytes: &[u8], releases: &AtomicUsize) -> KamanData {
    KamanData {
        release: Some(count_release),
        user: std::ptr::from_ref(releases).cast_mut().cast(),
        ..copied(bytes)
    }
}

/// One triangle on the `POS_NORMAL_COLOR` layout, in `color`.
fn triangle(color: [f32; 3]) -> (Vec<u8>, Vec<u8>) {
    let mut vertices = Vec::new();
    for p in [[-0.5f32, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]] {
        for f in p.iter().chain(&[0.0, 0.0, 1.0]).chain(&color) {
            vertices.extend_from_slice(&f.to_ne_bytes());
        }
    }
    let indices = [0u32, 1, 2].iter().flat_map(|i| i.to_ne_bytes()).collect();
    (vertices, indices)
}

fn mesh_desc(vertices: KamanData, indices: KamanData) -> KamanMeshDesc {
    KamanMeshDesc {
        layout: KAMAN_VERTEX_LAYOUT_POS_NORMAL_COLOR,
        vertices,
        indices,
    }
}

fn make_mesh(ctx: &Ctx) -> KamanMesh {
    let (v, i) = triangle([1.0, 0.0, 0.0]);
    let mut mesh = KamanMesh::default();
    // SAFETY: valid pointers; data copied.
    let r =
        unsafe { kaman_mesh_create(ctx.0, &mesh_desc(copied(&v), copied(&i)), null(), &mut mesh) };
    assert_eq!(r, KAMAN_OK, "{}", last_error());
    mesh
}

fn make_texture(ctx: &Ctx, width: u32, height: u32, color_space: KamanColorSpace) -> KamanTexture {
    let pixels = vec![255u8; (width * height * 4) as usize];
    let mut texture = KamanTexture::default();
    let desc = KamanTextureDesc {
        width,
        height,
        rgba8: copied(&pixels),
        color_space,
        flags: 0,
    };
    // SAFETY: valid pointers; data copied.
    assert_eq!(
        unsafe { kaman_texture_create(ctx.0, &desc, null(), &mut texture) },
        KAMAN_OK
    );
    texture
}

fn identity() -> KamanMat4 {
    kaman_mat4_identity()
}

#[test]
fn versions_and_result_names() {
    assert_eq!(kaman_api_version(), KAMAN_API_VERSION);
    // SAFETY: static strings.
    let version = unsafe { CStr::from_ptr(kaman_version_string()) }
        .to_str()
        .unwrap();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    let name = unsafe { CStr::from_ptr(kaman_result_name(KAMAN_NOT_READY)) };
    assert_eq!(name.to_str().unwrap(), "KAMAN_NOT_READY");
}

#[test]
fn context_creation_validates_its_description() {
    let mut ctx = null_mut();
    let cases = [
        (
            KamanContextDesc {
                api_version: KAMAN_API_VERSION + 1,
                ..desc(KAMAN_BACKEND_NULL, 8, 8)
            },
            KAMAN_ERROR_UNSUPPORTED,
        ),
        (desc(7, 8, 8), KAMAN_ERROR_INVALID_ARGUMENT),
        (desc(KAMAN_BACKEND_NULL, 0, 8), KAMAN_ERROR_INVALID_ARGUMENT),
        (
            desc(KAMAN_BACKEND_NULL, 8, 16385),
            KAMAN_ERROR_INVALID_ARGUMENT,
        ),
        (
            KamanContextDesc {
                metal_layer: std::ptr::dangling_mut::<c_void>(),
                ..desc(KAMAN_BACKEND_NULL, 8, 8)
            },
            KAMAN_ERROR_INVALID_ARGUMENT,
        ),
    ];
    for (d, expected) in cases {
        // SAFETY: valid pointers; the bogus layer pointer is rejected before use.
        assert_eq!(
            unsafe { kaman_context_create(&d, &mut ctx) },
            expected,
            "{d:?}"
        );
        assert!(ctx.is_null(), "no context on failure");
    }
    // SAFETY: NULL arguments are rejected.
    assert_eq!(
        unsafe { kaman_context_create(null(), &mut ctx) },
        KAMAN_ERROR_INVALID_ARGUMENT
    );
    assert!(last_error().contains("desc"), "{}", last_error());
    // SAFETY: destroying NULL is a no-op.
    assert_eq!(unsafe { kaman_context_destroy(null_mut()) }, KAMAN_OK);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_metal_backend_is_unavailable_off_apple_platforms() {
    let mut ctx = null_mut();
    // SAFETY: valid pointers.
    let r = unsafe { kaman_context_create(&desc(KAMAN_BACKEND_METAL, 8, 8), &mut ctx) };
    assert_eq!(r, KAMAN_ERROR_NO_DEVICE);
}

#[test]
fn a_new_context_is_ready_and_names_its_device() {
    let ctx = Ctx::null(8, 8);
    let events = ctx.poll();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, KAMAN_EVENT_CONTEXT_READY);
    // SAFETY: valid for the context's lifetime.
    let name = unsafe { CStr::from_ptr(kaman_context_device_name(ctx.0)) };
    assert!(name.to_str().unwrap().contains("null"));
    let (mut w, mut h) = (0, 0);
    // SAFETY: a live context.
    assert_eq!(
        unsafe { kaman_context_get_size(ctx.0, &mut w, &mut h) },
        KAMAN_OK
    );
    assert_eq!((w, h), (8, 8));
}

#[test]
fn meshes_are_ready_at_once_and_handles_are_never_reissued() {
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mesh = make_mesh(&ctx);
    assert_eq!(ctx.state(mesh.id), KAMAN_RESOURCE_STATE_READY);
    let events = ctx.poll();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].kind, events[0].object),
        (KAMAN_EVENT_RESOURCE_READY, mesh.id)
    );

    // SAFETY: a live context.
    unsafe {
        assert_eq!(kaman_mesh_destroy(ctx.0, mesh), KAMAN_OK);
        assert_eq!(ctx.state(mesh.id), KAMAN_RESOURCE_STATE_INVALID);
        assert_eq!(kaman_mesh_destroy(ctx.0, mesh), KAMAN_ERROR_INVALID_HANDLE);
        assert_eq!(
            kaman_mesh_destroy(ctx.0, KamanMesh { id: 0 }),
            KAMAN_ERROR_INVALID_HANDLE
        );
    }
    let again = make_mesh(&ctx);
    assert_ne!(again.id, mesh.id, "the freed slot gets a new generation");
    assert_eq!(ctx.state(mesh.id), KAMAN_RESOURCE_STATE_INVALID);
    // A mesh id is not a texture id.
    assert_eq!(
        unsafe { kaman_texture_destroy(ctx.0, KamanTexture { id: again.id }) },
        KAMAN_ERROR_INVALID_HANDLE
    );
}

#[test]
fn borrowed_data_is_released_exactly_once_and_never_on_failure() {
    let ctx = Ctx::null(8, 8);
    let (v, i) = triangle([0.0, 1.0, 0.0]);
    let releases = AtomicUsize::new(0);
    let mut mesh = KamanMesh::default();
    let ok = mesh_desc(borrowed(&v, &releases), borrowed(&i, &releases));
    // SAFETY: the borrowed buffers outlive the call.
    assert_eq!(
        unsafe { kaman_mesh_create(ctx.0, &ok, null(), &mut mesh) },
        KAMAN_OK
    );
    assert_eq!(
        releases.load(Ordering::SeqCst),
        2,
        "both buffers, before the call returned"
    );

    let bad = KamanMeshDesc {
        layout: 99,
        ..mesh_desc(borrowed(&v, &releases), borrowed(&i, &releases))
    };
    // SAFETY: as above.
    let r = unsafe { kaman_mesh_create(ctx.0, &bad, null(), &mut mesh) };
    assert_eq!(r, KAMAN_ERROR_INVALID_ARGUMENT);
    assert_eq!(
        releases.load(Ordering::SeqCst),
        2,
        "a failed call does not take the data"
    );
}

#[test]
fn malformed_meshes_and_textures_are_rejected() {
    let ctx = Ctx::null(8, 8);
    let (v, i) = triangle([0.0, 0.0, 1.0]);
    let mut mesh = KamanMesh::default();
    let out_of_range: Vec<u8> = [0u32, 1, 3].iter().flat_map(|x| x.to_ne_bytes()).collect();
    let null_with_length = KamanData {
        ptr: null(),
        len: 36,
        release: None,
        user: null_mut(),
    };
    for d in [
        mesh_desc(copied(&v[..v.len() - 1]), copied(&i)),
        mesh_desc(copied(&v), copied(&i[..8])),
        mesh_desc(copied(&v), copied(&out_of_range)),
        mesh_desc(null_with_length, copied(&i)),
    ] {
        // SAFETY: valid descriptors; the NULL data is rejected before use.
        assert_eq!(
            unsafe { kaman_mesh_create(ctx.0, &d, null(), &mut mesh) },
            KAMAN_ERROR_INVALID_ARGUMENT,
            "{}",
            last_error()
        );
    }
    let mut texture = KamanTexture::default();
    let short = KamanTextureDesc {
        width: 64,
        height: 64,
        rgba8: copied(&[0; 4]),
        color_space: KAMAN_COLOR_SPACE_SRGB,
        flags: 0,
    };
    // SAFETY: valid pointers.
    unsafe {
        assert_eq!(
            kaman_texture_create(ctx.0, &short, null(), &mut texture),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        let flagged = KamanTextureDesc {
            width: 1,
            height: 1,
            rgba8: copied(&[0; 4]),
            color_space: KAMAN_COLOR_SPACE_SRGB,
            flags: 1,
        };
        assert_eq!(
            kaman_texture_create(ctx.0, &flagged, null(), &mut texture),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        let unknown_space = KamanTextureDesc {
            width: 1,
            height: 1,
            rgba8: copied(&[0; 4]),
            color_space: 7,
            flags: 0,
        };
        assert_eq!(
            kaman_texture_create(ctx.0, &unknown_space, null(), &mut texture),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
    }
}

unsafe extern "C" fn record_event(user: *mut c_void, event: *const KamanEvent) {
    (*user.cast::<Vec<Event>>()).push(Event::from(&*event));
}

#[test]
fn a_completion_callback_receives_the_event_instead_of_the_poll_queue() {
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mut seen: Vec<Event> = Vec::new();
    let completion = KamanCompletion {
        callback: Some(record_event),
        user: std::ptr::from_mut(&mut seen).cast(),
    };
    let (v, i) = triangle([1.0, 1.0, 1.0]);
    let mut mesh = KamanMesh::default();
    // SAFETY: `seen` outlives the call.
    let r = unsafe {
        kaman_mesh_create(
            ctx.0,
            &mesh_desc(copied(&v), copied(&i)),
            &completion,
            &mut mesh,
        )
    };
    assert_eq!(r, KAMAN_OK);
    assert_eq!(seen.len(), 1, "delivered before the call returned");
    assert_eq!(
        (seen[0].kind, seen[0].object),
        (KAMAN_EVENT_RESOURCE_READY, mesh.id)
    );
    assert!(ctx.poll().is_empty(), "delivered exactly once");
}

struct Reentry {
    ctx: *mut KamanContext,
    state: KamanResourceState,
    destroy: KamanResult,
}

unsafe extern "C" fn reenter(user: *mut c_void, event: *const KamanEvent) {
    let r = &mut *user.cast::<Reentry>();
    r.state = kaman_resource_state(r.ctx, (*event).object);
    r.destroy = kaman_context_destroy(r.ctx);
}

#[test]
fn callbacks_may_call_back_in_but_not_destroy_the_context() {
    let ctx = Ctx::null(8, 8);
    let mut reentry = Reentry {
        ctx: ctx.0,
        state: 0,
        destroy: 0,
    };
    let completion = KamanCompletion {
        callback: Some(reenter),
        user: std::ptr::from_mut(&mut reentry).cast(),
    };
    let (v, i) = triangle([1.0, 1.0, 1.0]);
    let mut mesh = KamanMesh::default();
    // SAFETY: `reentry` outlives the call.
    unsafe {
        kaman_mesh_create(
            ctx.0,
            &mesh_desc(copied(&v), copied(&i)),
            &completion,
            &mut mesh,
        )
    };
    assert_eq!(
        reentry.state, KAMAN_RESOURCE_STATE_READY,
        "the guard is released for callbacks"
    );
    assert_eq!(reentry.destroy, KAMAN_ERROR_WRONG_STATE);
}

#[test]
fn frames_record_submit_and_complete() {
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mesh = make_mesh(&ctx);
    let frame = ctx.acquire();
    let mut second = null_mut();
    // SAFETY: a live context and an open frame.
    unsafe {
        assert_eq!(
            kaman_frame_acquire(ctx.0, &mut second),
            KAMAN_ERROR_WRONG_STATE
        );
        assert!(second.is_null());
        let camera = KamanCamera {
            view_projection: identity(),
            position: KamanVec3::default(),
        };
        assert_eq!(kaman_frame_set_camera(frame, &camera), KAMAN_OK);
        let mut sun = KamanSunSky::default();
        kaman_sun_sky_default(&mut sun);
        assert_eq!(kaman_frame_set_sun_sky(frame, &sun), KAMAN_OK);
        assert_eq!(
            kaman_frame_draw_mesh(frame, mesh, &identity(), null()),
            KAMAN_OK
        );
        assert_eq!(
            kaman_frame_draw_mesh(frame, KamanMesh { id: 12345 }, &identity(), null()),
            KAMAN_ERROR_INVALID_HANDLE
        );
        let mut number = 0;
        assert_eq!(kaman_frame_submit(frame, null(), &mut number), KAMAN_OK);
        assert_eq!(number, 1);
        // The frame is closed now.
        assert_eq!(
            kaman_frame_draw_mesh(frame, mesh, &identity(), null()),
            KAMAN_ERROR_WRONG_STATE
        );
    }
    let events = ctx.poll();
    assert!(
        events
            .iter()
            .any(|e| e.kind == KAMAN_EVENT_FRAME_COMPLETED && e.frame == 1),
        "{events:?}"
    );
    let stats = ctx.stats();
    assert_eq!((stats.frames_submitted, stats.frames_completed), (1, 1));
    assert_eq!((stats.draws_submitted, stats.draws_skipped), (1, 0));
    assert_eq!(stats.meshes, 1);
}

#[test]
fn a_resource_destroyed_after_recording_is_dropped_at_submit() {
    let ctx = Ctx::null(8, 8);
    let mesh = make_mesh(&ctx);
    let frame = ctx.acquire();
    // SAFETY: a live context and an open frame.
    unsafe {
        assert_eq!(
            kaman_frame_draw_mesh(frame, mesh, &identity(), null()),
            KAMAN_OK
        );
        assert_eq!(kaman_mesh_destroy(ctx.0, mesh), KAMAN_OK);
        assert_eq!(kaman_frame_submit(frame, null(), null_mut()), KAMAN_OK);
    }
    let stats = ctx.stats();
    assert_eq!((stats.draws_submitted, stats.draws_skipped), (0, 1));
}

#[test]
fn models_load_off_thread_and_report_ready_with_bounds() {
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let releases = AtomicUsize::new(0);
    let mut model = KamanModel::default();
    // SAFETY: CUBE is static.
    let r =
        unsafe { kaman_model_load_memory(ctx.0, borrowed(CUBE, &releases), null(), &mut model) };
    assert_eq!(r, KAMAN_OK);
    assert_eq!(ctx.state(model.id), KAMAN_RESOURCE_STATE_PENDING);
    assert_eq!(
        releases.load(Ordering::SeqCst),
        0,
        "released only by housekeeping"
    );

    let (mut lo, mut hi) = (KamanVec3::default(), KamanVec3::default());
    // SAFETY: a live context.
    assert_eq!(
        unsafe { kaman_model_get_bounds(ctx.0, model, &mut lo, &mut hi) },
        KAMAN_NOT_READY
    );
    let frame = ctx.acquire();
    // SAFETY: an open frame.
    unsafe {
        assert_eq!(
            kaman_frame_draw_model(frame, model, &identity(), null()),
            KAMAN_NOT_READY
        );
        kaman_frame_discard(frame);
    }

    ctx.flush();
    assert_eq!(ctx.state(model.id), KAMAN_RESOURCE_STATE_READY);
    assert_eq!(
        releases.load(Ordering::SeqCst),
        1,
        "released once the parse finished"
    );
    let events = ctx.poll();
    assert!(
        events
            .iter()
            .any(|e| e.kind == KAMAN_EVENT_RESOURCE_READY && e.object == model.id),
        "{events:?}"
    );
    // SAFETY: a live context.
    assert_eq!(
        unsafe { kaman_model_get_bounds(ctx.0, model, &mut lo, &mut hi) },
        KAMAN_OK
    );
    assert_eq!((lo.x, lo.y, lo.z), (-0.5, -0.5, -0.5));
    assert_eq!((hi.x, hi.y, hi.z), (0.5, 0.5, 0.5));

    let frame = ctx.acquire();
    // SAFETY: an open frame.
    unsafe {
        assert_eq!(
            kaman_frame_draw_model(frame, model, &identity(), null()),
            KAMAN_OK
        );
        assert_eq!(kaman_frame_submit(frame, null(), null_mut()), KAMAN_OK);
    }
    assert!(ctx.stats().draws_submitted >= 1);
}

#[test]
fn a_textured_model_loads_its_texture_from_a_file() {
    let ctx = Ctx::null(8, 8);
    let path = CString::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kaman-assets/tests/fixtures/textured_cube.gltf"
    ))
    .unwrap();
    let mut model = KamanModel::default();
    // SAFETY: valid pointers.
    assert_eq!(
        unsafe { kaman_model_load_file(ctx.0, path.as_ptr(), null(), &mut model) },
        KAMAN_OK
    );
    ctx.flush();
    assert_eq!(
        ctx.state(model.id),
        KAMAN_RESOURCE_STATE_READY,
        "{:?}",
        ctx.poll()
    );
}

#[test]
fn a_textured_model_loads_its_data_uri_texture_from_memory() {
    // A self-contained .gltf embeds its image as a `data:` URI; loading it from
    // memory needs no file system, so it works like the by-path load.
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mut model = KamanModel::default();
    // SAFETY: copied data.
    assert_eq!(
        unsafe { kaman_model_load_memory(ctx.0, copied(TEXTURED_CUBE), null(), &mut model) },
        KAMAN_OK
    );
    ctx.flush();
    assert_eq!(
        ctx.state(model.id),
        KAMAN_RESOURCE_STATE_READY,
        "{:?}",
        ctx.poll()
    );
}

#[test]
fn a_model_without_normals_loads_flat_shaded() {
    // glTF requires flat normals when a primitive has none; the importer
    // de-indexes such a primitive, and the C path must upload the result.
    let doc = br#"{
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": 36,
            "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA"}],
        "bufferViews": [{"buffer": 0, "byteLength": 36}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": [0, 0, 0], "max": [1, 1, 0]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "nodes": [{"mesh": 0}],
        "scenes": [{"nodes": [0]}],
        "scene": 0
    }"#;
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mut model = KamanModel::default();
    // SAFETY: copied data, valid out-pointers.
    unsafe {
        assert_eq!(
            kaman_model_load_memory(ctx.0, copied(doc), null(), &mut model),
            KAMAN_OK
        );
        ctx.flush();
        assert_eq!(
            ctx.state(model.id),
            KAMAN_RESOURCE_STATE_READY,
            "{:?}",
            ctx.poll()
        );
        let (mut min, mut max) = (KamanVec3::default(), KamanVec3::default());
        assert_eq!(
            kaman_model_get_bounds(ctx.0, model, &mut min, &mut max),
            KAMAN_OK
        );
        assert_eq!((min.x, min.y, max.x, max.y), (0.0, 0.0, 1.0, 1.0));
    }
}

#[test]
fn memory_loads_still_reject_external_file_references() {
    // There is no directory to resolve a relative URI against, so a document
    // that points at an external file fails, and says why.
    let doc = br#"{
        "asset": {"version": "2.0"},
        "images": [{"uri": "texture.png"}]
    }"#;
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let mut model = KamanModel::default();
    // SAFETY: copied data.
    assert_eq!(
        unsafe { kaman_model_load_memory(ctx.0, copied(doc), null(), &mut model) },
        KAMAN_OK
    );
    ctx.flush();
    let events = ctx.poll();
    let failed = events
        .iter()
        .find(|e| e.object == model.id)
        .expect("an event for the model");
    assert_eq!(
        (failed.kind, failed.result),
        (KAMAN_EVENT_RESOURCE_FAILED, KAMAN_ERROR_PARSE)
    );
    assert!(
        failed
            .message
            .as_deref()
            .unwrap_or("")
            .contains("external reference"),
        "{failed:?}"
    );
}

#[test]
fn bad_models_fail_with_a_reason() {
    let ctx = Ctx::null(8, 8);
    ctx.poll();
    let (mut garbage, mut missing) = (KamanModel::default(), KamanModel::default());
    let path = CString::new("/definitely/not/here.glb").unwrap();
    // SAFETY: valid pointers.
    unsafe {
        assert_eq!(
            kaman_model_load_memory(ctx.0, copied(b"not a gltf"), null(), &mut garbage),
            KAMAN_OK
        );
        assert_eq!(
            kaman_model_load_file(ctx.0, path.as_ptr(), null(), &mut missing),
            KAMAN_OK
        );
    }
    ctx.flush();
    let events = ctx.poll();
    let failure = |id| {
        events
            .iter()
            .find(|e| e.object == id)
            .unwrap_or_else(|| panic!("no event for {id}: {events:?}"))
            .clone()
    };
    let g = failure(garbage.id);
    assert_eq!(
        (g.kind, g.result),
        (KAMAN_EVENT_RESOURCE_FAILED, KAMAN_ERROR_PARSE)
    );
    assert!(g.message.is_some());
    let m = failure(missing.id);
    assert_eq!(
        (m.kind, m.result),
        (KAMAN_EVENT_RESOURCE_FAILED, KAMAN_ERROR_IO),
        "{m:?}"
    );
    assert_eq!(ctx.state(garbage.id), KAMAN_RESOURCE_STATE_FAILED);
    // A failed model is still a live handle: destroy it.
    // SAFETY: a live context.
    assert_eq!(unsafe { kaman_model_destroy(ctx.0, garbage) }, KAMAN_OK);
}

#[test]
fn destroying_a_context_releases_borrowed_model_bytes() {
    let releases = AtomicUsize::new(0);
    {
        let ctx = Ctx::null(8, 8);
        let mut model = KamanModel::default();
        // SAFETY: CUBE is static.
        unsafe { kaman_model_load_memory(ctx.0, borrowed(CUBE, &releases), null(), &mut model) };
    }
    assert_eq!(releases.load(Ordering::SeqCst), 1);
}

#[test]
fn readbacks_complete_or_fail_with_their_frame() {
    let ctx = Ctx::null(4, 2);
    ctx.poll();
    let mut pixels = vec![0xFFu8; 4 * 2 * 4];
    let frame = ctx.acquire();
    // SAFETY: `pixels` outlives the read-back event.
    unsafe {
        assert_eq!(
            kaman_frame_read_pixels_async(frame, pixels.as_mut_ptr().cast(), 3, null()),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            kaman_frame_read_pixels_async(frame, pixels.as_mut_ptr().cast(), pixels.len(), null()),
            KAMAN_OK
        );
        assert_eq!(
            kaman_frame_read_pixels_async(frame, pixels.as_mut_ptr().cast(), pixels.len(), null()),
            KAMAN_ERROR_WRONG_STATE
        );
        assert_eq!(kaman_frame_submit(frame, null(), null_mut()), KAMAN_OK);
    }
    let events = ctx.poll();
    assert!(
        events
            .iter()
            .any(|e| e.kind == KAMAN_EVENT_READBACK_COMPLETE && e.frame == 1),
        "{events:?}"
    );
    assert!(
        pixels.iter().all(|&b| b == 0),
        "the null backend reads back zeros"
    );

    let frame = ctx.acquire();
    // SAFETY: as above.
    unsafe {
        kaman_frame_read_pixels_async(frame, pixels.as_mut_ptr().cast(), pixels.len(), null());
        assert_eq!(kaman_frame_discard(frame), KAMAN_OK);
    }
    assert!(ctx
        .poll()
        .iter()
        .any(|e| e.kind == KAMAN_EVENT_READBACK_FAILED));
}

#[test]
fn fonts_measure_and_draw_until_their_atlas_goes() {
    let ctx = Ctx::null(64, 64);
    let atlas = make_texture(&ctx, 2, 2, KAMAN_COLOR_SPACE_LINEAR);
    let glyph = KamanGlyph {
        u1: 1.0,
        v1: 1.0,
        width: 0.5,
        height: 1.0,
        bearing_y: -1.0,
        advance: 0.6,
        ..KamanGlyph::default()
    };
    let glyphs = [glyph; 26];
    let desc = KamanFontDesc {
        atlas,
        first_codepoint: 'A' as u32,
        glyphs: glyphs.as_ptr(),
        glyph_count: 26,
        line_height: 1.2,
        fallback: KamanGlyph {
            advance: 0.5,
            ..KamanGlyph::default()
        },
    };
    let mut font = KamanFont::default();
    let text = CString::new("AB C").unwrap();
    let white = KamanColor {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    // SAFETY: valid pointers throughout.
    unsafe {
        assert_eq!(kaman_font_create(ctx.0, &desc, null(), &mut font), KAMAN_OK);
        let mut width = 0.0;
        assert_eq!(
            kaman_font_measure(ctx.0, font, text.as_ptr(), 10.0, &mut width),
            KAMAN_OK
        );
        assert!(
            (width - (0.6 + 0.6 + 0.5 + 0.6) * 10.0).abs() < 1e-4,
            "{width}"
        );

        let frame = ctx.acquire();
        assert_eq!(
            kaman_frame_draw_text(frame, font, text.as_ptr(), 4.0, 20.0, 10.0, white),
            KAMAN_OK
        );
        assert_eq!(kaman_texture_destroy(ctx.0, atlas), KAMAN_OK);
        assert_eq!(
            kaman_frame_draw_text(frame, font, text.as_ptr(), 4.0, 20.0, 10.0, white),
            KAMAN_NOT_READY
        );
        let bad_fill = KamanQuad {
            fill: 9,
            ..KamanQuad::default()
        };
        assert_eq!(
            kaman_frame_draw_quad(frame, &bad_fill),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(kaman_frame_submit(frame, null(), null_mut()), KAMAN_OK);
    }
    // Three glyph quads recorded before the atlas went, all dropped at submit,
    // plus the text draw refused afterwards.
    assert_eq!(ctx.stats().draws_skipped, 4);
}

#[test]
fn settings_round_trip_and_are_validated() {
    let ctx = Ctx::null(8, 8);
    let mut settings = KamanRenderSettings {
        shadows: 0,
        shadow_distance: 0.0,
        draw_distance_scale: 0.0,
    };
    // SAFETY: valid pointers.
    unsafe {
        kaman_render_settings_default(&mut settings);
        assert_eq!(settings.shadows, KAMAN_SHADOW_QUALITY_HIGH);
        settings.shadows = KAMAN_SHADOW_QUALITY_LOW;
        settings.shadow_distance = 0.5;
        assert_eq!(kaman_context_set_settings(ctx.0, &settings), KAMAN_OK);
        let mut read = settings;
        read.shadows = 0;
        assert_eq!(kaman_context_get_settings(ctx.0, &mut read), KAMAN_OK);
        assert_eq!(read, settings);
        settings.draw_distance_scale = f32::INFINITY;
        assert_eq!(
            kaman_context_set_settings(ctx.0, &settings),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(
            kaman_context_resize(ctx.0, 16, 16, 1.0),
            KAMAN_ERROR_UNSUPPORTED
        );
    }
}

#[test]
fn events_that_do_not_fit_stay_queued() {
    let ctx = Ctx::null(8, 8);
    for _ in 0..3 {
        make_mesh(&ctx);
    }
    let mut one = [KamanEvent {
        kind: 0,
        result: 0,
        object: 0,
        frame: 0,
        message: null(),
    }];
    let mut n = 0;
    // SAFETY: a one-event buffer.
    unsafe {
        assert_eq!(
            kaman_poll_events(ctx.0, one.as_mut_ptr(), 1, &mut n),
            KAMAN_OK
        );
        assert_eq!(n, 1);
        assert_eq!(kaman_poll_events(ctx.0, null_mut(), 0, &mut n), KAMAN_OK);
        assert_eq!(n, 0);
        assert_eq!(
            kaman_poll_events(ctx.0, null_mut(), 1, &mut n),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
    }
    assert_eq!(
        ctx.poll().len(),
        3,
        "context ready + the two remaining mesh events"
    );
}

/// The C API and the Rust API render the same pixels for the same scene.
#[cfg(target_os = "macos")]
#[test]
fn the_c_api_renders_what_the_rust_api_renders() {
    use std::io::Write;

    use kaman_math::glam::{Mat4, Vec3};
    use kaman_render::MetalRenderer;
    use kaman_render_api::{
        FrameRecorder, MaterialParams, MeshData, PipelineDescriptor, PipelineKind, RenderDevice,
        SunSky,
    };

    const W: u32 = 64;
    const H: u32 = 48;
    let Some(mut rust) = MetalRenderer::new_offscreen(W, H) else {
        // Visible past the harness's capture; a failure under KAMAN_REQUIRE_GPU=1
        // (CI), so a runner without a GPU cannot pass this test unrun.
        let required = std::env::var("KAMAN_REQUIRE_GPU")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        assert!(!required, "no Metal device, and KAMAN_REQUIRE_GPU is set");
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED the_c_api_renders_what_the_rust_api_renders: no Metal device"
        );
        return;
    };
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, Vec3::Y);
    let vp = Mat4::perspective_rh(0.9, W as f32 / H as f32, 0.1, 50.0) * view;
    let (v, i) = triangle([0.9, 0.2, 0.1]);
    let indices: Vec<u32> = i
        .chunks_exact(4)
        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
        .collect();

    let pipeline = rust.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor));
    let mesh = rust.create_mesh(&MeshData {
        vertices: &v,
        indices: &indices,
        layout: PipelineKind::VertexColor.vertex_layout(),
    });
    rust.set_view_projection(vp);
    rust.set_camera_position(Vec3::new(0.0, 0.0, 2.0));
    rust.set_sun_sky(&SunSky::default());
    rust.begin_frame();
    rust.set_pipeline(pipeline);
    rust.draw_mesh_matrix(mesh, Mat4::IDENTITY, &MaterialParams::default());
    rust.submit();
    let expected = rust.read_pixels().expect("offscreen read-back");

    let mut ctx = null_mut();
    // SAFETY: valid pointers.
    assert_eq!(
        unsafe { kaman_context_create(&desc(KAMAN_BACKEND_METAL, W, H), &mut ctx) },
        KAMAN_OK,
        "{}",
        last_error()
    );
    let ctx = Ctx(ctx);
    let mut c_mesh = KamanMesh::default();
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    // SAFETY: valid pointers; `pixels` outlives the flush.
    unsafe {
        assert_eq!(
            kaman_mesh_create(
                ctx.0,
                &mesh_desc(copied(&v), copied(&i)),
                null(),
                &mut c_mesh
            ),
            KAMAN_OK
        );
        let frame = ctx.acquire();
        let camera = KamanCamera {
            view_projection: vp.into_c(),
            position: KamanVec3 {
                x: 0.0,
                y: 0.0,
                z: 2.0,
            },
        };
        kaman_frame_set_camera(frame, &camera);
        assert_eq!(
            kaman_frame_draw_mesh(frame, c_mesh, &identity(), null()),
            KAMAN_OK
        );
        assert_eq!(
            kaman_frame_read_pixels_async(frame, pixels.as_mut_ptr().cast(), pixels.len(), null()),
            KAMAN_OK
        );
        assert_eq!(kaman_frame_submit(frame, null(), null_mut()), KAMAN_OK);
    }
    ctx.flush();
    assert!(ctx
        .poll()
        .iter()
        .any(|e| e.kind == KAMAN_EVENT_READBACK_COMPLETE));
    assert_eq!(pixels, expected, "the C API drew different pixels");
    assert_ne!(pixels, vec![0u8; pixels.len()], "the frame was drawn");

    trait IntoC {
        fn into_c(self) -> KamanMat4;
    }
    impl IntoC for Mat4 {
        fn into_c(self) -> KamanMat4 {
            KamanMat4 {
                m: self.to_cols_array(),
            }
        }
    }
}

#[test]
fn distance_fields_must_be_linear_textures() {
    // An SDF atlas stores distances; decoding them as sRGB colour would move the
    // glyph edges. Fonts and SDF quads refuse an sRGB texture outright.
    let ctx = Ctx::null(64, 64);
    let srgb = make_texture(&ctx, 2, 2, KAMAN_COLOR_SPACE_SRGB);
    let linear = make_texture(&ctx, 2, 2, KAMAN_COLOR_SPACE_LINEAR);
    let font_desc = |atlas| KamanFontDesc {
        atlas,
        first_codepoint: 'A' as u32,
        glyphs: null(),
        glyph_count: 0,
        line_height: 1.0,
        fallback: KamanGlyph::default(),
    };
    let mut font = KamanFont::default();
    // SAFETY: valid pointers throughout.
    unsafe {
        assert_eq!(
            kaman_font_create(ctx.0, &font_desc(srgb), null(), &mut font),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        assert!(
            last_error().contains("KAMAN_COLOR_SPACE_LINEAR"),
            "{}",
            last_error()
        );
        assert_eq!(
            kaman_font_create(ctx.0, &font_desc(linear), null(), &mut font),
            KAMAN_OK
        );

        let frame = ctx.acquire();
        let quad = |texture| KamanQuad {
            width: 8.0,
            height: 8.0,
            u1: 1.0,
            v1: 1.0,
            fill: KAMAN_FILL_SDF,
            texture,
            ..KamanQuad::default()
        };
        assert_eq!(
            kaman_frame_draw_quad(frame, &quad(srgb)),
            KAMAN_ERROR_INVALID_ARGUMENT
        );
        assert_eq!(kaman_frame_draw_quad(frame, &quad(linear)), KAMAN_OK);
        // Plain textured quads take either colour space.
        let textured = KamanQuad {
            fill: KAMAN_FILL_TEXTURE,
            ..quad(srgb)
        };
        assert_eq!(kaman_frame_draw_quad(frame, &textured), KAMAN_OK);
        assert_eq!(kaman_frame_discard(frame), KAMAN_OK);
    }
}
