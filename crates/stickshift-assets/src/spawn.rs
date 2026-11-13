// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Spawn ECS entities from an imported [`SceneAsset`]'s node graph.
//!
//! This is the one place `stickshift-assets` touches `stickshift-ecs`. The dependency
//! direction is acyclic: `stickshift-ecs` is a leaf (math + hecs + rapier) that never
//! depends on `stickshift-assets`, so this helper adds a `stickshift-assets → stickshift-ecs`
//! edge and no cycle. Keeping the helper here (rather than in the game) lets any
//! title spawn imported scenes with one call while staying engine-generic — there
//! are no game concepts in this module.
//!
//! Each mesh-bearing node in the baked node tree becomes one entity carrying a
//! [`TransformComponent`] (the node's baked world transform), a
//! [`RenderComponent`] rebuilt from the mesh's packed `[pos,normal,color]`
//! geometry, and a [`StaticTag`] (imported static meshes do not move under
//! physics in v1).
//!
//! A [`TransformComponent`] holds position, rotation and scale, which cannot
//! express shear. When a node's world matrix has some (a rotated child under a
//! non-uniformly scaled parent), the entity keeps only the translation and the
//! rest of the matrix is baked into its vertices, so the entity still draws
//! exactly where the glTF puts it.

use stickshift_ecs::hecs::{Entity, World};
use stickshift_ecs::{RenderComponent, RenderShape, StaticTag, TransformComponent};
use stickshift_math::glam::{Mat3, Mat4, Vec3};
use stickshift_math::Transform;

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
        let (transform, residual) = split_world(node.world);
        let render = baked_render_component(mesh, residual);
        let entity = world.spawn((TransformComponent::new(transform), render, StaticTag));
        entities.push(entity);
    }
    entities
}

/// Rebuild an ECS [`RenderComponent`] (a `[f32; 9]` `[pos,normal,color]` mesh)
/// from an imported [`MeshAsset`]'s parsed attributes.
///
/// The ECS mesh path stores `[pos,normal,color]` vertices, so this rebuilds them
/// from the asset's parsed `positions`/`normals` (not the packed bytes, which
/// differ between the untextured `[pos,normal,color]` and textured
/// `[pos,normal,uv]` layouts). A textured mesh's color is taken from its material
/// base-color factor; an untextured mesh uses the packed default import color.
#[must_use]
pub fn render_component_from_mesh(mesh: &MeshAsset) -> RenderComponent {
    baked_render_component(mesh, None)
}

/// Split a node's world matrix into what a [`TransformComponent`] can hold and
/// a linear residual to bake into the vertices: `(trs, None)` when position,
/// rotation and scale reproduce `world`, else `(translation only, Some(3×3))`.
fn split_world(world: Mat4) -> (Transform, Option<Mat3>) {
    let (scale, rotation, position) = world.to_scale_rotation_translation();
    let trs = Transform {
        position,
        rotation,
        scale,
    };
    let magnitude = world
        .to_cols_array()
        .iter()
        .fold(1.0f32, |m, v| m.max(v.abs()));
    if trs.to_matrix().abs_diff_eq(world, 1e-5 * magnitude) {
        (trs, None)
    } else {
        (
            Transform::from_position(position),
            Some(Mat3::from_mat4(world)),
        )
    }
}

/// [`render_component_from_mesh`], with the mesh's positions carried through
/// `linear` and its normals through its inverse-transpose, when given.
fn baked_render_component(mesh: &MeshAsset, linear: Option<Mat3>) -> RenderComponent {
    // Color: base-color factor for a textured mesh, else the packed vertex color
    // (the default import color) recovered from the first `[pos,normal,color]`
    // record.
    let color = if mesh.is_textured() {
        let f = mesh.base_color_factor;
        [f[0], f[1], f[2]]
    } else {
        bytes_to_f32(&mesh.vertices)
            .chunks_exact(RENDER_VERTEX_FLOATS)
            .next()
            .map(|c| [c[6], c[7], c[8]])
            .unwrap_or([0.75, 0.75, 0.78])
    };

    let carried = linear.map(|m| (m, m.inverse().transpose()));
    let vertices: Vec<[f32; 9]> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut p = *p;
            let mut n = mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            if let Some((m, normal_m)) = carried {
                p = (m * Vec3::from_array(p)).to_array();
                n = (normal_m * Vec3::from_array(n))
                    .normalize_or_zero()
                    .to_array();
            }
            [
                p[0], p[1], p[2], n[0], n[1], n[2], color[0], color[1], color[2],
            ]
        })
        .collect();

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
