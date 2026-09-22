// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Imported scene data: [`SceneAsset`], [`MeshAsset`], and the [`Node`] tree.
//!
//! These are plain-data structures produced by the [importer](crate::import) and
//! consumed either directly (packed bytes ready for the render seam's
//! [`MeshData`](kaman_render_api::MeshData)) or via the [`spawn`](crate::spawn)
//! helper. There are no GPU types here and no game concepts — this is
//! engine-generic geometry plus a baked transform hierarchy.

use kaman_math::Transform;
use kaman_render_api::{VertexAttribute, VertexFormat, VertexLayout};

/// Number of `f32`s in one packed render vertex: `[pos_xyz, normal_xyz, color_rgb]`.
pub const RENDER_VERTEX_FLOATS: usize = 9;

/// Byte stride of the packed `[pos,normal,color]` render vertex (36 bytes).
pub const RENDER_VERTEX_STRIDE: u32 = (RENDER_VERTEX_FLOATS * 4) as u32;

/// The canonical interleaved `[position_xyz, normal_xyz, color_rgb]` render
/// layout that imported geometry is packed onto.
///
/// This matches the layout the built-in Phong pipeline binds against, so
/// imported meshes render immediately through the existing (untextured) pipeline
/// with a default vertex color. The parsed UVs are *not* in this layout — they
/// are retained separately on [`MeshAsset::uvs`] for KE-0403's textured pipeline.
#[must_use]
pub fn render_vertex_layout() -> VertexLayout {
    VertexLayout::new(
        RENDER_VERTEX_STRIDE,
        vec![
            VertexAttribute {
                location: 0,
                offset: 0,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 1,
                offset: 12,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 2,
                offset: 24,
                format: VertexFormat::Float32x3,
            },
        ],
    )
}

/// A single imported mesh: geometry packed for the render seam plus the parsed
/// attributes retained for later use.
///
/// The [`vertices`](Self::vertices) bytes are tightly interleaved on
/// [`layout`](Self::layout) (`[pos,normal,color]`, 36-byte stride) and, together
/// with [`indices`](Self::indices), are ready to hand to
/// [`MeshData`](kaman_render_api::MeshData). The per-vertex
/// [`uvs`](Self::uvs) are retained (not in the packed bytes) so the KE-0403
/// textured pipeline can build a UV-carrying vertex buffer without re-parsing the
/// source file.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshAsset {
    /// Tightly-packed `[pos,normal,color]` vertex bytes for the render seam.
    pub vertices: Vec<u8>,
    /// 32-bit triangle indices into the packed vertex array.
    pub indices: Vec<u32>,
    /// The layout describing [`vertices`](Self::vertices) — the canonical
    /// `[pos,normal,color]` render layout.
    pub layout: VertexLayout,
    /// Per-vertex model-space positions, one per packed vertex (parsed source).
    pub positions: Vec<[f32; 3]>,
    /// Per-vertex normals, one per packed vertex (parsed source; defaulted to
    /// `+Y` when the source mesh has none).
    pub normals: Vec<[f32; 3]>,
    /// Per-vertex texture coordinates, one per packed vertex — **retained for
    /// KE-0403**, deliberately not packed into [`vertices`](Self::vertices).
    /// Defaults to `[0, 0]` when the source mesh has no UV set.
    pub uvs: Vec<[f32; 2]>,
}

impl MeshAsset {
    /// Number of packed vertices in this mesh.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Number of triangles (index count / 3).
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A node in the imported scene's transform hierarchy.
///
/// Each node carries a **world** (baked) [`transform`](Self::transform) — the
/// static v1 importer flattens the glTF node tree so no runtime hierarchy math is
/// needed — an optional [`mesh`](Self::mesh) index into
/// [`SceneAsset::meshes`](SceneAsset::meshes), and its child node indices.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node's baked world-space transform.
    pub transform: Transform,
    /// Index into [`SceneAsset::meshes`] if this node draws a mesh, else `None`.
    pub mesh: Option<usize>,
    /// Indices (into [`SceneAsset::nodes`]) of this node's children.
    pub children: Vec<usize>,
    /// Index (into [`SceneAsset::nodes`]) of this node's parent, or `None` for a
    /// scene root. Lets a consumer walk the tree in either direction.
    pub parent: Option<usize>,
}

/// A fully imported scene: its meshes and its (baked) node tree.
///
/// Produced by [`import_gltf`](crate::import_gltf) /
/// [`import_slice`](crate::import_slice). Meshes are shared by index across
/// nodes, so an instanced source mesh is parsed once and referenced N times.
#[derive(Debug, Clone, Default)]
pub struct SceneAsset {
    /// Unique meshes referenced by the node tree, in import order.
    pub meshes: Vec<MeshAsset>,
    /// All nodes in the scene; [`roots`](Self::roots) indexes the top level.
    pub nodes: Vec<Node>,
    /// Indices into [`nodes`](Self::nodes) of the scene's root nodes.
    pub roots: Vec<usize>,
}

impl SceneAsset {
    /// Total vertex count across every mesh in the scene.
    #[must_use]
    pub fn total_vertices(&self) -> usize {
        self.meshes.iter().map(MeshAsset::vertex_count).sum()
    }

    /// Iterate `(node_index, &node)` over only the nodes that draw a mesh.
    pub fn mesh_nodes(&self) -> impl Iterator<Item = (usize, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.mesh.is_some())
    }
}

/// Pack parsed per-vertex attributes into the canonical `[pos,normal,color]`
/// render bytes with a single default color for every vertex.
///
/// Positions and normals come from the source mesh; the color is the caller's
/// default (imported geometry has no per-vertex color in v1 — materials/textures
/// are KE-0403). `positions.len()` drives the count; `normals` shorter than that
/// are padded with `+Y`.
pub(crate) fn pack_render_vertices(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    default_color: [f32; 3],
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(positions.len() * RENDER_VERTEX_STRIDE as usize);
    for (i, pos) in positions.iter().enumerate() {
        let n = normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        for f in pos {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in &n {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in &default_color {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

/// The default per-vertex color imported geometry is packed with when the source
/// has no color (a mid-grey), so real glTF meshes are visible immediately.
pub const DEFAULT_IMPORT_COLOR: [f32; 3] = [0.75, 0.75, 0.78];

/// Compose a glTF node's local TRS (given as a column-major 4×4 matrix from the
/// `gltf` crate) into a [`Transform`], multiplied under a parent world matrix.
pub(crate) fn bake_transform(world: kaman_math::glam::Mat4) -> Transform {
    let (scale, rotation, translation) = world.to_scale_rotation_translation();
    Transform {
        position: translation,
        rotation,
        scale,
    }
}
