// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Asset loading: import glTF models/textures once through `kaman-assets` and
//! fit them to the road.
//!
//! Every model the demo draws (the player car, the traffic cars, the roadside
//! buildings, the road tile, the skyline backdrop) is a committed `.glb`/`.gltf`
//! file loaded exactly once via [`AssetCache`] and then shared by handle
//! (KE-0103's "upload once, reference by handle" rule, applied to whole asset
//! files). This module holds the two load paths — [`load_model`] for a
//! multi-part model whose primitives may or may not carry a texture, and
//! [`load_textured_mesh`] for a single always-textured mesh — plus the *fit*
//! functions ([`fit_transform`], [`building_fit`]) that turn a model's authored
//! bounds into a placement transform, and the small helpers the render pass
//! ([`crate::render`]) uses to compose and pick among loaded parts.

use kaman_assets::AssetCache;
use kaman_core::Renderer;
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render_api::{MeshHandle, TextureData, TextureHandle, VertexLayout};

use crate::config::{BUILDING_FOOTPRINT, CAR_BOTTOM, CAR_LEN, CAR_YAW};

/// One drawable piece of an imported car: its `(fitted transform, mesh handle,
/// base-color texture)`. A textured part (`Some`) draws on the textured pipeline
/// with its texture bound; an untextured part (`None`) draws on the Phong pipeline
/// with the material base-color the importer packed as its vertex color.
pub(crate) type CarPart = (Transform, MeshHandle, Option<TextureHandle>);

/// Import a model through the asset cache and return its drawable parts: one
/// `(fitted transform, mesh handle, base-color texture)` per mesh-node. The file is
/// parsed + its meshes uploaded exactly once (KE-0103); each mesh's decoded
/// base-color image is uploaded here via `create_texture` (the cache uploads
/// meshes, not textures). `fit` computes the model→world fit transform from the
/// scene bounds ([`fit_transform`] for cars, [`building_fit`] for buildings); each
/// part's stored transform is `fit ∘ node.transform` so the render pass only
/// composes it with the entity's placement.
pub(crate) fn load_model(
    cache: &mut AssetCache,
    renderer: &mut dyn Renderer,
    path: &str,
    fit: fn(&kaman_assets::SceneAsset) -> Transform,
) -> Vec<CarPart> {
    let asset = cache
        .load(renderer, path)
        .unwrap_or_else(|e| panic!("import model asset {path}: {e}"));

    let fit = fit(&asset.scene);

    asset
        .scene
        .mesh_nodes()
        .map(|(_, node)| {
            let mesh_idx = node.mesh.expect("mesh_nodes yields only mesh-bearing nodes");
            let local = compose(fit, &node.transform);
            let handle = asset.mesh_handles[mesh_idx];
            // Upload this mesh's base-color image, if it has one, so the textured
            // pipeline can sample it. A mesh with no texture stays `None` and is
            // drawn on the untextured pipeline.
            let texture = asset.scene.meshes[mesh_idx].base_color.as_ref().map(|tex| {
                renderer.create_texture(&TextureData {
                    width: tex.width,
                    height: tex.height,
                    rgba8: &tex.rgba8,
                })
            });
            (local, handle, texture)
        })
        .collect()
}

/// Compute the transform that fits a building prefab: a uniform scale so its
/// horizontal footprint is [`BUILDING_FOOTPRINT`] (height scales with it, keeping
/// tall/short character), centered in `X`/`Z`, with its **underside at the model
/// origin** so the spawn transform drops the base onto the ground plane. No
/// rotation — buildings keep their authored upright orientation.
pub(crate) fn building_fit(scene: &kaman_assets::SceneAsset) -> Transform {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (_, node) in scene.mesh_nodes() {
        let mesh = &scene.meshes[node.mesh.expect("mesh node has a mesh")];
        for p in &mesh.positions {
            let w = node.transform.transform_point(Vec3::from_array(*p));
            min = min.min(w);
            max = max.max(w);
        }
    }

    let size = max - min;
    let footprint = size.x.max(size.z).max(f32::EPSILON);
    let scale = BUILDING_FOOTPRINT / footprint;

    // Pivot at the footprint center / underside → origin, so `base = 0`, centered.
    let pivot = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    Transform {
        position: -(pivot * scale),
        rotation: Quat::IDENTITY,
        scale: Vec3::splat(scale),
    }
}

/// Compute the transform that fits an imported car model to the road: a uniform
/// scale so its longer horizontal extent is [`CAR_LEN`], a [`CAR_YAW`] rotation so
/// it faces the travel direction, and a translation centering it in `X`/`Z` with
/// its underside at [`CAR_BOTTOM`]. Robust to any model's authored scale /
/// orientation, since it works from the baked world-space bounds of the geometry.
pub(crate) fn fit_transform(scene: &kaman_assets::SceneAsset) -> Transform {
    // World-space AABB over every mesh-node's baked vertices.
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (_, node) in scene.mesh_nodes() {
        let mesh = &scene.meshes[node.mesh.expect("mesh node has a mesh")];
        for p in &mesh.positions {
            let w = node.transform.transform_point(Vec3::from_array(*p));
            min = min.min(w);
            max = max.max(w);
        }
    }

    let size = max - min;
    let horizontal = size.x.max(size.z).max(f32::EPSILON);
    let scale = CAR_LEN / horizontal;
    let rotation = Quat::from_rotation_y(CAR_YAW);

    // Pivot: footprint center in X/Z, underside in Y. Scaling + rotating about the
    // pivot keeps the car centered and level; then lift the underside to CAR_BOTTOM.
    let pivot = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    let translation = -(rotation * (pivot * scale)) + Vec3::new(0.0, CAR_BOTTOM, 0.0);

    Transform {
        position: translation,
        rotation,
        scale: Vec3::splat(scale),
    }
}

/// Import a **single-mesh textured** glTF (a unit quad whose material carries an
/// embedded base-color PNG — the asphalt road tile, KE-0704, or the skyline
/// billboard, KE-0705) and return `(mesh handle, base-color texture handle)`.
///
/// The importer packs the quad on the `[pos,normal,uv]` textured layout and
/// `AssetCache::load` uploads it as a textured mesh (one handle); the decoded
/// base-color RGBA8 is handed straight to `create_texture`. Parsed + uploaded once
/// (KE-0103); the mesh's node transform is identity (the quad is a unit tile
/// placed/scaled by the caller's transform), so only its handle is needed.
pub(crate) fn load_textured_mesh(
    cache: &mut AssetCache,
    renderer: &mut dyn Renderer,
    path: &str,
) -> (MeshHandle, TextureHandle) {
    let asset = cache
        .load(renderer, path)
        .unwrap_or_else(|e| panic!("import textured asset {path}: {e}"));

    // A single-mesh glTF; grab its uploaded handle and its base-color image.
    let mesh = *asset
        .mesh_handles
        .first()
        .expect("textured asset has one uploaded mesh");
    let base_color = asset.scene.meshes[0]
        .base_color
        .as_ref()
        .expect("textured mesh carries a base-color texture");
    let texture = renderer.create_texture(&TextureData {
        width: base_color.width,
        height: base_color.height,
        rgba8: &base_color.rgba8,
    });
    (mesh, texture)
}

/// Pick the parts of one of `models` by index, falling back to the first model
/// when the index is out of range (so a stale/oversized variant never panics).
pub(crate) fn pick_model(models: &[Vec<CarPart>], i: usize) -> &[CarPart] {
    models
        .get(i)
        .or_else(|| models.first())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Compose a `parent` world transform with a `child` (local) transform, so an
/// imported mesh part draws at its entity's placement times its baked node
/// transform. Done via matrices so scale/rotation/translation all combine
/// correctly (the demo's car nodes are authored at the origin, but this stays
/// correct for any baked hierarchy).
pub(crate) fn compose(parent: Transform, child: &Transform) -> Transform {
    let m = parent.to_matrix() * child.to_matrix();
    let (scale, rotation, position) = m.to_scale_rotation_translation();
    Transform {
        position,
        rotation,
        scale,
    }
}

/// The untextured `[pos,normal,color]` layout (36-byte stride) every procedural
/// mesh uses — the same layout the untextured Phong pipeline binds.
pub(crate) fn color_layout() -> VertexLayout {
    kaman_assets::render_vertex_layout()
}

/// The textured `[pos,normal,uv]` layout (32-byte stride) the road tiles use — the
/// same layout the textured pipeline binds and the importer packs the road quad on.
pub(crate) fn textured_layout() -> VertexLayout {
    kaman_assets::textured_vertex_layout()
}
