// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Integration tests for the glTF importer, layout packing, and the dedup cache.
//!
//! The fixture (`fixtures/cube.gltf`) is a self-contained glTF 2.0 unit cube with
//! per-face normals and an embedded base64 buffer, so these tests need no binary
//! blob and are fully reproducible (regenerate via
//! `cargo run -p kaman-assets --example gen_cube`).

use std::io::Write;

use kaman_assets::scene::{
    textured_vertex_layout, DEFAULT_IMPORT_COLOR, RENDER_VERTEX_STRIDE, TEXTURED_VERTEX_STRIDE,
};
use kaman_assets::{
    import_gltf, import_slice, render_vertex_layout, AssetCache, MeshAsset, SceneAsset,
};
use kaman_render_api::{MeshData, MeshHandle, NullRenderer, RenderDevice, VertexFormat};

/// The committed cube fixture, embedded so slice-import tests are hermetic.
const CUBE_GLTF: &[u8] = include_bytes!("fixtures/cube.gltf");

/// The committed **textured** cube fixture (KE-0403): a cube with UVs, a material,
/// and an embedded base-color checkerboard PNG.
const TEXTURED_CUBE_GLTF: &[u8] = include_bytes!("fixtures/textured_cube.gltf");

fn cube_scene() -> SceneAsset {
    import_slice(CUBE_GLTF).expect("cube fixture parses")
}

fn textured_cube_scene() -> SceneAsset {
    // The `gltf` slice importer rejects `data:` **image** URIs
    // (`ExternalReferenceInSliceImport`), so write the fixture to a temp file and
    // import it by path — which resolves embedded images.
    //
    // Use a dir unique **per call** (process id + an atomic counter): multiple
    // tests call this concurrently under `cargo test`, and a process-id-only dir
    // let one test's `remove_dir_all` race another's read.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kaman-assets-tex-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("textured_cube.gltf");
    std::fs::write(&path, TEXTURED_CUBE_GLTF).unwrap();
    let scene = import_gltf(&path).expect("textured cube fixture parses");
    let _ = std::fs::remove_dir_all(&dir);
    scene
}

#[test]
fn parses_cube_vertex_and_index_counts() {
    let scene = cube_scene();
    assert_eq!(scene.meshes.len(), 1, "one mesh (one primitive)");
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.vertex_count(), 24, "cube has 24 verts (4 per face)");
    assert_eq!(mesh.indices.len(), 36, "cube has 12 triangles (36 indices)");
    assert_eq!(mesh.triangle_count(), 12);
    // Every index references a real vertex.
    for &i in &mesh.indices {
        assert!((i as usize) < mesh.vertex_count(), "index {i} in range");
    }
}

#[test]
fn parses_known_cube_positions_and_normals() {
    let mesh = &cube_scene().meshes[0];

    // A unit cube spans [-0.5, 0.5] on every axis.
    for p in &mesh.positions {
        for c in p {
            assert!((c.abs() - 0.5).abs() < 1e-6, "position component {c} is ±0.5");
        }
    }
    // Bounding box corners are all present.
    assert!(mesh.positions.contains(&[-0.5, -0.5, 0.5]));
    assert!(mesh.positions.contains(&[0.5, 0.5, -0.5]));

    // First face is +Z: its four normals point +Z.
    for n in &mesh.normals[0..4] {
        assert_eq!(*n, [0.0, 0.0, 1.0]);
    }
    // Normals are unit length.
    for n in &mesh.normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "normal {n:?} unit length");
    }
}

#[test]
fn node_tree_has_one_root_mesh_node() {
    let scene = cube_scene();
    assert_eq!(scene.roots.len(), 1, "single root node");
    let root = &scene.nodes[scene.roots[0]];
    assert_eq!(root.mesh, Some(0), "root draws mesh 0");
    assert!(root.parent.is_none(), "root has no parent");
    assert_eq!(scene.mesh_nodes().count(), 1, "one mesh-bearing node");
}

#[test]
fn packs_onto_canonical_render_layout() {
    let mesh = &cube_scene().meshes[0];

    // Layout is the `[pos,normal,color]` 36-byte layout.
    let layout = render_vertex_layout();
    assert_eq!(mesh.layout, layout);
    assert_eq!(layout.stride, RENDER_VERTEX_STRIDE);
    assert_eq!(layout.stride, 36);
    assert_eq!(layout.attributes.len(), 3);
    assert!(layout
        .attributes
        .iter()
        .all(|a| a.format == VertexFormat::Float32x3));
    assert_eq!(layout.attributes[0].offset, 0);
    assert_eq!(layout.attributes[1].offset, 12);
    assert_eq!(layout.attributes[2].offset, 24);

    // Packed bytes are exactly stride * vertex_count.
    assert_eq!(
        mesh.vertices.len(),
        mesh.vertex_count() * RENDER_VERTEX_STRIDE as usize
    );
}

#[test]
fn packed_bytes_match_parsed_attributes_with_default_color() {
    let mesh = &cube_scene().meshes[0];
    let floats: Vec<f32> = mesh
        .vertices
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    for (i, chunk) in floats.chunks_exact(9).enumerate() {
        assert_eq!([chunk[0], chunk[1], chunk[2]], mesh.positions[i], "pos {i}");
        assert_eq!([chunk[3], chunk[4], chunk[5]], mesh.normals[i], "normal {i}");
        assert_eq!(
            [chunk[6], chunk[7], chunk[8]],
            DEFAULT_IMPORT_COLOR,
            "default color {i}"
        );
    }
}

#[test]
fn uvs_are_retained_even_when_source_has_none() {
    // The cube fixture has no UV set; the importer still retains a UV per vertex
    // (defaulted), one per packed vertex, for KE-0403 to consume.
    let mesh = &cube_scene().meshes[0];
    assert_eq!(mesh.uvs.len(), mesh.vertex_count(), "one UV per vertex");
    assert!(mesh.uvs.iter().all(|uv| *uv == [0.0, 0.0]));
}

#[test]
fn mesh_data_round_trips_through_the_seam() {
    // The packed MeshAsset drops straight into MeshData and uploads on a device.
    let scene = cube_scene();
    let mesh: &MeshAsset = &scene.meshes[0];
    let mut device = NullRenderer::new();
    let handle: MeshHandle = device.create_mesh(&MeshData {
        vertices: &mesh.vertices,
        indices: &mesh.indices,
        layout: mesh.layout.clone(),
    });
    device.destroy_mesh(handle);
}

#[test]
fn committed_game_asset_imports_from_path() {
    // The asset the game loads is a real file on disk (not gitignored).
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../games/playable-demo/assets/cube.gltf"
    );
    let scene = import_gltf(path).expect("committed cube.gltf imports");
    assert_eq!(scene.meshes[0].vertex_count(), 24);
    // KE-0403: the committed game asset is now textured (base-color checkerboard).
    assert!(
        scene.meshes[0].is_textured(),
        "the committed game asset carries a base-color texture"
    );
}

#[test]
fn parses_material_base_color_texture() {
    // The textured fixture's primitive references a material with a base-color
    // texture; the importer decodes it to RGBA8 and attaches it to the mesh.
    let scene = textured_cube_scene();
    let mesh = &scene.meshes[0];
    assert!(mesh.is_textured(), "mesh has a base-color texture");
    let tex = mesh.base_color.as_ref().expect("decoded base-color texture");
    // The generator writes an 8x8 checkerboard.
    assert_eq!(tex.width, 8);
    assert_eq!(tex.height, 8);
    assert_eq!(
        tex.rgba8.len(),
        (tex.width * tex.height * 4) as usize,
        "tightly-packed RGBA8"
    );
    // Checkerboard ⇒ the decoded pixels are non-uniform (not all one color).
    let first4 = &tex.rgba8[0..4];
    assert!(
        tex.rgba8.chunks_exact(4).any(|p| p != first4),
        "base-color texture is non-uniform (checkerboard decoded)"
    );
    // Opaque white base-color factor from the material.
    assert_eq!(mesh.base_color_factor, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn textured_mesh_packs_pos_normal_uv_layout() {
    let scene = textured_cube_scene();
    let mesh = &scene.meshes[0];

    // A textured mesh packs the `[pos,normal,uv]` layout, not `[pos,normal,color]`.
    assert_eq!(mesh.layout, textured_vertex_layout());
    assert_eq!(mesh.layout.stride, TEXTURED_VERTEX_STRIDE);
    assert_eq!(mesh.layout.stride, 32);
    assert_eq!(mesh.layout.attributes.len(), 3);
    assert_eq!(
        mesh.layout.attributes[2].format,
        VertexFormat::Float32x2,
        "attribute 2 is UV (2 floats), not color"
    );
    assert_eq!(
        mesh.vertices.len(),
        mesh.vertex_count() * TEXTURED_VERTEX_STRIDE as usize
    );

    // The packed UV bytes match the retained UVs (offset 24, 8 bytes/vertex).
    let stride = TEXTURED_VERTEX_STRIDE as usize;
    for (i, uv) in mesh.uvs.iter().enumerate() {
        let base = i * stride + 24;
        let u = f32::from_ne_bytes(mesh.vertices[base..base + 4].try_into().unwrap());
        let v = f32::from_ne_bytes(mesh.vertices[base + 4..base + 8].try_into().unwrap());
        assert!((u - uv[0]).abs() < 1e-6 && (v - uv[1]).abs() < 1e-6, "uv {i}");
    }
    // The fixture's UVs span the full 0..1 range (not all default [0,0]).
    assert!(mesh.uvs.iter().any(|uv| *uv != [0.0, 0.0]), "real UVs parsed");
}

#[test]
fn untextured_mesh_keeps_default_color_layout() {
    // The original (untextured) fixture stays on the `[pos,normal,color]` path.
    let mesh = &cube_scene().meshes[0];
    assert!(!mesh.is_textured());
    assert!(mesh.base_color.is_none());
    assert_eq!(mesh.layout, render_vertex_layout());
    assert_eq!(mesh.layout.stride, RENDER_VERTEX_STRIDE);
}

#[test]
fn cache_parses_each_path_once() {
    // Write two distinct copies of the fixture to temp files, then load each
    // several times. The cache must parse each path exactly once.
    let dir = std::env::temp_dir().join(format!("kaman-assets-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.gltf");
    let b = dir.join("b.gltf");
    for p in [&a, &b] {
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(CUBE_GLTF).unwrap();
    }

    let mut cache = AssetCache::new();
    // Repeated parse-only loads of `a` parse once.
    let first = cache.load_parse_only(&a).unwrap();
    let second = cache.load_parse_only(&a).unwrap();
    assert_eq!(cache.parse_count(), 1, "a parsed once across two loads");
    // Same underlying Arc-shared scene.
    assert!(std::sync::Arc::ptr_eq(&first.scene, &second.scene));

    // A different path parses once more.
    cache.load_parse_only(&b).unwrap();
    assert_eq!(cache.parse_count(), 2, "b parsed once");
    assert_eq!(cache.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cache_uploads_meshes_once_and_shares_handles() {
    let dir = std::env::temp_dir().join(format!("kaman-assets-upload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cube.gltf");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(CUBE_GLTF)
        .unwrap();

    let mut device = NullRenderer::new();
    let mut cache = AssetCache::new();

    let first = cache.load(&mut device, &path).unwrap();
    assert_eq!(first.mesh_handles.len(), 1, "one uploaded mesh handle");
    let first_handle = first.mesh_handles[0];

    // A second instance loads the same path: no re-parse, same handle shared.
    let second = cache.load(&mut device, &path).unwrap();
    assert_eq!(cache.parse_count(), 1, "parsed once for both instances");
    assert_eq!(second.mesh_handles[0], first_handle, "handle shared");

    let _ = std::fs::remove_dir_all(&dir);
}
