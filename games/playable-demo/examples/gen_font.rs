// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! One-shot generator for the committed **SDF font atlas** the demo's HUD draws
//! with (KE-0404).
//!
//! The source face is a real, permissively-licensed font — Roboto (SIL Open Font
//! License, committed alongside as `assets/font.ttf` + `assets/font-OFL.txt`).
//! This bakes the printable ASCII range into a **signed-distance-field** atlas so
//! glyphs stay crisp at any size, and writes one self-contained binary the runtime
//! can load with no image decoder and no font parser:
//!
//! ```text
//! assets/font.bin
//!   magic      "KFNT" (4 bytes)
//!   version    u32
//!   width      u32          atlas width in pixels
//!   height     u32          atlas height in pixels
//!   first_char u32          codepoint of glyphs[0]
//!   count      u32          number of glyph records
//!   line_height f32         baseline-to-baseline, in em units
//!   glyphs     count * 9 * f32   (u0,v0,u1,v1, w,h, bx,by, advance)
//!   pixels     width*height u8   single-channel signed distance
//! ```
//!
//! All glyph metrics are in **em units** (normalised by the raster size), so the
//! HUD scales one atlas to any pixel size — the point of an SDF font.
//!
//! The deliverable is the committed `font.bin`; this tool is an example, so normal
//! builds never run it and `fontdue`/`image` stay dev-dependencies.
//!
//! Run:
//! ```sh
//! cargo run -p playable-demo --example gen_font
//! ```

use std::path::Path;

/// Pixel size each glyph is rasterised at before the distance transform. Larger
/// gives a more accurate field; the atlas is still scaled freely at draw time.
const RASTER_PX: f32 = 48.0;
/// Padding around each glyph, in pixels — the distance field needs room to fall
/// off outside the glyph's ink or edges clip.
const PAD: usize = 6;
/// Distance, in pixels, that maps to the full 0..255 encoded range. Larger spreads
/// the gradient wider (softer, more room for effects); smaller keeps it tight.
const SPREAD: f32 = 6.0;
/// First and last codepoints baked (printable ASCII).
const FIRST_CHAR: char = ' ';
const LAST_CHAR: char = '~';

/// A rasterised glyph awaiting packing.
struct Raster {
    /// Single-channel coverage bitmap, `w * h`.
    cov: Vec<u8>,
    w: usize,
    h: usize,
    /// Metrics in em units.
    advance: f32,
    bearing: [f32; 2],
}

/// Signed distance (in pixels) from each texel to the glyph's edge, encoded to
/// 0..255 with 128 at the edge — the convention the overlay shader's SDF mode
/// expects (it compares against 0.5 after normalising).
///
/// Uses a brute-force nearest-opposite-texel search. That is `O(n²)` per glyph,
/// but glyphs are small and this runs once, offline — clarity beats cleverness.
fn signed_distance_field(cov: &[u8], w: usize, h: usize) -> Vec<u8> {
    let inside = |x: usize, y: usize| cov[y * w + x] >= 128;
    let mut out = vec![0u8; w * h];

    for y in 0..h {
        for x in 0..w {
            // Edge texels: the rasteriser's partial coverage already encodes where
            // the outline crosses this texel, which is far finer than a
            // texel-granular search. Coverage 0.5 means the edge runs through the
            // texel centre, so `coverage - 0.5` is the signed distance in texels.
            // Without this the field only takes whole-texel steps and never lands
            // near its edge value.
            let raw = cov[y * w + x];
            if raw > 0 && raw < 255 {
                let signed = (raw as f32 / 255.0) - 0.5;
                let norm = (signed / SPREAD).clamp(-1.0, 1.0);
                out[y * w + x] = (((norm + 1.0) * 0.5) * 255.0).round() as u8;
                continue;
            }

            let is_in = inside(x, y);
            // Nearest texel of the opposite state gives the unsigned distance.
            let mut best = f32::INFINITY;
            // Search outward in rings so we can stop as soon as the ring's own
            // lower bound exceeds the best distance found.
            let max_r = (w.max(h)) as i32;
            for r in 1..=max_r {
                if (r as f32 - 1.0) > best {
                    break;
                }
                let mut ring_hit = false;
                for dy in -r..=r {
                    for dx in -r..=r {
                        // Only the ring's boundary, not its interior.
                        if dx.abs() != r && dy.abs() != r {
                            continue;
                        }
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        if inside(nx as usize, ny as usize) != is_in {
                            let d = ((dx * dx + dy * dy) as f32).sqrt();
                            if d < best {
                                best = d;
                                ring_hit = true;
                            }
                        }
                    }
                }
                let _ = ring_hit;
            }
            if !best.is_finite() {
                // Uniform glyph (all in or all out): saturate accordingly.
                best = SPREAD;
            }

            // Half-texel correction: the nearest opposite texel is at least 1 away,
            // but the actual edge lies about halfway between the two texel centers.
            // Without this the field never reaches its edge value and glyphs render
            // half a texel too fat.
            best = (best - 0.5).max(0.0);

            // Signed: positive inside, negative outside; encode 128 at the edge.
            let signed = if is_in { best } else { -best };
            let norm = (signed / SPREAD).clamp(-1.0, 1.0);
            out[y * w + x] = (((norm + 1.0) * 0.5) * 255.0).round() as u8;
        }
    }
    out
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "games/playable-demo/assets".to_string());
    let dir = Path::new(&out_dir);
    let ttf_path = dir.join("font.ttf");
    let out_path = dir.join("font.bin");

    let ttf = std::fs::read(&ttf_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", ttf_path.display()));
    let font = fontdue::Font::from_bytes(ttf.as_slice(), fontdue::FontSettings::default())
        .expect("parse font");

    // 1. Rasterise every glyph in range and convert it to a padded SDF tile.
    let mut rasters: Vec<Raster> = Vec::new();
    for code in (FIRST_CHAR as u32)..=(LAST_CHAR as u32) {
        let c = char::from_u32(code).expect("ascii range is valid");
        let (metrics, coverage) = font.rasterize(c, RASTER_PX);

        // Pad the coverage bitmap so the distance field has room to fall off.
        let (gw, gh) = (metrics.width, metrics.height);
        let (w, h) = (gw + PAD * 2, gh + PAD * 2);
        let mut padded = vec![0u8; w * h];
        for y in 0..gh {
            for x in 0..gw {
                padded[(y + PAD) * w + (x + PAD)] = coverage[y * gw + x];
            }
        }

        let cov = if gw == 0 || gh == 0 {
            // Whitespace: no ink, but it still advances the pen.
            Vec::new()
        } else {
            signed_distance_field(&padded, w, h)
        };

        rasters.push(Raster {
            w: if cov.is_empty() { 0 } else { w },
            h: if cov.is_empty() { 0 } else { h },
            cov,
            advance: metrics.advance_width / RASTER_PX,
            // fontdue's ymin is the distance from the baseline to the glyph's
            // bottom; the quad's top is that plus its height, negated because
            // overlay Y grows downward.
            bearing: [
                (metrics.xmin as f32 - PAD as f32) / RASTER_PX,
                -((metrics.ymin + gh as i32) as f32 + PAD as f32) / RASTER_PX,
            ],
        });
    }

    // 2. Pack the tiles into a square-ish atlas with a simple shelf packer.
    let atlas_w: usize = 512;
    let mut shelf_x = 0usize;
    let mut shelf_y = 0usize;
    let mut shelf_h = 0usize;
    let mut placements: Vec<(usize, usize)> = Vec::with_capacity(rasters.len());
    for r in &rasters {
        if r.w == 0 {
            placements.push((0, 0));
            continue;
        }
        if shelf_x + r.w > atlas_w {
            shelf_x = 0;
            shelf_y += shelf_h;
            shelf_h = 0;
        }
        placements.push((shelf_x, shelf_y));
        shelf_x += r.w;
        shelf_h = shelf_h.max(r.h);
    }
    let atlas_h = (shelf_y + shelf_h).next_power_of_two().max(64);

    let mut pixels = vec![0u8; atlas_w * atlas_h];
    for (r, &(px, py)) in rasters.iter().zip(&placements) {
        for y in 0..r.h {
            for x in 0..r.w {
                let (dx, dy) = (px + x, py + y);
                if dx < atlas_w && dy < atlas_h {
                    pixels[dy * atlas_w + dx] = r.cov[y * r.w + x];
                }
            }
        }
    }

    // 3. Serialise metrics + pixels into the self-contained binary.
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"KFNT");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(atlas_w as u32).to_le_bytes());
    out.extend_from_slice(&(atlas_h as u32).to_le_bytes());
    out.extend_from_slice(&(FIRST_CHAR as u32).to_le_bytes());
    out.extend_from_slice(&(rasters.len() as u32).to_le_bytes());
    // Roboto's default line spacing, in em units.
    out.extend_from_slice(&1.2f32.to_le_bytes());

    for (r, &(px, py)) in rasters.iter().zip(&placements) {
        let (u0, v0, u1, v1) = if r.w == 0 {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (
                px as f32 / atlas_w as f32,
                py as f32 / atlas_h as f32,
                (px + r.w) as f32 / atlas_w as f32,
                (py + r.h) as f32 / atlas_h as f32,
            )
        };
        let size = [r.w as f32 / RASTER_PX, r.h as f32 / RASTER_PX];
        for f in [
            u0,
            v0,
            u1,
            v1,
            size[0],
            size[1],
            r.bearing[0],
            r.bearing[1],
            r.advance,
        ] {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    out.extend_from_slice(&pixels);

    std::fs::write(&out_path, &out)
        .unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));
    println!(
        "wrote {}: {}x{} atlas, {} glyphs ('{}'..='{}'), {} bytes",
        out_path.display(),
        atlas_w,
        atlas_h,
        rasters.len(),
        FIRST_CHAR,
        LAST_CHAR,
        out.len(),
    );
}
