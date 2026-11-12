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

use kaman_math::glam::Mat4;
use kaman_render_api::{PipelineKind, VertexLayout};

/// Number of `f32`s in one packed render vertex: `[pos_xyz, normal_xyz, color_rgb]`.
pub const RENDER_VERTEX_FLOATS: usize = 9;

/// Byte stride of the packed `[pos,normal,color]` render vertex (36 bytes).
pub const RENDER_VERTEX_STRIDE: u32 = (RENDER_VERTEX_FLOATS * 4) as u32;

/// Number of `f32`s in one packed **textured** vertex: `[pos_xyz, normal_xyz, uv]`.
pub const TEXTURED_VERTEX_FLOATS: usize = 8;

/// Byte stride of the packed `[pos,normal,uv]` textured vertex (32 bytes).
pub const TEXTURED_VERTEX_STRIDE: u32 = (TEXTURED_VERTEX_FLOATS * 4) as u32;

/// The canonical interleaved `[position_xyz, normal_xyz, color_rgb]` render
/// layout that imported geometry is packed onto.
///
/// This matches the layout the built-in Phong pipeline binds against, so
/// imported meshes render immediately through the existing (untextured) pipeline
/// with a default vertex color. The parsed UVs are *not* in this layout — they
/// are retained separately on [`MeshAsset::uvs`] for the textured pipeline.
#[must_use]
pub fn render_vertex_layout() -> VertexLayout {
    PipelineKind::VertexColor.vertex_layout()
}

/// The interleaved `[position_xyz, normal_xyz, uv]` render layout that a mesh
/// carrying a base-color texture is packed onto.
///
/// Textured meshes swap the `color_rgb` attribute for a two-float `uv` at
/// location 2 (offset 24, 32-byte stride) so the backend's **textured** pipeline
/// can sample the bound base-color texture at each vertex's UV. Untextured meshes
/// stay on [`render_vertex_layout`] with a default vertex color — the two paths
/// are distinct pipelines, so the untextured (box) pixel-hash is unaffected.
#[must_use]
pub fn textured_vertex_layout() -> VertexLayout {
    PipelineKind::Textured.vertex_layout()
}

/// A decoded 2D base-color (albedo) texture retained on a [`MeshAsset`].
///
/// Pixels are 8-bit **RGBA**, row-major, top-left origin (`4 * width * height`
/// bytes) — exactly the shape the render seam's
/// [`TextureData`](kaman_render_api::TextureData) accepts. This is engine-generic
/// CPU-side data: there is no GPU type here. The importer decodes glTF images
/// (PNG/JPEG) into this form; the backend uploads it as RGBA8 with mipmaps
/// (ASTC compression is not implemented yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseColorTexture {
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Row-major 8-bit RGBA pixel bytes (`4 * width * height`).
    pub rgba8: Vec<u8>,
}

/// A single imported mesh: geometry packed for the render seam plus the parsed
/// attributes retained for later use.
///
/// The [`vertices`](Self::vertices) bytes are tightly interleaved on
/// [`layout`](Self::layout) (`[pos,normal,color]`, 36-byte stride) and, together
/// with [`indices`](Self::indices), are ready to hand to
/// [`MeshData`](kaman_render_api::MeshData). The per-vertex
/// [`uvs`](Self::uvs) are retained (not in the packed bytes) so the
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
    /// Per-vertex normals, one per packed vertex. When the source primitive has
    /// none they are flat face normals, as glTF requires: each triangle then has
    /// its own three vertices.
    pub normals: Vec<[f32; 3]>,
    /// Per-vertex texture coordinates, one per packed vertex. When this mesh has
    /// a [`base_color`](Self::base_color) texture these UVs are packed **into**
    /// [`vertices`](Self::vertices) (the `[pos,normal,uv]`
    /// [`textured_vertex_layout`]); otherwise they are retained here but not
    /// packed. Defaults to `[0, 0]` when the source mesh has no UV set.
    pub uvs: Vec<[f32; 2]>,
    /// The decoded base-color (albedo) texture from this primitive's glTF
    /// material, if it has one. `Some` ⇒ this mesh is packed on the
    /// `[pos,normal,uv]` [`textured_vertex_layout`] and should be drawn with the
    /// textured pipeline; `None` ⇒ the default-color `[pos,normal,color]` path.
    /// glTF base-colour images are sRGB-encoded: upload them with
    /// [`ColorSpace::Srgb`](kaman_render_api::ColorSpace::Srgb).
    pub base_color: Option<BaseColorTexture>,
    /// The material's base-color factor (linear RGBA), multiplied with the
    /// sampled base-color texture (or used alone when there is no texture).
    /// Defaults to opaque white.
    pub base_color_factor: [f32; 4],
    /// The decoded normal map, if the material has one (best-effort).
    /// RGBA8 like [`base_color`](Self::base_color); the tangent-space normal is
    /// in RGB. Plumbed toward a basic lit material.
    pub normal_map: Option<BaseColorTexture>,
    /// The decoded metallic-roughness texture, if present (best-effort). glTF
    /// packs roughness in G and metalness in B; retained RGBA8.
    pub metallic_roughness: Option<BaseColorTexture>,
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

    /// Whether this mesh carries a base-color texture (and is therefore packed on
    /// the `[pos,normal,uv]` [`textured_vertex_layout`] for the textured
    /// pipeline). `false` ⇒ the untextured, default-color `[pos,normal,color]`
    /// path.
    #[must_use]
    pub fn is_textured(&self) -> bool {
        self.base_color.is_some()
    }
}

/// A node in the imported scene's transform hierarchy.
///
/// Each node carries its **world** (baked) matrix, [`world`](Self::world) — the
/// static importer flattens the glTF node tree so no runtime hierarchy math is
/// needed — an optional [`mesh`](Self::mesh) index into
/// [`SceneAsset::meshes`](SceneAsset::meshes), and its child node indices.
///
/// The world matrix is kept whole rather than as position/rotation/scale: a
/// rotated child under a non-uniformly scaled parent is sheared, which a
/// [`Transform`](kaman_math::Transform) cannot represent.
#[derive(Debug, Clone)]
pub struct Node {
    /// The node's baked model-to-world matrix: every ancestor's local matrix
    /// times its own, exactly as glTF composes them.
    pub world: Mat4,
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
/// are handled separately). `positions.len()` drives the count; `normals` shorter than that
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

/// Pack parsed per-vertex attributes into the `[pos,normal,uv]` textured render
/// bytes.
///
/// Used for a mesh that has a base-color texture: positions and normals come
/// from the source mesh, and each vertex carries its UV instead of a color, so
/// the backend's textured pipeline can sample the bound base-color texture.
/// `positions.len()` drives the count; shorter `normals`/`uvs` are padded
/// (`+Y` / `[0, 0]`).
pub(crate) fn pack_textured_vertices(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(positions.len() * TEXTURED_VERTEX_STRIDE as usize);
    for (i, pos) in positions.iter().enumerate() {
        let n = normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);
        for f in pos {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in &n {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in &uv {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

/// The default per-vertex color imported geometry is packed with when the source
/// has no color (a mid-grey), so real glTF meshes are visible immediately.
pub const DEFAULT_IMPORT_COLOR: [f32; 3] = [0.75, 0.75, 0.78];
