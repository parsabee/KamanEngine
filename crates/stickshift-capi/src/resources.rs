// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Meshes and textures, the input validation every upload goes through, and
//! resource states.
//!
//! Uploads are synchronous in the beta: the resource is `READY` when the create
//! call returns, and its `RESOURCE_READY` event is queued (or its callback runs
//! before the call returns).

use stickshift_render_api::{ColorSpace, MeshHandle, PipelineKind, TextureHandle};

use crate::context::StickShiftContext;
use crate::context::{with_context, Inner, OwnedEvent, MAX_DIMENSION};
use crate::core::{call, input, output, Error};
use crate::handles::{kind_of, Kind};
use crate::models::ModelState;
use crate::types::*;

pub(crate) struct MeshEntry {
    pub(crate) kind: PipelineKind,
    pub(crate) handle: MeshHandle,
}

pub(crate) struct TextureEntry {
    pub(crate) handle: TextureHandle,
    /// How the texture was uploaded; font atlases and SDF quads need `Linear`.
    pub(crate) color_space: ColorSpace,
}

/// The engine colour space for a `STICKSHIFT_COLOR_SPACE_*` value.
pub(crate) fn color_space(value: StickShiftColorSpace) -> Result<ColorSpace, Error> {
    match value {
        STICKSHIFT_COLOR_SPACE_SRGB => Ok(ColorSpace::Srgb),
        STICKSHIFT_COLOR_SPACE_LINEAR => Ok(ColorSpace::Linear),
        other => Err(Error::invalid_argument(format!(
            "{other} is not a STICKSHIFT_COLOR_SPACE_* value"
        ))),
    }
}

/// Borrow the bytes of `data` for the duration of the call.
///
/// # Safety
/// `data.ptr` must be valid for `data.len` bytes (or `len` 0).
pub(crate) unsafe fn bytes<'a>(data: &StickShiftData, what: &str) -> Result<&'a [u8], Error> {
    if data.len == 0 {
        return Ok(&[]);
    }
    if data.ptr.is_null() {
        return Err(Error::invalid_argument(format!(
            "{what}.ptr is NULL but {what}.len is {}",
            data.len
        )));
    }
    Ok(std::slice::from_raw_parts(data.ptr.cast(), data.len))
}

/// The pipeline a `STICKSHIFT_VERTEX_LAYOUT_*` value selects.
pub(crate) fn pipeline_kind(layout: StickShiftVertexLayout) -> Result<PipelineKind, Error> {
    match layout {
        STICKSHIFT_VERTEX_LAYOUT_POS_NORMAL_COLOR => Ok(PipelineKind::VertexColor),
        STICKSHIFT_VERTEX_LAYOUT_POS_NORMAL_UV => Ok(PipelineKind::Textured),
        other => Err(Error::invalid_argument(format!(
            "{other} is not a STICKSHIFT_VERTEX_LAYOUT_* value"
        ))),
    }
}

/// Check mesh data against its layout: whole vertices and triangles, indices in
/// range.
pub(crate) fn validate_mesh(
    kind: PipelineKind,
    vertices: &[u8],
    indices: &[u32],
) -> Result<(), Error> {
    let stride = kind.vertex_layout().stride as usize;
    if !vertices.len().is_multiple_of(stride) {
        return Err(Error::invalid_argument(format!(
            "{} vertex bytes is not a whole number of {stride}-byte vertices",
            vertices.len()
        )));
    }
    if !indices.len().is_multiple_of(3) {
        return Err(Error::invalid_argument(format!(
            "{} indices is not a whole number of triangles",
            indices.len()
        )));
    }
    let vertex_count = vertices.len() / stride;
    if let Some(bad) = indices.iter().find(|&&i| i as usize >= vertex_count) {
        return Err(Error::invalid_argument(format!(
            "index {bad} is out of range for {vertex_count} vertices"
        )));
    }
    Ok(())
}

/// Check a texture's size and pixel data.
pub(crate) fn validate_texture(width: u32, height: u32, rgba8: &[u8]) -> Result<(), Error> {
    if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
        return Err(Error::invalid_argument(format!(
            "{width}x{height}: each side must be 1..={MAX_DIMENSION}"
        )));
    }
    let need = width as usize * height as usize * 4;
    if rgba8.len() < need {
        return Err(Error::invalid_argument(format!(
            "{width}x{height} RGBA8 needs {need} bytes, got {}",
            rgba8.len()
        )));
    }
    Ok(())
}

/// Index bytes as `u32`s, whatever their alignment.
fn indices(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    if !bytes.len().is_multiple_of(4) {
        return Err(Error::invalid_argument(format!(
            "{} index bytes is not a whole number of uint32_t",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn limit(what: &str) -> Error {
    Error::new(
        STICKSHIFT_ERROR_LIMIT_EXCEEDED,
        format!("too many live {what}"),
    )
}

/// The state of any handle id.
pub(crate) fn state(inner: &Inner, id: u64) -> StickShiftResourceState {
    let live = match kind_of(id) {
        Some(Kind::Mesh) => inner.meshes.get(id).is_some(),
        Some(Kind::Texture) => inner.textures.get(id).is_some(),
        Some(Kind::Font) => inner.fonts.get(id).is_some(),
        Some(Kind::Model) => {
            return match inner.models.get(id).map(|m| m.state) {
                None => STICKSHIFT_RESOURCE_STATE_INVALID,
                Some(ModelState::Pending) => STICKSHIFT_RESOURCE_STATE_PENDING,
                Some(ModelState::Ready) => STICKSHIFT_RESOURCE_STATE_READY,
                Some(ModelState::Failed) => STICKSHIFT_RESOURCE_STATE_FAILED,
            }
        }
        None => false,
    };
    if live {
        STICKSHIFT_RESOURCE_STATE_READY
    } else {
        STICKSHIFT_RESOURCE_STATE_INVALID
    }
}

/// Upload a mesh. `READY` on return in the beta.
///
/// # Safety
/// `ctx` must be NULL or a live context; `desc`, `completion` and `out` NULL or
/// valid; the `StickShiftData` in `desc` valid as documented.
#[no_mangle]
pub unsafe extern "C" fn stickshift_mesh_create(
    ctx: *mut StickShiftContext,
    desc: *const StickShiftMeshDesc,
    completion: *const StickShiftCompletion,
    out: *mut StickShiftMesh,
) -> StickShiftResult {
    call(|| {
        let desc = input(desc, "desc")?;
        let out = output(out, "out")?;
        let completion = completion.as_ref().copied();
        let kind = pipeline_kind(desc.layout)?;
        let vertices = bytes(&desc.vertices, "vertices")?;
        let index_list = indices(bytes(&desc.indices, "indices")?)?;
        validate_mesh(kind, vertices, &index_list)?;
        let id = with_context(ctx, |inner| {
            let handle = inner.backend.create_mesh(kind, vertices, &index_list)?;
            let id = match inner.meshes.insert(MeshEntry { kind, handle }) {
                Ok(id) => id,
                Err(_) => {
                    inner.backend.destroy_mesh(handle);
                    return Err(limit("meshes"));
                }
            };
            inner.release(desc.vertices);
            inner.release(desc.indices);
            inner.emit(
                OwnedEvent::new(STICKSHIFT_EVENT_RESOURCE_READY, id, 0),
                completion,
            );
            Ok(id)
        })?;
        *out = StickShiftMesh { id };
        Ok(STICKSHIFT_OK)
    })
}

/// Destroy a mesh. Frames already submitted keep drawing it until they finish.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_mesh_destroy(
    ctx: *mut StickShiftContext,
    mesh: StickShiftMesh,
) -> StickShiftResult {
    call(|| {
        with_context(ctx, |inner| {
            let entry = inner
                .meshes
                .remove(mesh.id)
                .ok_or_else(|| Error::invalid_handle("mesh"))?;
            inner.backend.destroy_mesh(entry.handle);
            Ok(STICKSHIFT_OK)
        })
    })
}

/// Upload an RGBA8 texture with a full mip chain. `READY` on return in the beta.
///
/// # Safety
/// `ctx` must be NULL or a live context; `desc`, `completion` and `out` NULL or
/// valid; `desc->rgba8` valid as documented.
#[no_mangle]
pub unsafe extern "C" fn stickshift_texture_create(
    ctx: *mut StickShiftContext,
    desc: *const StickShiftTextureDesc,
    completion: *const StickShiftCompletion,
    out: *mut StickShiftTexture,
) -> StickShiftResult {
    call(|| {
        let desc = input(desc, "desc")?;
        let out = output(out, "out")?;
        let completion = completion.as_ref().copied();
        if desc.flags != 0 {
            return Err(Error::invalid_argument("flags is reserved and must be 0"));
        }
        let space = color_space(desc.color_space)?;
        let pixels = bytes(&desc.rgba8, "rgba8")?;
        validate_texture(desc.width, desc.height, pixels)?;
        let id = with_context(ctx, |inner| {
            let handle = inner
                .backend
                .create_texture(desc.width, desc.height, pixels, space)?;
            let id = match inner.textures.insert(TextureEntry {
                handle,
                color_space: space,
            }) {
                Ok(id) => id,
                Err(_) => {
                    inner.backend.destroy_texture(handle);
                    return Err(limit("textures"));
                }
            };
            inner.release(desc.rgba8);
            inner.emit(
                OwnedEvent::new(STICKSHIFT_EVENT_RESOURCE_READY, id, 0),
                completion,
            );
            Ok(id)
        })?;
        *out = StickShiftTexture { id };
        Ok(STICKSHIFT_OK)
    })
}

/// Destroy a texture. Frames already submitted keep sampling it until they
/// finish; fonts using it as their atlas stop drawing.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn stickshift_texture_destroy(
    ctx: *mut StickShiftContext,
    texture: StickShiftTexture,
) -> StickShiftResult {
    call(|| {
        with_context(ctx, |inner| {
            let entry = inner
                .textures
                .remove(texture.id)
                .ok_or_else(|| Error::invalid_handle("texture"))?;
            inner.backend.destroy_texture(entry.handle);
            Ok(STICKSHIFT_OK)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_validation_checks_vertices_triangles_and_indices() {
        let v = vec![0u8; 36 * 3];
        assert!(validate_mesh(PipelineKind::VertexColor, &v, &[0, 1, 2]).is_ok());
        assert!(validate_mesh(PipelineKind::VertexColor, &v[..35], &[]).is_err());
        assert!(validate_mesh(PipelineKind::VertexColor, &v, &[0, 1]).is_err());
        assert!(validate_mesh(PipelineKind::VertexColor, &v, &[0, 1, 3]).is_err());
        // 108 bytes is 3 textured (36-byte) vertices but not whole 32-byte ones.
        assert!(validate_mesh(PipelineKind::Textured, &v, &[0, 1, 2]).is_err());
    }

    #[test]
    fn texture_validation_checks_size_and_length() {
        assert!(validate_texture(2, 2, &[0; 16]).is_ok());
        assert!(validate_texture(2, 2, &[0; 15]).is_err());
        assert!(validate_texture(0, 2, &[]).is_err());
        assert!(validate_texture(MAX_DIMENSION + 1, 1, &[]).is_err());
    }

    #[test]
    fn indices_are_read_at_any_alignment() {
        let mut raw = vec![0xAAu8];
        raw.extend_from_slice(&7u32.to_ne_bytes());
        raw.extend_from_slice(&9u32.to_ne_bytes());
        assert_eq!(indices(&raw[1..]).unwrap(), vec![7, 9]);
        assert!(indices(&raw[1..6]).is_err());
    }
}
