// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.
//
//! One-shot generator for a committed **textured** `cube.gltf` fixture (KE-0403).
//!
//! Run with `cargo run -p kaman-assets --example gen_textured_cube -- <out_path>`.
//! It emits a self-contained glTF 2.0 file: a unit cube with per-face normals and
//! UVs, plus a material whose base-color texture is a small embedded checkerboard
//! **PNG** (a `data:` URI, so the repo carries no separate binary blob). The
//! importer decodes the PNG to RGBA8 and packs the mesh on the `[pos,normal,uv]`
//! textured layout so the backend samples it. Kept as an example (not a build
//! step) so normal builds never run it.

use std::fmt::Write as _;

use image::{ImageEncoder, ImageFormat};

/// 24-vertex unit cube with per-face normals, UVs, and 36 indices.
#[allow(clippy::type_complexity)]
fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u16>) {
    // Each face: normal + 4 CCW corners. A full 0..1 UV quad per face so the
    // checkerboard tiles across every face.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [0.5, -0.5, -0.5],
                [-0.5, -0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [-0.5, 0.5, 0.5],
                [0.5, 0.5, 0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, -0.5, 0.5],
                [-0.5, -0.5, 0.5],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [
                [0.5, -0.5, 0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [0.5, 0.5, 0.5],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-0.5, -0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [-0.5, 0.5, 0.5],
                [-0.5, 0.5, -0.5],
            ],
        ),
    ];
    let uv_quad = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (n, verts) in faces {
        let base = positions.len() as u16;
        for (i, v) in verts.into_iter().enumerate() {
            positions.push(v);
            normals.push(n);
            uvs.push(uv_quad[i]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, normals, uvs, indices)
}

/// An 8x8 RGBA checkerboard encoded as PNG bytes (two contrasting colors), so the
/// textured cube renders a non-uniform, obviously-textured surface.
fn checkerboard_png() -> Vec<u8> {
    const N: u32 = 8;
    let a = [230u8, 60, 40, 255]; // warm red
    let b = [40u8, 90, 220, 255]; // cool blue
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let c = if (x + y) % 2 == 0 { a } else { b };
            rgba.extend_from_slice(&c);
        }
    }
    let mut png: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, N, N, image::ExtendedColorType::Rgba8)
        .expect("encode checkerboard png");
    debug_assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
    png
}

/// Minimal standard base64 (no external crate) so the fixture generator has no
/// extra dependency surface for the buffer/image URIs.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "games/car-runner/assets/cube.gltf".to_string());

    let (positions, normals, uvs, indices) = cube();

    // Interleave-free buffer: indices, then positions, then normals, then UVs.
    let mut buf: Vec<u8> = Vec::new();
    for &i in &indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
    let pos_off = buf.len();
    for p in &positions {
        for f in p {
            buf.extend_from_slice(&f.to_le_bytes());
        }
    }
    let nrm_off = buf.len();
    for n in &normals {
        for f in n {
            buf.extend_from_slice(&f.to_le_bytes());
        }
    }
    let uv_off = buf.len();
    for uv in &uvs {
        for f in uv {
            buf.extend_from_slice(&f.to_le_bytes());
        }
    }

    let idx_len = indices.len() * 2;
    let pos_len = positions.len() * 12;
    let nrm_len = normals.len() * 12;
    let uv_len = uvs.len() * 8;
    let b64 = base64(&buf);

    let png = checkerboard_png();
    let png_b64 = base64(&png);

    let mut json = String::new();
    write!(
        json,
        r#"{{
  "asset": {{ "version": "2.0", "generator": "KamanEngine KE-0403 textured cube fixture" }},
  "scene": 0,
  "scenes": [{{ "name": "cube_scene", "nodes": [0] }}],
  "nodes": [{{ "name": "cube", "mesh": 0 }}],
  "meshes": [{{ "name": "cube", "primitives": [{{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0, "material": 0, "mode": 4 }}] }}],
  "materials": [{{
    "name": "checker",
    "pbrMetallicRoughness": {{
      "baseColorTexture": {{ "index": 0 }},
      "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
      "metallicFactor": 0.0,
      "roughnessFactor": 1.0
    }}
  }}],
  "textures": [{{ "source": 0, "sampler": 0 }}],
  "samplers": [{{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }}],
  "images": [{{ "mimeType": "image/png", "uri": "data:image/png;base64,{png_b64}" }}],
  "buffers": [{{ "byteLength": {buflen}, "uri": "data:application/octet-stream;base64,{b64}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {idx_len}, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {nrm_off}, "byteLength": {nrm_len}, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uv_off}, "byteLength": {uv_len}, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5123, "count": {idx_count}, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": {vcount}, "type": "VEC3", "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5] }},
    {{ "bufferView": 2, "componentType": 5126, "count": {vcount}, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": {vcount}, "type": "VEC2" }}
  ]
}}
"#,
        buflen = buf.len(),
        idx_count = indices.len(),
        vcount = positions.len(),
    )
    .unwrap();

    std::fs::write(&out, json).expect("write textured cube.gltf");
    println!(
        "wrote {out}: {} verts, {} indices, {} buffer bytes, {}-byte png texture",
        positions.len(),
        indices.len(),
        buf.len(),
        png.len(),
    );
}
