// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! One-shot generator for the committed **distant city skyline** backdrop asset
//! (KE-0705).
//!
//! The skyline is a real, free **CC0** photo — a New York City skyline seen across
//! the Hudson from Union City, NJ (Wikimedia Commons, CC0, public domain) —
//! committed as `games/playable-demo/assets/skyline_src.jpg`. This generator crops
//! the **skyline band** out of that photo (dropping the foreground trees, road and
//! water), downscales it, and bakes it into a self-contained glTF 2.0 file
//! (`assets/skyline.gltf`) whose mesh is a single **vertical billboard quad**. The
//! demo imports it through `kaman-assets`, uploads the texture once, and draws it
//! as a far billboard locked to the camera's XZ so it reads as a distant skyline
//! (KE-0705). The photo's tops already dissolve into haze, which blends into the
//! engine's distance fog + gradient sky.
//!
//! The deliverable is the committed asset + the import path, not this tool; normal
//! builds never run it (it is an example, and `image` is a dev-dependency only).
//!
//! Run:
//! ```sh
//! cargo run -p playable-demo --example gen_skyline
//! ```
//! (writes `games/playable-demo/assets/skyline.gltf`), or pass an output
//! directory: `cargo run -p playable-demo --example gen_skyline -- <out_dir>`.

use std::fmt::Write as _;
use std::path::Path;

use image::{ImageEncoder, ImageFormat};

/// Crop rectangle (in the source photo's pixels, 2560×1920) of the skyline band:
/// buildings + their hazy tops, down to the waterfront, with the foreground trees
/// on the far left excluded.
const CROP_X: u32 = 210;
const CROP_Y: u32 = 700;
const CROP_W: u32 = 2350;
const CROP_H: u32 = 350;

/// Baked billboard texture width in pixels (height follows the crop aspect). A
/// distant, fogged backdrop needs no more than this.
const TEX_W: u32 = 1536;

/// Crop the skyline band out of the source photo and encode it as PNG bytes.
fn skyline_png(src_path: &Path) -> (Vec<u8>, u32, u32) {
    let src = image::open(src_path)
        .unwrap_or_else(|e| panic!("open {}: {e}", src_path.display()))
        .to_rgba8();

    // Clamp the crop to the actual image bounds so a differently-sized source
    // can't panic the generator.
    let (iw, ih) = (src.width(), src.height());
    let x = CROP_X.min(iw.saturating_sub(1));
    let y = CROP_Y.min(ih.saturating_sub(1));
    let w = CROP_W.min(iw - x);
    let h = CROP_H.min(ih - y);

    let cropped = image::imageops::crop_imm(&src, x, y, w, h).to_image();

    // Downscale to TEX_W, keeping the crop's aspect ratio.
    let tex_h = ((TEX_W as u64 * h as u64) / w as u64) as u32;
    let mut scaled = image::imageops::resize(
        &cropped,
        TEX_W,
        tex_h,
        image::imageops::FilterType::Lanczos3,
    );

    // The source is a low-contrast, foggy-day photo, and the engine's distance fog
    // washes it further. Boost contrast (push each channel away from mid-grey) and
    // darken a touch so the buildings read as a distinct skyline rather than a flat
    // grey band.
    const CONTRAST: f32 = 1.9;
    const BRIGHTNESS: f32 = -0.07;
    for px in scaled.pixels_mut() {
        for c in 0..3 {
            let v = px[c] as f32 / 255.0;
            let nv = ((v - 0.5) * CONTRAST + 0.5 + BRIGHTNESS).clamp(0.0, 1.0);
            px[c] = (nv * 255.0) as u8;
        }
    }

    let mut png: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(scaled.as_raw(), TEX_W, tex_h, image::ExtendedColorType::Rgba8)
        .expect("encode skyline png");
    debug_assert_eq!(image::guess_format(&png).unwrap(), ImageFormat::Png);
    (png, TEX_W, tex_h)
}

/// How far to slide the skyline picture within the (fixed) billboard, as a fraction
/// of the texture height. **Positive shifts the image down, negative shifts it up.**
/// The billboard frame is deliberately hung low (so it covers the mid-ground and no
/// sky seeps between the skyline and the terrain); this negative shift lifts the
/// picture back up inside that frame so the distant buildings sit at the right
/// height. Sampling past either edge clamps (CLAMP_TO_EDGE): the bottom of the
/// frame becomes the waterfront row, which reads as haze behind the terrain.
const V_SHIFT: f32 = -0.05;

/// Number of vertical strips the curved backdrop is built from (more = smoother
/// curve).
const SEGMENTS: u32 = 24;

/// How far (in world units) the backdrop's left/right edges bow **toward the
/// camera** relative to its flat center, so it curves around the road for a
/// panoramic illusion. The demo draws the mesh with `scale.z = 1`, so this is a
/// world-space depth; the center strip stays at the nominal backdrop distance and
/// the edges come forward by up to this much.
const BEND_DEPTH: f32 = 56.0;

/// The curved skyline backdrop mesh: a shallow horizontal arc (a unit quad in
/// X/Y, `x ∈ [-0.5, 0.5]`, `y ∈ [-0.5, 0.5]`, scaled by the demo) whose strips bow
/// **forward** (`+Z`, toward the trailing camera) quadratically toward the edges by
/// [`BEND_DEPTH`], so the skyline wraps gently around the road instead of reading as
/// a flat wall. UVs map the skyline upright across the width, shifted down by
/// [`V_SHIFT`]. Faces `+Z` (and the material is double-sided).
#[allow(clippy::type_complexity)]
fn billboard_arc() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u16>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    let (top_v, bot_v) = (-V_SHIFT, 1.0 - V_SHIFT);

    // One vertical strip (top + bottom vertex) per column.
    for i in 0..=SEGMENTS {
        let u = i as f32 / SEGMENTS as f32;
        let x = u - 0.5; // -0.5 .. 0.5 across the width
        let t = 2.0 * u - 1.0; // -1 at left edge, 0 center, +1 right edge
        let z = BEND_DEPTH * t * t; // forward bow, 0 at center, BEND_DEPTH at edges

        positions.push([x, 0.5, z]); // top
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([u, top_v]);

        positions.push([x, -0.5, z]); // bottom
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([u, bot_v]);
    }

    // Two triangles per segment between adjacent strips.
    for i in 0..SEGMENTS {
        let tl = (2 * i) as u16;
        let bl = (2 * i + 1) as u16;
        let tr = (2 * (i + 1)) as u16;
        let br = (2 * (i + 1) + 1) as u16;
        indices.extend_from_slice(&[tl, br, tr, tl, bl, br]);
    }

    (positions, normals, uvs, indices)
}

/// Minimal standard base64 (no external crate) for the buffer/image URIs — mirrors
/// the other asset generators.
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
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "games/playable-demo/assets".to_string());
    let dir = Path::new(&out_dir);
    let out = dir.join("skyline.gltf");
    let src = dir.join("skyline_src.jpg");

    let (positions, normals, uvs, indices) = billboard_arc();

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

    let (png, tw, th) = skyline_png(&src);
    let png_b64 = base64(&png);

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
  "asset": {{ "version": "2.0", "generator": "KamanEngine KE-0705 skyline backdrop generator" }},
  "scene": 0,
  "scenes": [{{ "name": "skyline_scene", "nodes": [0] }}],
  "nodes": [{{ "name": "skyline", "mesh": 0 }}],
  "meshes": [{{ "name": "skyline", "primitives": [{{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0, "material": 0, "mode": 4 }}] }}],
  "materials": [{{
    "name": "skyline",
    "doubleSided": true,
    "pbrMetallicRoughness": {{
      "baseColorTexture": {{ "index": 0 }},
      "baseColorFactor": [1.0, 1.0, 1.0, 1.0],
      "metallicFactor": 0.0,
      "roughnessFactor": 1.0
    }}
  }}],
  "textures": [{{ "source": 0, "sampler": 0 }}],
  "samplers": [{{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }}],
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

    std::fs::write(&out, json).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
    println!(
        "wrote {}: billboard quad, {}-byte {}x{} skyline png (source {})",
        out.display(),
        png.len(),
        tw,
        th,
        src.display(),
    );
}
