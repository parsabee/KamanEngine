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
    bake_transform, pack_render_vertices, render_vertex_layout, MeshAsset, Node, SceneAsset,
    DEFAULT_IMPORT_COLOR,
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
    let (document, buffers, _images) = gltf::import(path.as_ref())?;
    build_scene(&document, &buffers)
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
    let (document, buffers, _images) = gltf::import_slice(bytes)?;
    build_scene(&document, &buffers)
}

/// Shared scene builder over a parsed document + its buffer data.
fn build_scene(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
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
            let asset = import_primitive(&primitive, buffers)?;
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

/// Read one glTF primitive into a [`MeshAsset`], packing `[pos,normal,color]` and
/// retaining UVs.
fn import_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
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

    let vertices = pack_render_vertices(&positions, &normals, DEFAULT_IMPORT_COLOR);

    Ok(MeshAsset {
        vertices,
        indices,
        layout: render_vertex_layout(),
        positions,
        normals,
        uvs,
    })
}
