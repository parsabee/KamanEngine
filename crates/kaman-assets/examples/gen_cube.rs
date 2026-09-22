// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.
//
//! One-shot generator for the committed `cube.gltf` test/runtime fixture.
//!
//! Run with `cargo run -p kaman-assets --example gen_cube -- <out_path>`. It emits
//! a self-contained glTF 2.0 file (embedded base64 buffer) for a unit cube with
//! per-face normals, so the repo carries no binary blob and the fixture is fully
//! reproducible. Kept as an example (not a build step) so normal builds never run
//! it.

use std::fmt::Write as _;

/// 24-vertex unit cube with per-face normals and 36 indices.
fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u16>) {
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
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for (n, verts) in faces {
        let base = positions.len() as u16;
        for v in verts {
            positions.push(v);
            normals.push(n);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, normals, indices)
}

/// Minimal standard base64 (no external crate) so the fixture generator has no
/// dependency surface.
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

    let (positions, normals, indices) = cube();

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

    let idx_len = indices.len() * 2;
    let pos_len = positions.len() * 12;
    let nrm_len = normals.len() * 12;
    let b64 = base64(&buf);

    let mut json = String::new();
    write!(
        json,
        r#"{{
  "asset": {{ "version": "2.0", "generator": "KamanEngine KE-0402 cube fixture" }},
  "scene": 0,
  "scenes": [{{ "name": "cube_scene", "nodes": [0] }}],
  "nodes": [{{ "name": "cube", "mesh": 0 }}],
  "meshes": [{{ "name": "cube", "primitives": [{{ "attributes": {{ "POSITION": 1, "NORMAL": 2 }}, "indices": 0, "mode": 4 }}] }}],
  "buffers": [{{ "byteLength": {buflen}, "uri": "data:application/octet-stream;base64,{b64}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": {idx_len}, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {nrm_off}, "byteLength": {nrm_len}, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5123, "count": {idx_count}, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": {vcount}, "type": "VEC3", "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5] }},
    {{ "bufferView": 2, "componentType": 5126, "count": {vcount}, "type": "VEC3" }}
  ]
}}
"#,
        buflen = buf.len(),
        idx_count = indices.len(),
        vcount = positions.len(),
    )
    .unwrap();

    std::fs::write(&out, json).expect("write cube.gltf");
    println!(
        "wrote {out}: {} verts, {} indices, {} buffer bytes",
        positions.len(),
        indices.len(),
        buf.len()
    );
}
