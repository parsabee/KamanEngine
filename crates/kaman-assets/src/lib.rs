// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Engine-generic static-mesh asset import for KamanEngine.
//!
//! `kaman-assets` imports static **glTF** (`.gltf` / `.glb`) files into
//! engine-neutral geometry ([`SceneAsset`]) that is ready for the render seam,
//! and provides a load-once/dedup [`AssetCache`] plus an ECS [`spawn_scene`]
//! helper. It is **metal-free** and **engine-generic**: it depends only on
//! [`gltf`], [`kaman_math`], [`kaman_render_api`], and [`kaman_ecs`] — never on
//! `metal` and never on any game crate.
//!
//! # Import → seam mapping
//!
//! [`import_gltf`] walks the glTF default scene, **bakes** each node's transform
//! into world space (static v1: no skinning/animation), and reads each mesh's
//! position / normal / UV. Geometry is packed onto the canonical
//! `[position_xyz, normal_xyz, color_rgb]` render layout
//! ([`render_vertex_layout`]) with a **default vertex color**, so an imported
//! mesh renders immediately through the existing (untextured) Phong pipeline. The
//! parsed **UVs are retained** on [`MeshAsset::uvs`] — deliberately *not* packed
//! into the render bytes — so KE-0403 can build a UV-carrying, textured vertex
//! buffer without re-parsing the file. Each [`MeshAsset`] carries the packed
//! `vertices`, `indices`, and its [`VertexLayout`](kaman_render_api::VertexLayout),
//! ready to drop into [`MeshData`](kaman_render_api::MeshData).
//!
//! # Load once, reference by handle
//!
//! [`AssetCache`] parses (and optionally uploads) a file **once** and shares it
//! by handle across N instances — the KE-0103 "upload once, reference by handle"
//! rule applied to whole asset files. See the [`cache`] module.
//!
//! # Spawning
//!
//! [`spawn_scene`] turns a [`SceneAsset`]'s baked node graph into ECS entities
//! (`TransformComponent` + `RenderComponent` + `StaticTag`). It lives here (not in
//! a game crate) so any title reuses it; the `kaman-assets → kaman-ecs` edge is
//! acyclic. See the [`spawn`] module.
//!
//! # Engine-generic boundary
//!
//! Like `kaman-ecs`, this crate carries **no** game concepts. A unit test scans
//! the crate source for forbidden concept words so a game type cannot slip in.

#![deny(missing_docs)]

pub mod cache;
pub mod error;
pub mod import;
pub mod scene;
pub mod spawn;

pub use cache::{upload_scene, AssetCache, CachedAsset};
pub use error::ImportError;
pub use import::{import_gltf, import_slice};
pub use scene::{
    render_vertex_layout, textured_vertex_layout, BaseColorTexture, MeshAsset, Node, SceneAsset,
};
pub use spawn::{render_component_from_mesh, spawn_scene};

#[cfg(test)]
mod guard_tests {
    /// A-boundary guard: `kaman-assets` must stay engine-generic.
    ///
    /// Mirrors the `kaman-ecs` guard: it scans every source file in this crate,
    /// token by token, for any game-specific identifier. The forbidden words are
    /// assembled from ASCII byte codes so they never appear literally in this file
    /// (the test cannot false-positive on its own body). Matching is
    /// case-insensitive and whole-word.
    #[test]
    fn no_game_specific_symbols() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("cache.rs"),
            include_str!("error.rs"),
            include_str!("import.rs"),
            include_str!("scene.rs"),
            include_str!("spawn.rs"),
        ];

        // Game concepts, spelled from byte codes so no literal occurrence exists
        // here: [99,97,114], [114,111,97,100], [115,99,111,114,101],
        // [111,98,115,116,97,99,108,101], [108,97,110,101].
        let forbidden: Vec<String> = [
            &[99u8, 97, 114][..],
            &[114, 111, 97, 100][..],
            &[115, 99, 111, 114, 101][..],
            &[111, 98, 115, 116, 97, 99, 108, 101][..],
            &[108, 97, 110, 101][..],
        ]
        .iter()
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
        .collect();

        for src in sources {
            for token in src.split(|c: char| !c.is_ascii_alphanumeric()) {
                if token.is_empty() {
                    continue;
                }
                let lower = token.to_ascii_lowercase();
                for bad in &forbidden {
                    assert_ne!(
                        &lower, bad,
                        "game-specific identifier `{token}` found: \
                         kaman-assets must stay engine-generic",
                    );
                }
            }
        }
    }
}
