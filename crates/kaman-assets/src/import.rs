// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! glTF (`.gltf` / `.glb`) import into a [`SceneAsset`].
//!
//! The importer walks the glTF default scene's node tree, **bakes** each node's
//! transform into world space (static v1: no runtime hierarchy math, no
//! skinning/animation), reads each mesh's positions/normals/UVs, and packs the
//! geometry onto the canonical `[pos,normal,color]` render layout with a default
//! vertex color. Parsed UVs are retained on [`MeshAsset::uvs`] for KE-0403.

use std::path::Path;

use kaman_math::glam::Mat4;

use crate::error::ImportError;
use crate::scene::{
    bake_transform, pack_render_vertices, pack_textured_vertices, render_vertex_layout,
    textured_vertex_layout, BaseColorTexture, MeshAsset, Node, SceneAsset, DEFAULT_IMPORT_COLOR,
};

/// Import a glTF or GLB file at `path` into a [`SceneAsset`].
///
/// Both text `.gltf` (with external or embedded buffers) and binary `.glb` are
/// supported — the `gltf` crate dispatches on the file. External buffer/`.bin`
/// resources are resolved relative to `path`.
///
/// # Errors
/// Returns [`ImportError`] if the file cannot be read or parsed, or if a mesh
/// primitive is missing required position data.
pub fn import_gltf(path: impl AsRef<Path>) -> Result<SceneAsset, ImportError> {
    let (document, buffers, images) = gltf::import(path.as_ref())?;
    build_scene(&document, &buffers, &images)
}

/// Import a glTF/GLB document from an in-memory byte slice.
///
/// Used for embedded fixtures and tests: the slice must be a self-contained
/// document (embedded/`data:` buffers or GLB), since there is no path to resolve
/// external resources against.
///
/// # Errors
/// Returns [`ImportError`] on a parse failure or missing geometry.
pub fn import_slice(bytes: &[u8]) -> Result<SceneAsset, ImportError> {
    let (document, buffers, images) = gltf::import_slice(bytes)?;
    build_scene(&document, &buffers, &images)
}

/// Shared scene builder over a parsed document + its buffer and image data.
fn build_scene(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<SceneAsset, ImportError> {
    // 1. Import every unique mesh once (deduplicated by glTF mesh index). A glTF
    //    "mesh" may hold several primitives; each becomes its own MeshAsset, and
    //    we remember the primitive->MeshAsset index range per glTF mesh so nodes
    //    can reference them.
    let mut scene = SceneAsset::default();
    // For each glTF mesh index, the range of MeshAsset indices it produced.
    let mut mesh_ranges: Vec<std::ops::Range<usize>> = Vec::new();

    for mesh in document.meshes() {
        let start = scene.meshes.len();
        for primitive in mesh.primitives() {
            let asset = import_primitive(&primitive, buffers, images)?;
            scene.meshes.push(asset);
        }
        mesh_ranges.push(start..scene.meshes.len());
    }

    // 2. Walk the default scene (or the first scene) and bake the node tree.
    let gltf_scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    if let Some(gltf_scene) = gltf_scene {
        for node in gltf_scene.nodes() {
            let idx = bake_node(&node, Mat4::IDENTITY, None, &mesh_ranges, &mut scene);
            scene.roots.push(idx);
        }
    }

    Ok(scene)
}

/// Recursively bake one glTF node (and its subtree) into flat [`Node`]s, pushing
/// them onto `scene.nodes` and returning the pushed node's index.
///
/// A node with a mesh that expanded into multiple primitives is represented as a
/// single node pointing at the *first* primitive's MeshAsset, with the remaining
/// primitives attached as child nodes sharing the same baked transform — so every
/// primitive is drawn without losing the hierarchy.
fn bake_node(
    node: &gltf::Node,
    parent_world: Mat4,
    parent: Option<usize>,
    mesh_ranges: &[std::ops::Range<usize>],
    scene: &mut SceneAsset,
) -> usize {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_world * local;
    let transform = bake_transform(world);

    // Which MeshAsset indices this node's glTF mesh maps to (may be several).
    let primitive_indices: Vec<usize> = node
        .mesh()
        .and_then(|m| mesh_ranges.get(m.index()))
        .map(|r| r.clone().collect())
        .unwrap_or_default();

    let first_mesh = primitive_indices.first().copied();

    // Reserve this node's slot.
    let self_index = scene.nodes.len();
    scene.nodes.push(Node {
        transform,
        mesh: first_mesh,
        children: Vec::new(),
        parent,
    });

    let mut children = Vec::new();

    // Extra primitives beyond the first become mesh-only child nodes sharing the
    // baked world transform (identity local, since `world` is already baked).
    for &extra in primitive_indices.iter().skip(1) {
        let child_index = scene.nodes.len();
        scene.nodes.push(Node {
            transform,
            mesh: Some(extra),
            children: Vec::new(),
            parent: Some(self_index),
        });
        children.push(child_index);
    }

    // Recurse into real glTF children.
    for child in node.children() {
        let child_index = bake_node(&child, world, Some(self_index), mesh_ranges, scene);
        children.push(child_index);
    }

    scene.nodes[self_index].children = children;
    self_index
}

/// Read one glTF primitive into a [`MeshAsset`].
///
/// The primitive's material is inspected for a base-color texture (and, best
/// effort, a normal and metallic-roughness texture). If a base-color texture is
/// present the mesh is packed on the `[pos,normal,uv]`
/// [`textured_vertex_layout`] and the decoded RGBA8 image is attached; otherwise
/// it stays on the default-color `[pos,normal,color]` layout. UVs are retained on
/// the asset either way.
fn import_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<MeshAsset, ImportError> {
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(ImportError::MissingPositions)?
        .collect();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|iter| iter.collect())
        .unwrap_or_default();

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|tc| tc.into_f32().collect())
        .unwrap_or_default();

    // Indices: use the primitive's index buffer, or synthesize a trivial one
    // (0..n) for a non-indexed primitive.
    let indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };

    // Normalize per-vertex attribute lengths to the position count.
    let mut normals = normals;
    normals.resize(positions.len(), [0.0, 1.0, 0.0]);
    let mut uvs = uvs;
    uvs.resize(positions.len(), [0.0, 0.0]);

    // Inspect the primitive's material for texture slots.
    let material = primitive.material();
    let pbr = material.pbr_metallic_roughness();
    let base_color_factor = pbr.base_color_factor();

    let base_color = pbr
        .base_color_texture()
        .and_then(|info| decode_texture(&info.texture(), images));
    let normal_map = material
        .normal_texture()
        .and_then(|info| decode_texture(&info.texture(), images));
    let metallic_roughness = pbr
        .metallic_roughness_texture()
        .and_then(|info| decode_texture(&info.texture(), images));

    // A base-color texture selects the `[pos,normal,uv]` textured layout so the
    // backend's textured pipeline can sample it; otherwise stay on the default
    // `[pos,normal,color]` path (untextured pixel-hash unaffected).
    let (vertices, layout) = if base_color.is_some() {
        (
            pack_textured_vertices(&positions, &normals, &uvs),
            textured_vertex_layout(),
        )
    } else {
        (
            pack_render_vertices(&positions, &normals, DEFAULT_IMPORT_COLOR),
            render_vertex_layout(),
        )
    };

    Ok(MeshAsset {
        vertices,
        indices,
        layout,
        positions,
        normals,
        uvs,
        base_color,
        base_color_factor,
        normal_map,
        metallic_roughness,
    })
}

/// Decode one glTF texture's source image into an RGBA8 [`BaseColorTexture`].
///
/// The `gltf` crate already decodes embedded/external images into
/// [`gltf::image::Data`] (raw pixels + a [`gltf::image::Format`]); this converts
/// whatever channel layout that is into tightly-packed RGBA8, the shape the
/// render seam's [`TextureData`](kaman_render_api::TextureData) accepts. Returns
/// `None` if the image index is out of range or the pixel data is malformed.
fn decode_texture(
    texture: &gltf::Texture,
    images: &[gltf::image::Data],
) -> Option<BaseColorTexture> {
    let data = images.get(texture.source().index())?;
    let rgba8 = to_rgba8(data)?;
    Some(BaseColorTexture {
        width: data.width,
        height: data.height,
        rgba8,
    })
}

/// Convert a decoded glTF image into tightly-packed RGBA8 bytes.
///
/// Handles the channel layouts the `gltf`/`image` decoders produce for the
/// common cases (RGB8/RGBA8, and single/dual-channel greyscale), expanding each
/// to 4 bytes per pixel with an opaque alpha default. Returns `None` if the pixel
/// buffer is too small for the declared dimensions.
fn to_rgba8(data: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let px = (data.width as usize).checked_mul(data.height as usize)?;
    let src = &data.pixels;
    let mut out = Vec::with_capacity(px * 4);
    match data.format {
        Format::R8G8B8A8 => {
            if src.len() < px * 4 {
                return None;
            }
            out.extend_from_slice(&src[..px * 4]);
        }
        Format::R8G8B8 => {
            if src.len() < px * 3 {
                return None;
            }
            for c in src[..px * 3].chunks_exact(3) {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Format::R8G8 => {
            if src.len() < px * 2 {
                return None;
            }
            for c in src[..px * 2].chunks_exact(2) {
                out.extend_from_slice(&[c[0], c[0], c[0], c[1]]);
            }
        }
        Format::R8 => {
            if src.len() < px {
                return None;
            }
            for &g in &src[..px] {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        // 16-bit formats are uncommon for base-color glTF textures; downsample
        // each 16-bit channel to its high byte so we still upload something
        // sensible rather than dropping the texture.
        Format::R16 | Format::R16G16 | Format::R16G16B16 | Format::R16G16B16A16 => {
            let channels = match data.format {
                Format::R16 => 1,
                Format::R16G16 => 2,
                Format::R16G16B16 => 3,
                _ => 4,
            };
            if src.len() < px * channels * 2 {
                return None;
            }
            for pixel in src[..px * channels * 2].chunks_exact(channels * 2) {
                let hi = |i: usize| pixel[i * 2 + 1];
                let (r, g, b, a) = match channels {
                    1 => (hi(0), hi(0), hi(0), 255),
                    2 => (hi(0), hi(0), hi(0), hi(1)),
                    3 => (hi(0), hi(1), hi(2), 255),
                    _ => (hi(0), hi(1), hi(2), hi(3)),
                };
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        // 32-bit float formats: rare; clamp+scale each channel's first byte.
        Format::R32G32B32FLOAT | Format::R32G32B32A32FLOAT => {
            let channels = if matches!(data.format, Format::R32G32B32FLOAT) {
                3
            } else {
                4
            };
            if src.len() < px * channels * 4 {
                return None;
            }
            for pixel in src[..px * channels * 4].chunks_exact(channels * 4) {
                let ch = |i: usize| {
                    let f = f32::from_le_bytes([
                        pixel[i * 4],
                        pixel[i * 4 + 1],
                        pixel[i * 4 + 2],
                        pixel[i * 4 + 3],
                    ]);
                    (f.clamp(0.0, 1.0) * 255.0) as u8
                };
                let a = if channels == 4 { ch(3) } else { 255 };
                out.extend_from_slice(&[ch(0), ch(1), ch(2), a]);
            }
        }
    }
    Some(out)
}
