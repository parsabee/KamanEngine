// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Integration tests for the glTF importer, layout packing, and the dedup cache.
//!
//! The fixture (`fixtures/cube.gltf`) is a self-contained glTF 2.0 unit cube with
//! per-face normals and an embedded base64 buffer, so these tests need no binary
//! blob and are fully reproducible (regenerate via
//! `cargo run -p stickshift-assets --example gen_cube`).

use std::io::Write;

use stickshift_assets::scene::{
    textured_vertex_layout, DEFAULT_IMPORT_COLOR, RENDER_VERTEX_STRIDE, TEXTURED_VERTEX_STRIDE,
};
use stickshift_assets::{
    import_gltf, import_slice, render_vertex_layout, AssetCache, ImportError, MeshAsset, SceneAsset,
};
use stickshift_render_api::{MeshData, MeshHandle, NullRenderer, RenderDevice, VertexFormat};

/// The committed cube fixture, embedded so slice-import tests are hermetic.
const CUBE_GLTF: &[u8] = include_bytes!("fixtures/cube.gltf");

/// The committed **textured** cube fixture: a cube with UVs, a material,
/// and an embedded base-color checkerboard PNG.
const TEXTURED_CUBE_GLTF: &[u8] = include_bytes!("fixtures/textured_cube.gltf");

/// A one-triangle fixture whose material carries a distinctive (non-white,
/// non-default) `baseColorFactor` and no texture. Exercises the
/// untextured path taking its packed vertex color from the material factor.
const MATERIAL_COLOR_GLTF: &[u8] = include_bytes!("fixtures/material_color.gltf");

fn cube_scene() -> SceneAsset {
    import_slice(CUBE_GLTF).expect("cube fixture parses")
}

fn textured_cube_scene() -> SceneAsset {
    // The fixture embeds its PNG as a `data:` URI, which a slice import decodes.
    import_slice(TEXTURED_CUBE_GLTF).expect("textured cube fixture parses")
}

/// Import a copy of `bytes` written to a temporary `.gltf` file, by path.
fn import_via_file(bytes: &[u8]) -> SceneAsset {
    // A dir unique per call (process id + counter): tests run concurrently.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "stickshift-assets-file-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("model.gltf");
    std::fs::write(&path, bytes).unwrap();
    let scene = import_gltf(&path).expect("imports by path");
    let _ = std::fs::remove_dir_all(&dir);
    scene
}

/// Wrap a glTF JSON document and its binary chunk into a GLB.
fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin = bin.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn le_bytes<T: Copy, const N: usize>(values: &[T], to: fn(T) -> [u8; N]) -> Vec<u8> {
    values.iter().flat_map(|&v| to(v)).collect()
}

/// A GLB with one primitive over float `positions` and, optionally, an index
/// accessor over `(bytes, component type, count)`.
fn primitive_glb(mode: u32, positions: &[f32], indices: Option<(&[u8], u32, usize)>) -> Vec<u8> {
    let count = positions.len() / 3;
    primitive_glb_with(mode, positions, indices, ("VEC3", count), None)
}

/// [`primitive_glb`] with the POSITION accessor's `(type, count)` and its
/// buffer view's `byteStride` spelled out, so a test can make them malformed.
fn primitive_glb_with(
    mode: u32,
    positions: &[f32],
    indices: Option<(&[u8], u32, usize)>,
    (pos_type, pos_count): (&str, usize),
    pos_stride: Option<usize>,
) -> Vec<u8> {
    let mut bin = le_bytes(positions, f32::to_le_bytes);
    let pos_len = bin.len();
    let stride = pos_stride.map_or(String::new(), |s| format!(r#", "byteStride": {s}"#));
    let (index_view, index_accessor, index_ref) = match indices {
        Some((bytes, component, count)) => {
            let offset = bin.len();
            bin.extend_from_slice(bytes);
            (
                format!(
                    r#", {{"buffer": 0, "byteOffset": {offset}, "byteLength": {}}}"#,
                    bytes.len()
                ),
                format!(
                    r#", {{"bufferView": 1, "componentType": {component}, "count": {count}, "type": "SCALAR"}}"#
                ),
                r#", "indices": 1"#.to_string(),
            )
        }
        None => Default::default(),
    };
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "buffers": [{{"byteLength": {}}}],
  "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {pos_len}{stride}}}{index_view}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": {pos_count}, "type": "{pos_type}",
      "min": [0,0,0], "max": [1,1,0]}}{index_accessor}
  ],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "mode": {mode}{index_ref}}}]}}],
  "nodes": [{{"mesh": 0}}],
  "scenes": [{{"nodes": [0]}}],
  "scene": 0
}}"#,
        bin.len(),
    );
    glb(&json, &bin)
}

/// The four corners of a unit quad in the XY plane.
const QUAD: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0];

fn u32_indices(indices: &[u32]) -> Vec<u8> {
    le_bytes(indices, u32::to_le_bytes)
}

/// The mesh's triangle corners, in index order, as positions. These primitives
/// carry no normals, so the importer flat-shades them and may duplicate
/// vertices; comparing corners checks the topology either way.
fn corners(mesh: &MeshAsset) -> Vec<[f32; 3]> {
    mesh.indices
        .iter()
        .map(|&i| mesh.positions[i as usize])
        .collect()
}

/// The corners `indices` name in a flat `positions` array.
fn corners_of(positions: &[f32], indices: &[u32]) -> Vec<[f32; 3]> {
    indices
        .iter()
        .map(|&i| {
            let i = i as usize * 3;
            [positions[i], positions[i + 1], positions[i + 2]]
        })
        .collect()
}

/// glTF primitive modes.
const LINES: u32 = 1;
const TRIANGLES: u32 = 4;
const TRIANGLE_STRIP: u32 = 5;
const TRIANGLE_FAN: u32 = 6;
const U8: u32 = 5121;
const U16: u32 = 5123;
const U32: u32 = 5125;
const FLOAT: u32 = 5126;

fn import_err(doc: &[u8]) -> ImportError {
    match import_slice(doc) {
        Ok(_) => panic!("malformed document imported"),
        Err(e) => e,
    }
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
            assert!(
                (c.abs() - 0.5).abs() < 1e-6,
                "position component {c} is ±0.5"
            );
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
        assert_eq!(
            [chunk[3], chunk[4], chunk[5]],
            mesh.normals[i],
            "normal {i}"
        );
        assert_eq!(
            [chunk[6], chunk[7], chunk[8]],
            DEFAULT_IMPORT_COLOR,
            "default color {i}"
        );
    }
}

#[test]
fn untextured_material_base_color_factor_becomes_vertex_color() {
    // an untextured primitive with an explicit material packs that
    // material's base-color factor (RGB) as its per-vertex color, so a
    // multi-material mesh (e.g. a body + dark wheels) renders each part's color
    // through the untextured `[pos,normal,color]` pipeline.
    let scene = import_slice(MATERIAL_COLOR_GLTF).expect("material-color fixture parses");
    let mesh = &scene.meshes[0];

    // Not textured — stays on the color layout.
    assert!(!mesh.is_textured());
    assert_eq!(mesh.layout, render_vertex_layout());
    // The material declared this factor.
    assert_eq!(mesh.base_color_factor, [0.2, 0.4, 0.6, 1.0]);

    // Every packed vertex carries the factor's RGB as its color, not the default.
    let floats: Vec<f32> = mesh
        .vertices
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    for (i, chunk) in floats.chunks_exact(9).enumerate() {
        assert_eq!(
            [chunk[6], chunk[7], chunk[8]],
            [0.2, 0.4, 0.6],
            "material color {i}"
        );
        assert_ne!(
            [chunk[6], chunk[7], chunk[8]],
            DEFAULT_IMPORT_COLOR,
            "not default {i}"
        );
    }
}

#[test]
fn uvs_are_retained_even_when_source_has_none() {
    // The cube fixture has no UV set; the importer still retains a UV per vertex
    // (defaulted), one per packed vertex, for the textured path to consume.
    let mesh = &cube_scene().meshes[0];
    assert_eq!(mesh.uvs.len(), mesh.vertex_count(), "one UV per vertex");
    assert!(mesh.uvs.iter().all(|uv| *uv == [0.0, 0.0]));
}

#[test]
fn mesh_data_round_trips_through_the_seam() {
    // The packed MeshAsset drops straight into MeshData, and the device draws it:
    // its layout is the vertex-colour pipeline's and it has whole triangles.
    use stickshift_math::Transform;
    use stickshift_render_api::{
        FrameRecorder, MaterialParams, PipelineDescriptor, PipelineKind, ProtocolViolation,
    };
    let scene = cube_scene();
    let mesh: &MeshAsset = &scene.meshes[0];
    assert_eq!(mesh.vertices.len() % mesh.layout.stride as usize, 0);
    let mut device = NullRenderer::new();
    let handle: MeshHandle = device.create_mesh(&MeshData {
        vertices: &mesh.vertices,
        indices: &mesh.indices,
        layout: mesh.layout.clone(),
    });
    let pipeline = device.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor));
    device.begin_frame();
    device.set_pipeline(pipeline);
    device.draw_mesh(handle, &Transform::identity(), &MaterialParams::default());
    device.submit();
    assert_eq!(device.draw_count(), 1, "the uploaded mesh is drawable");
    assert!(device.protocol_violations().is_empty());

    device.destroy_mesh(handle);
    assert_eq!(device.live_mesh_count(), 0);
    device.destroy_mesh(handle);
    assert_eq!(
        device.protocol_violations(),
        &[ProtocolViolation::UnknownMesh(handle)],
        "a second destroy is caught"
    );
}

#[test]
fn slice_and_path_imports_of_a_textured_gltf_agree() {
    // A `data:` image decodes from memory exactly as it does from a file.
    let from_memory = textured_cube_scene();
    let from_file = import_via_file(TEXTURED_CUBE_GLTF);
    let (a, b) = (&from_memory.meshes[0], &from_file.meshes[0]);
    let (ta, tb) = (
        a.base_color.as_ref().unwrap(),
        b.base_color.as_ref().unwrap(),
    );
    assert_eq!(
        (ta.width, ta.height, &ta.rgba8),
        (tb.width, tb.height, &tb.rgba8)
    );
    assert_eq!(a.vertices, b.vertices);
}

#[test]
fn a_slice_import_still_rejects_external_file_references() {
    let doc = br#"{"asset": {"version": "2.0"}, "images": [{"uri": "texture.png"}]}"#;
    assert!(matches!(
        import_err(doc),
        ImportError::Gltf(gltf::Error::ExternalReferenceInSliceImport)
    ));
}

#[test]
fn triangle_strips_expand_to_a_triangle_list() {
    let doc = primitive_glb(
        TRIANGLE_STRIP,
        &QUAD,
        Some((&u32_indices(&[0, 1, 2, 3]), U32, 4)),
    );
    let mesh = &import_slice(&doc).unwrap().meshes[0];
    // Two triangles, the second wound (1, 3, 2) as the spec defines.
    assert_eq!(corners(mesh), corners_of(&QUAD, &[0, 1, 2, 1, 3, 2]));
    assert_eq!(mesh.triangle_count(), 2);
}

#[test]
fn a_non_indexed_strip_expands_over_its_vertices_in_order() {
    let doc = primitive_glb(TRIANGLE_STRIP, &QUAD, None);
    let mesh = &import_slice(&doc).unwrap().meshes[0];
    assert_eq!(corners(mesh), corners_of(&QUAD, &[0, 1, 2, 1, 3, 2]));
}

#[test]
fn strip_stitching_triangles_are_dropped() {
    // Repeating index 2 stitches two strips; the zero-area triangles that
    // produces, (1, 2, 2) and (2, 2, 3), are dropped.
    let positions: Vec<f32> = (0..5)
        .flat_map(|i| [i as f32, (i % 2) as f32, 0.0])
        .collect();
    let doc = primitive_glb(
        TRIANGLE_STRIP,
        &positions,
        Some((&u32_indices(&[0, 1, 2, 2, 3, 4]), U32, 6)),
    );
    let mesh = &import_slice(&doc).unwrap().meshes[0];
    assert_eq!(corners(mesh), corners_of(&positions, &[0, 1, 2, 2, 4, 3]));
}

#[test]
fn triangle_fans_expand_around_the_first_vertex() {
    let doc = primitive_glb(
        TRIANGLE_FAN,
        &QUAD,
        Some((&u32_indices(&[0, 1, 3, 2]), U32, 4)),
    );
    let mesh = &import_slice(&doc).unwrap().meshes[0];
    assert_eq!(corners(mesh), corners_of(&QUAD, &[1, 3, 0, 3, 2, 0]));
}

#[test]
fn small_index_types_import() {
    let u8_doc = primitive_glb(TRIANGLES, &QUAD, Some((&[0, 1, 2], U8, 3)));
    assert_eq!(
        corners(&import_slice(&u8_doc).unwrap().meshes[0]),
        corners_of(&QUAD, &[0, 1, 2])
    );
    let u16_doc = primitive_glb(
        TRIANGLES,
        &QUAD,
        Some((&le_bytes(&[3u16, 2, 1], u16::to_le_bytes), U16, 3)),
    );
    assert_eq!(
        corners(&import_slice(&u16_doc).unwrap().meshes[0]),
        corners_of(&QUAD, &[3, 2, 1])
    );
}

#[test]
fn missing_normals_are_flat() {
    // glTF: a primitive without NORMAL must be flat-shaded. An XY quad faces
    // +Z when wound counter-clockwise and -Z when wound the other way; it must
    // never get the old constant +Y.
    let doc = primitive_glb(
        TRIANGLES,
        &QUAD,
        Some((&u32_indices(&[0, 1, 3, 0, 3, 2, 0, 2, 3]), U32, 9)),
    );
    let mesh = &import_slice(&doc).unwrap().meshes[0];
    assert_eq!(mesh.normals.len(), mesh.positions.len());
    let face = |t: usize| mesh.normals[mesh.indices[t * 3] as usize];
    for t in 0..2 {
        assert_eq!(face(t), [0.0, 0.0, 1.0], "triangle {t}");
    }
    assert_eq!(face(2), [0.0, 0.0, -1.0], "the reversed triangle faces -Z");
    // Every corner of a triangle carries that triangle's normal.
    for (t, tri) in mesh.indices.chunks_exact(3).enumerate() {
        for &i in tri {
            assert_eq!(mesh.normals[i as usize], face(t));
        }
    }
    // The packed vertices carry the same normals (offset 12 in either layout).
    let stride = mesh.layout.stride as usize;
    for (i, normal) in mesh.normals.iter().enumerate() {
        let at = |k: usize| {
            let o = i * stride + 12 + k * 4;
            f32::from_le_bytes(mesh.vertices[o..o + 4].try_into().unwrap())
        };
        assert_eq!([at(0), at(1), at(2)], *normal, "packed normal {i}");
    }
}

#[test]
fn points_and_lines_are_rejected() {
    let doc = primitive_glb(LINES, &QUAD, Some((&u32_indices(&[0, 1, 2, 3]), U32, 4)));
    assert!(matches!(
        import_err(&doc),
        ImportError::UnsupportedPrimitiveMode("LINES")
    ));
    let doc = primitive_glb(0, &QUAD, None);
    assert!(matches!(
        import_err(&doc),
        ImportError::UnsupportedPrimitiveMode("POINTS")
    ));
}

#[test]
fn malformed_primitives_are_errors_not_panics() {
    let invalid = |doc: Vec<u8>| {
        assert!(
            matches!(import_err(&doc), ImportError::InvalidPrimitive(_)),
            "expected InvalidPrimitive"
        );
    };
    // Float indices (the gltf reader hits `unreachable!`).
    invalid(primitive_glb(
        TRIANGLES,
        &QUAD,
        Some((&le_bytes(&[0.0f32, 1.0, 2.0], f32::to_le_bytes), FLOAT, 3)),
    ));
    // An index past the last vertex.
    invalid(primitive_glb(
        TRIANGLES,
        &QUAD,
        Some((&u32_indices(&[0, 1, 99]), U32, 3)),
    ));
    // A triangle list that is not a whole number of triangles.
    invalid(primitive_glb(
        TRIANGLES,
        &QUAD,
        Some((&u32_indices(&[0, 1, 2, 3]), U32, 4)),
    ));
    // A strip too short to make a triangle.
    invalid(primitive_glb(
        TRIANGLE_STRIP,
        &QUAD,
        Some((&u32_indices(&[0, 1]), U32, 2)),
    ));
}

#[test]
fn malformed_position_accessors_are_errors_not_panics() {
    let invalid = |accessor: (&str, usize), stride: Option<usize>| {
        let doc = primitive_glb_with(TRIANGLES, &QUAD[..9], None, accessor, stride);
        assert!(
            matches!(import_err(&doc), ImportError::InvalidPrimitive(_)),
            "{accessor:?} stride {stride:?}"
        );
    };
    // A count of zero (the reader's range arithmetic underflows).
    invalid(("VEC3", 0), None);
    // VEC2 positions (the reader assumes three floats).
    invalid(("VEC2", 3), None);
    // A stride shorter than one 12-byte position.
    invalid(("VEC3", 3), Some(8));
    // More positions than the buffer view holds.
    invalid(("VEC3", 4), None);
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
    // the committed game asset is now textured (base-color checkerboard).
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
    let tex = mesh
        .base_color
        .as_ref()
        .expect("decoded base-color texture");
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
        assert!(
            (u - uv[0]).abs() < 1e-6 && (v - uv[1]).abs() < 1e-6,
            "uv {i}"
        );
    }
    // The fixture's UVs span the full 0..1 range (not all default [0,0]).
    assert!(
        mesh.uvs.iter().any(|uv| *uv != [0.0, 0.0]),
        "real UVs parsed"
    );
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
    let dir = std::env::temp_dir().join(format!("stickshift-assets-cache-{}", std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("stickshift-assets-upload-{}", std::process::id()));
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

/// A glTF whose child node, rotated 45° about Z, sits under a parent scaled
/// `(2, 1, 1)`: the child's world matrix is sheared. Its mesh is one triangle
/// with a corner at `(1, 0, 0)`.
const SHEARED_HIERARCHY_GLTF: &[u8] = br#"{
    "asset": {"version": "2.0"},
    "buffers": [{"byteLength": 36,
        "uri": "data:application/octet-stream;base64,AACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAA"}],
    "bufferViews": [{"buffer": 0, "byteLength": 36}],
    "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
        "min": [0, 0, 0], "max": [1, 1, 0]}],
    "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
    "nodes": [
        {"scale": [2, 1, 1], "children": [1]},
        {"rotation": [0, 0, 0.3826834323650898, 0.9238795325112867], "mesh": 0}
    ],
    "scenes": [{"nodes": [0]}],
    "scene": 0
}"#;

#[test]
fn sheared_hierarchy_bakes_exactly() {
    // glTF composes parent × child as matrices. Under a non-uniform parent the
    // rotated child is sheared, which position/rotation/scale cannot hold: the
    // old TRS bake put (1, 0, 0) at (1.108, 1.061, 0) instead of here.
    let scene = import_slice(SHEARED_HIERARCHY_GLTF).unwrap();
    let (_, node) = scene.mesh_nodes().next().expect("one mesh node");
    let world = node.world.transform_point3(stickshift_math::glam::Vec3::X);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let expected = stickshift_math::glam::Vec3::new(2.0 * h, h, 0.0);
    assert!(
        (world - expected).length() < 1e-5,
        "(1, 0, 0) baked to {world}, expected {expected}"
    );
}

#[cfg(feature = "ecs")]
#[test]
fn spawning_a_sheared_hierarchy_keeps_its_vertices_exact() {
    // An ECS transform cannot hold shear either, so the spawn helper keeps only
    // the translation and bakes the rest into the vertices. Either way the
    // entity's vertices must land where the node matrix puts them.
    use stickshift_ecs::{RenderShape, TransformComponent};
    let scene = import_slice(SHEARED_HIERARCHY_GLTF).unwrap();
    let mut world = stickshift_ecs::hecs::World::new();
    let entities = stickshift_assets::spawn_scene(&mut world, &scene);
    assert_eq!(entities.len(), 1);
    let (_, node) = scene.mesh_nodes().next().unwrap();
    let transform = world
        .get::<&TransformComponent>(entities[0])
        .unwrap()
        .transform;
    let render = world
        .get::<&stickshift_ecs::RenderComponent>(entities[0])
        .unwrap();
    let RenderShape::Mesh { vertices, .. } = &render.shape else {
        panic!("an imported node spawns a mesh");
    };
    for (v, p) in vertices.iter().zip(&scene.meshes[0].positions) {
        let drawn = transform.transform_point(stickshift_math::glam::Vec3::new(v[0], v[1], v[2]));
        let expected = node
            .world
            .transform_point3(stickshift_math::glam::Vec3::from_array(*p));
        assert!((drawn - expected).length() < 1e-5, "{drawn} != {expected}");
    }
}
