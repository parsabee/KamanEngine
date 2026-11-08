// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! glTF models: parsed on a small worker pool, uploaded on the host thread
//! during housekeeping, then drawn whole.
//!
//! The workers only read and decode (file IO, glTF parse, image decode). The
//! GPU upload and every handle-table change happen on the host thread inside
//! `kaman_poll_events` / `kaman_context_flush`, so the context itself is never
//! touched off the host thread.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use kaman_assets::{ImportError, SceneAsset};
use kaman_math::glam::{Mat4, Vec3};
use kaman_render_api::{MeshHandle, PipelineKind, TextureHandle};

use crate::backend::Backend;
use crate::context::KamanContext;
use crate::context::{with_context, Inner, OwnedEvent};
use crate::core::{call, output, utf8, Error};
use crate::resources::{bytes, validate_mesh, validate_texture};
use crate::types::*;

/// How many worker threads parse models.
const WORKERS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelState {
    Pending,
    Ready,
    Failed,
}

/// One uploaded primitive of a model.
pub(crate) struct Part {
    pub(crate) kind: PipelineKind,
    pub(crate) mesh: MeshHandle,
    pub(crate) texture: Option<TextureHandle>,
    pub(crate) base_color: [f32; 4],
}

pub(crate) struct ModelEntry {
    pub(crate) state: ModelState,
    completion: Option<KamanCompletion>,
    pub(crate) parts: Vec<Part>,
    /// `(part, node transform)` for every node that draws a part.
    pub(crate) instances: Vec<(usize, Mat4)>,
    bounds: (Vec3, Vec3),
}

/// Where a model's bytes come from.
enum Source {
    File(PathBuf),
    Owned(Vec<u8>),
    /// Host memory lent with a release function, valid until it is released.
    Borrowed {
        ptr: *const u8,
        len: usize,
    },
}

// SAFETY: `Borrowed` points at host memory the host promised to keep valid and
// unchanged until the engine releases it, which happens only after the worker
// reading it has sent its result; the worker only reads it.
unsafe impl Send for Source {}

struct Job {
    model: u64,
    source: Source,
}

/// A parsed model, ready to upload.
struct Parsed {
    scene: SceneAsset,
    instances: Vec<(usize, Mat4)>,
    bounds: (Vec3, Vec3),
}

struct JobResult {
    model: u64,
    outcome: Result<Parsed, Error>,
}

/// The worker pool, started on a context's first model load.
pub(crate) struct Loader {
    jobs: Option<mpsc::Sender<Job>>,
    results: mpsc::Receiver<JobResult>,
    cancel: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    in_flight: u32,
    /// Borrowed model bytes, by model id, released when their parse is done.
    borrowed: HashMap<u64, KamanData>,
}

impl Loader {
    fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (result_tx, results) = mpsc::channel();
        let jobs = Arc::new(Mutex::new(job_rx));
        let cancel = Arc::new(AtomicBool::new(false));
        let threads = (0..WORKERS)
            .filter_map(|i| {
                let (jobs, results, cancel) =
                    (Arc::clone(&jobs), result_tx.clone(), Arc::clone(&cancel));
                std::thread::Builder::new()
                    .name(format!("kaman-loader-{i}"))
                    .spawn(move || worker(&jobs, &results, &cancel))
                    .ok()
            })
            .collect();
        Self {
            jobs: Some(job_tx),
            results,
            cancel,
            threads,
            in_flight: 0,
            borrowed: HashMap::new(),
        }
    }

    fn submit(&mut self, job: Job, borrowed: Option<KamanData>) -> Result<(), Error> {
        let model = job.model;
        let sent = self
            .jobs
            .as_ref()
            .is_some_and(|jobs| !self.threads.is_empty() && jobs.send(job).is_ok());
        if !sent {
            return Err(Error::new(
                KAMAN_ERROR_INTERNAL,
                "the model loader threads are not running",
            ));
        }
        self.in_flight += 1;
        if let Some(data) = borrowed {
            self.borrowed.insert(model, data);
        }
        Ok(())
    }

    /// Loads that have not finished parsing, or finished but not been
    /// processed by housekeeping yet.
    pub(crate) fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// Stop the workers (a parse in progress finishes, queued ones are
    /// skipped) and return every borrowed buffer for release.
    pub(crate) fn shutdown(mut self) -> Vec<KamanData> {
        self.cancel.store(true, Ordering::SeqCst);
        drop(self.jobs.take());
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        self.borrowed.into_values().collect()
    }
}

fn worker(
    jobs: &Mutex<mpsc::Receiver<Job>>,
    results: &mpsc::Sender<JobResult>,
    cancel: &AtomicBool,
) {
    loop {
        let job = jobs.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(job) = job else { return };
        let outcome = if cancel.load(Ordering::SeqCst) {
            Err(Error::wrong_state("the context was destroyed"))
        } else {
            parse(&job.source)
        };
        if results
            .send(JobResult {
                model: job.model,
                outcome,
            })
            .is_err()
        {
            return;
        }
    }
}

fn import_error(e: &ImportError) -> Error {
    let code = if e.is_io() {
        KAMAN_ERROR_IO
    } else {
        KAMAN_ERROR_PARSE
    };
    Error::new(code, e.to_string())
}

/// Read and decode a model, off the host thread.
fn parse(source: &Source) -> Result<Parsed, Error> {
    let imported = catch_unwind(AssertUnwindSafe(|| match source {
        Source::File(path) => kaman_assets::import_gltf(path),
        Source::Owned(bytes) => kaman_assets::import_slice(bytes),
        // SAFETY: see `Source`'s `Send` impl: the bytes stay valid until this
        // job's result has been processed.
        Source::Borrowed { ptr, len } => {
            kaman_assets::import_slice(unsafe { std::slice::from_raw_parts(*ptr, *len) })
        }
    }));
    let scene = match imported {
        Ok(Ok(scene)) => scene,
        Ok(Err(e)) => return Err(import_error(&e)),
        Err(_) => {
            return Err(Error::new(
                KAMAN_ERROR_PARSE,
                "malformed glTF: the importer could not process this document",
            ))
        }
    };
    let instances: Vec<(usize, Mat4)> = scene
        .mesh_nodes()
        .filter_map(|(_, node)| Some((node.mesh?, node.transform.to_matrix())))
        .filter(|&(mesh, _)| mesh < scene.meshes.len())
        .collect();
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for &(mesh, transform) in &instances {
        for p in &scene.meshes[mesh].positions {
            let p = transform.transform_point3(Vec3::from(*p));
            bounds = Some(match bounds {
                None => (p, p),
                Some((lo, hi)) => (lo.min(p), hi.max(p)),
            });
        }
    }
    Ok(Parsed {
        bounds: bounds.unwrap_or((Vec3::ZERO, Vec3::ZERO)),
        scene,
        instances,
    })
}

fn destroy_parts(backend: &mut dyn Backend, parts: &[Part]) {
    for part in parts {
        backend.destroy_mesh(part.mesh);
        if let Some(texture) = part.texture {
            backend.destroy_texture(texture);
        }
    }
}

/// Upload a parsed model's meshes and textures; all or nothing.
fn upload(backend: &mut dyn Backend, scene: &SceneAsset) -> Result<Vec<Part>, Error> {
    let mut parts = Vec::with_capacity(scene.meshes.len());
    for mesh in &scene.meshes {
        let part = (|| {
            let kind = PipelineKind::for_layout(&mesh.layout).ok_or_else(|| {
                Error::new(
                    KAMAN_ERROR_PARSE,
                    "a mesh uses an unsupported vertex layout",
                )
            })?;
            validate_mesh(kind, &mesh.vertices, &mesh.indices)?;
            let handle = backend.create_mesh(kind, &mesh.vertices, &mesh.indices)?;
            let texture = match &mesh.base_color {
                None => None,
                Some(t) => {
                    let created = validate_texture(t.width, t.height, &t.rgba8)
                        .and_then(|()| backend.create_texture(t.width, t.height, &t.rgba8));
                    match created {
                        Ok(texture) => Some(texture),
                        Err(e) => {
                            backend.destroy_mesh(handle);
                            return Err(e);
                        }
                    }
                }
            };
            Ok(Part {
                kind,
                mesh: handle,
                texture,
                base_color: mesh.base_color_factor,
            })
        })();
        match part {
            Ok(part) => parts.push(part),
            Err(e) => {
                destroy_parts(backend, &parts);
                return Err(e);
            }
        }
    }
    Ok(parts)
}

/// Housekeeping: take finished parses, release their borrowed bytes, upload,
/// and report each model ready or failed.
pub(crate) fn pump(inner: &mut Inner) {
    let Some(loader) = inner.loader.as_mut() else {
        return;
    };
    let finished: Vec<JobResult> = loader.results.try_iter().collect();
    for result in finished {
        let loader = inner.loader.as_mut().expect("checked above");
        loader.in_flight -= 1;
        if let Some(data) = loader.borrowed.remove(&result.model) {
            inner.release(data);
        }
        let Some(entry) = inner.models.get_mut(result.model) else {
            continue; // destroyed while it was loading
        };
        let completion = entry.completion.take();
        let uploaded = result.outcome.and_then(|parsed| {
            upload(inner.backend.as_mut(), &parsed.scene).map(|parts| (parts, parsed))
        });
        let entry = inner.models.get_mut(result.model).expect("still live");
        let event = match uploaded {
            Ok((parts, parsed)) => {
                entry.state = ModelState::Ready;
                entry.parts = parts;
                entry.instances = parsed.instances;
                entry.bounds = parsed.bounds;
                OwnedEvent::new(KAMAN_EVENT_RESOURCE_READY, result.model, 0)
            }
            Err(e) => {
                entry.state = ModelState::Failed;
                OwnedEvent::failed(KAMAN_EVENT_RESOURCE_FAILED, result.model, 0, &e)
            }
        };
        inner.emit(event, completion);
    }
}

/// Reserve a pending model and queue its parse.
fn start_load(
    inner: &mut Inner,
    source: Source,
    borrowed: Option<KamanData>,
    completion: Option<KamanCompletion>,
) -> Result<u64, Error> {
    let id = inner
        .models
        .insert(ModelEntry {
            state: ModelState::Pending,
            completion,
            parts: Vec::new(),
            instances: Vec::new(),
            bounds: (Vec3::ZERO, Vec3::ZERO),
        })
        .map_err(|_| Error::new(KAMAN_ERROR_LIMIT_EXCEEDED, "too many live models"))?;
    let loader = inner.loader.get_or_insert_with(Loader::new);
    if let Err(e) = loader.submit(Job { model: id, source }, borrowed) {
        inner.models.remove(id);
        return Err(e);
    }
    Ok(id)
}

/// Destroy a model's GPU resources (if uploaded) and its handle.
pub(crate) fn destroy(inner: &mut Inner, id: u64) -> bool {
    match inner.models.remove(id) {
        Some(model) => {
            destroy_parts(inner.backend.as_mut(), &model.parts);
            true
        }
        None => false,
    }
}

/// Start loading a `.gltf` or `.glb` file. Returns at once with a pending
/// model; its `RESOURCE_READY` or `RESOURCE_FAILED` event follows.
///
/// # Safety
/// `ctx` must be NULL or a live context; `path` NULL or a NUL-terminated
/// string; `completion` and `out` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn kaman_model_load_file(
    ctx: *mut KamanContext,
    path: *const std::ffi::c_char,
    completion: *const KamanCompletion,
    out: *mut KamanModel,
) -> KamanResult {
    call(|| {
        let path = PathBuf::from(utf8(path, "path")?);
        let out = output(out, "out")?;
        let completion = completion.as_ref().copied();
        let id = with_context(ctx, |inner| {
            start_load(inner, Source::File(path), None, completion)
        })?;
        *out = KamanModel { id };
        Ok(KAMAN_OK)
    })
}

/// Start loading a self-contained `.glb` or `.gltf` (embedded buffers and
/// images) from memory. Returns at once with a pending model.
///
/// # Safety
/// `ctx` must be NULL or a live context; `bytes` valid as documented for
/// `KamanData`; `completion` and `out` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn kaman_model_load_memory(
    ctx: *mut KamanContext,
    bytes_in: KamanData,
    completion: *const KamanCompletion,
    out: *mut KamanModel,
) -> KamanResult {
    call(|| {
        let out = output(out, "out")?;
        let slice = bytes(&bytes_in, "bytes")?;
        let completion = completion.as_ref().copied();
        let (source, borrowed) = if bytes_in.release.is_some() {
            (
                Source::Borrowed {
                    ptr: slice.as_ptr(),
                    len: slice.len(),
                },
                Some(bytes_in),
            )
        } else {
            (Source::Owned(slice.to_vec()), None)
        };
        let id = with_context(ctx, |inner| start_load(inner, source, borrowed, completion))?;
        *out = KamanModel { id };
        Ok(KAMAN_OK)
    })
}

/// Destroy a model. A pending load is cancelled (its borrowed bytes are still
/// released, once the parse in progress lets go of them).
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn kaman_model_destroy(
    ctx: *mut KamanContext,
    model: KamanModel,
) -> KamanResult {
    call(|| {
        with_context(ctx, |inner| {
            if destroy(inner, model.id) {
                Ok(KAMAN_OK)
            } else {
                Err(Error::invalid_handle("model"))
            }
        })
    })
}

/// The model's bounding box in model space. `KAMAN_NOT_READY` until it loads.
///
/// # Safety
/// `ctx` must be NULL or a live context; `min` and `max` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn kaman_model_get_bounds(
    ctx: *mut KamanContext,
    model: KamanModel,
    min: *mut KamanVec3,
    max: *mut KamanVec3,
) -> KamanResult {
    call(|| {
        let min = output(min, "min")?;
        let max = output(max, "max")?;
        with_context(ctx, |inner| {
            let entry = inner
                .models
                .get(model.id)
                .ok_or_else(|| Error::invalid_handle("model"))?;
            if entry.state != ModelState::Ready {
                return Ok(KAMAN_NOT_READY);
            }
            *min = entry.bounds.0.into();
            *max = entry.bounds.1.into();
            Ok(KAMAN_OK)
        })
    })
}
