// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! One-shot generator for the committed **asphalt road** asset the playable-demo
//! imports (KE-0704).
//!
//! The asphalt is a real, free **CC0** photographic texture — Poly Haven
//! `asphalt_02` diffuse (<https://polyhaven.com/a/asphalt_02>, CC0, no attribution
//! required) — committed as `games/playable-demo/assets/asphalt_src.jpg`. This
//! generator loads that photo, tiles it to the road's aspect, composites our own
//! **dashed white lane lines** at the interior lane boundaries, and embeds the
//! result into a self-contained glTF 2.0 file (`assets/road.gltf`) whose mesh is a
//! single **flat road-tile quad** carrying the tiling UVs. The road glTF is loaded
//! through `kaman-assets` at runtime; the importer decodes the embedded PNG to
//! RGBA8 and packs the quad on the `[pos,normal,uv]` textured layout so the
//! backend's textured pipeline samples it (with the free mip chain + trilinear
//! repeat sampler from KE-0403). The deliverable is the committed asset + the
//! import path, not this tool; normal builds never run it (it is an example, and
//! `image` is a dev-dependency only).
//!
//! # Geometry & UVs
//!
//! The quad spans a **unit tile** in X/Z (`[-0.5, 0.5]` on both), lying in a
//! horizontal plane at local `y = +0.5`. The road entity's transform (set
//! game-side, unchanged by KE-0704) is `position.y = -0.5`, `scale = (12, 0.4,
//! spawn_interval)`, so after scaling the quad becomes a `12 × spawn_interval`
//! road surface whose top sits at world `y = -0.3` — exactly where the former
//! black road box's top face was, keeping the cars resting on the surface and
//! collision (a fixed game-side AABB) untouched.
//!
//! UVs are baked so tiles seam invisibly under streaming:
//! - **U = 0..1** across the full road width (the texture spans the road once), so
//!   the two dashed lane lines land at the interior lane boundaries
//!   (`x = ±1.5` ⇒ `U = 0.375 / 0.625`).
//! - **V = 0..3** along the tile's `spawn_interval` (6-unit) length — three integer
//!   repeats — so every tile-to-tile seam falls on a texture-wrap boundary and, the
//!   asphalt being seamless in V, is invisible.
//!
//! The baked texture is **6:1** (wide) so the photographic asphalt keeps a natural,
//! isotropic scale: `U = 0..1` covers the 12-unit width while each `V` repeat
//! covers 2 units, and the seamless source is tiled `H_TILES` times across the
//! width so each copy is ~2 world units — no horizontal smear.
//!
//! Run:
//! ```sh
//! cargo run -p playable-demo --example gen_asphalt
//! ```
//! (writes `games/playable-demo/assets/road.gltf`), or pass an output directory:
//! `cargo run -p playable-demo --example gen_asphalt -- <out_dir>`.

use std::fmt::Write as _;
use std::path::Path;

use image::{ImageEncoder, ImageFormat};

/// Baked texture width in pixels (6× the height so the asphalt scale is isotropic:
/// 12 world units across the width vs 2 per V repeat).
const TEX_W: u32 = 1536;
/// Baked texture height in pixels (one V repeat ⇒ 2 world units of road length).
const TEX_H: u32 = 256;
/// How many times the seamless source photo is tiled across the width; each copy
/// is `TEX_W / H_TILES` px ≈ 2 world units of asphalt.
const H_TILES: u32 = 6;

/// How many integer texture repeats run along the tile's length (the V axis). The
/// road quad bakes `V = 0..V_REPEATS` so tile-to-tile seams land on wrap
/// boundaries. Kept in sync with the demo's `spawn_interval` (6.0): three repeats
/// over six units ⇒ a 2-unit texture period, a natural asphalt scale.
const V_REPEATS: f32 = 3.0;

/// Bake the asphalt base-color texture as PNG bytes: the committed CC0 asphalt
/// photo tiled to the road aspect, with our **dashed white lane lines** composited
/// at the interior lane boundaries (`U ≈ 0.375` and `0.625`). Dashes run along the
/// road (the V/length axis) so they read as painted lane markings.
///
/// The asphalt is *not* generated — it is the real photographic texture; only the
/// lane lines are added here.
fn asphalt_png(src_path: &Path) -> Vec<u8> {
    // Load the committed CC0 asphalt photo and downscale one seamless tile to
    // TEX_H × TEX_H, so tiling it `H_TILES`× across fills the baked width exactly.
    let tile_px = TEX_W / H_TILES; // = TEX_H when TEX_W == H_TILES * TEX_H
    let src = image::open(src_path)
        .unwrap_or_else(|e| panic!("open {}: {e}", src_path.display()))
        .to_rgba8();
    let tile = image::imageops::resize(
        &src,
        tile_px,
        TEX_H,
        image::imageops::FilterType::Lanczos3,
    );

    // Lane markings as U columns. Interior lane dividers (U = 0.375, 0.625) are
    // dashed; the two outer edges of the drivable lanes (x = ±4.5 on the 12-unit
    // road ⇒ U = 0.125, 0.875) are solid. Line width + dash cadence as texture
    // fractions.
    let dashed_u = [0.375f32, 0.625f32];
    let solid_u = [0.125f32, 0.875f32];
    let line_half = 0.006f32; // ~0.14 world units wide ⇒ a realistic painted stripe
    // Period divides 1.0 evenly (1 per texture height) so dashes tile seamlessly
    // across the V wrap boundary; a large period spaces the dashes out, and the
    // painted fraction keeps each dash's length (≈ the prior 1/2 × 0.72 dash).
    let dash_len = 1.0f32; // dash + gap period along V
    let dash_on = 0.36f32; // fraction of the period that is painted (rest is gap)

    let mut rgba: Vec<u8> = Vec::with_capacity((TEX_W * TEX_H * 4) as usize);
    for y in 0..TEX_H {
        for x in 0..TEX_W {
            let u = x as f32 / TEX_W as f32;
            let v = y as f32 / TEX_H as f32;

            // Asphalt: the seamless photo tiled across the width.
            let px = tile.get_pixel(x % tile_px, y);
            let (mut r, mut g, mut b) = (px[0], px[1], px[2]);

            // Slightly worn white so lines aren't a flat, fake pure white.
            let paint = |r: &mut u8, g: &mut u8, b: &mut u8| {
                *r = 218;
                *g = 218;
                *b = 208;
            };

            // Solid outer edge lines: painted continuously.
            for &su in &solid_u {
                if (u - su).abs() < line_half {
                    paint(&mut r, &mut g, &mut b);
                }
            }

            // Dashed interior lane dividers: painted only on the dash phase.
            for &lu in &dashed_u {
                if (u - lu).abs() < line_half {
                    let phase = (v / dash_len).fract();
                    if phase < dash_on {
                        paint(&mut r, &mut g, &mut b);
                    }
                }
            }

            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }

    let mut png: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, TEX_W, TEX_H, image::ExtendedColorType::Rgba8)
        .expect("encode asphalt png");
    debug_assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
    png
}

/// The flat road-tile quad: 4 vertices (unit tile in X/Z at local `y = +0.5`),
/// `+Y` normals, tiling UVs, 6 indices (two triangles, CCW when viewed from
/// above).
#[allow(clippy::type_complexity)]
fn road_quad() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u16>) {
    // Corners in the X/Z plane. U runs across X (full width once), V runs across Z
    // (V_REPEATS integer repeats) so streaming seams land on wrap boundaries.
    let positions = vec![
        [-0.5, 0.5, -0.5],
        [0.5, 0.5, -0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let normals = vec![[0.0, 1.0, 0.0]; 4];
    let uvs = vec![
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, V_REPEATS],
        [0.0, V_REPEATS],
    ];
    // CCW from above (+Y) so the surface faces up.
    let indices = vec![0u16, 2, 1, 0, 3, 2];
    (positions, normals, uvs, indices)
}

/// Minimal standard base64 (no external crate) so the fixture generator has no
/// extra dependency surface for the buffer/image URIs — mirrors the cube/car gens.
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
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "games/playable-demo/assets".to_string());
    let dir = Path::new(&out_dir);
    let out = dir.join("road.gltf");
    let src = dir.join("asphalt_src.jpg");

    let (positions, normals, uvs, indices) = road_quad();

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

    let png = asphalt_png(&src);
    let png_b64 = base64(&png);

    // Position accessor bounds (the quad's model-space extents).
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in &positions {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }

    let mut json = String::new();
    write!(
        json,
        r#"{{
  "asset": {{ "version": "2.0", "generator": "KamanEngine KE-0704 asphalt road generator" }},
  "scene": 0,
  "scenes": [{{ "name": "road_scene", "nodes": [0] }}],
  "nodes": [{{ "name": "road", "mesh": 0 }}],
  "meshes": [{{ "name": "road", "primitives": [{{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0, "material": 0, "mode": 4 }}] }}],
  "materials": [{{
    "name": "asphalt",
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
    {{ "bufferView": 1, "componentType": 5126, "count": {vcount}, "type": "VEC3", "min": [{min0}, {min1}, {min2}], "max": [{max0}, {max1}, {max2}] }},
    {{ "bufferView": 2, "componentType": 5126, "count": {vcount}, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": {vcount}, "type": "VEC2" }}
  ]
}}
"#,
        buflen = buf.len(),
        idx_count = indices.len(),
        vcount = positions.len(),
        min0 = min[0], min1 = min[1], min2 = min[2],
        max0 = max[0], max1 = max[1], max2 = max[2],
    )
    .unwrap();

    std::fs::write(&out, json)
        .unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
    println!(
        "wrote {}: {} verts, {} indices, {} buffer bytes, {}-byte {}x{} asphalt png (source {})",
        out.display(),
        positions.len(),
        indices.len(),
        buf.len(),
        png.len(),
        TEX_W,
        TEX_H,
        src.display(),
    );
}
