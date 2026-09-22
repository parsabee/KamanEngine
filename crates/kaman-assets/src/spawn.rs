// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Spawn ECS entities from an imported [`SceneAsset`]'s node graph.
//!
//! This is the one place `kaman-assets` touches `kaman-ecs`. The dependency
//! direction is acyclic: `kaman-ecs` is a leaf (math + hecs + rapier) that never
//! depends on `kaman-assets`, so this helper adds a `kaman-assets → kaman-ecs`
//! edge and no cycle. Keeping the helper here (rather than in the game) lets any
//! title spawn imported scenes with one call while staying engine-generic — there
//! are no game concepts in this module.
//!
//! Each mesh-bearing node in the baked node tree becomes one entity carrying a
//! [`TransformComponent`] (the node's baked world transform), a
//! [`RenderComponent`] rebuilt from the mesh's packed `[pos,normal,color]`
//! geometry, and a [`StaticTag`] (imported static meshes do not move under
//! physics in v1).

use kaman_ecs::hecs::{Entity, World};
use kaman_ecs::{RenderComponent, RenderShape, StaticTag, TransformComponent};

use crate::scene::{MeshAsset, SceneAsset, RENDER_VERTEX_FLOATS};

/// Spawn one ECS entity per mesh-bearing node of `scene` into `world`, returning
/// the spawned entities.
///
/// Nodes without a mesh (pure transform groups) are skipped — their baked
/// transform is already folded into their descendants (the importer bakes world
/// transforms), so nothing is lost. Each spawned entity gets:
/// - [`TransformComponent`] from the node's baked world transform,
/// - [`RenderComponent`] rebuilt from the referenced [`MeshAsset`]'s geometry,
/// - [`StaticTag`].
pub fn spawn_scene(world: &mut World, scene: &SceneAsset) -> Vec<Entity> {
    let mut entities = Vec::new();
    for (_idx, node) in scene.mesh_nodes() {
        let Some(mesh_idx) = node.mesh else { continue };
        let Some(mesh) = scene.meshes.get(mesh_idx) else {
            continue;
        };
        let render = render_component_from_mesh(mesh);
        let entity = world.spawn((
            TransformComponent::new(node.transform),
            render,
            StaticTag,
        ));
        entities.push(entity);
    }
    entities
}

/// Rebuild an ECS [`RenderComponent`] (a `[f32; 9]` `[pos,normal,color]` mesh)
/// from an imported [`MeshAsset`]'s packed bytes.
///
/// The packed render bytes are the exact `[pos,normal,color]` interleave the ECS
/// mesh path expects, so this unpacks them back into the `[f32; 9]` vertex form
/// `RenderShape::Mesh` stores. The component's `color` field is taken from the
/// first vertex's packed color (the default import color).
#[must_use]
pub fn render_component_from_mesh(mesh: &MeshAsset) -> RenderComponent {
    let mut vertices: Vec<[f32; 9]> = Vec::with_capacity(mesh.vertex_count());
    let floats = bytes_to_f32(&mesh.vertices);
    for chunk in floats.chunks_exact(RENDER_VERTEX_FLOATS) {
        let mut v = [0.0f32; RENDER_VERTEX_FLOATS];
        v.copy_from_slice(chunk);
        vertices.push(v);
    }

    let color = vertices
        .first()
        .map(|v| [v[6], v[7], v[8]])
        .unwrap_or([0.75, 0.75, 0.78]);

    RenderComponent::new(
        color,
        RenderShape::Mesh {
            vertices,
            indices: mesh.indices.clone(),
        },
    )
}

/// Reinterpret tightly-packed native-endian `f32` bytes as an `f32` vector.
fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}
