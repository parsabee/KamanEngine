// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Render pixel-hash guard (KR1.3).
//!
//! Deterministically renders fixed reference content into an offscreen Metal
//! texture, reads the pixels back, hashes it, and asserts the hash equals a
//! committed baseline. This is the golden net that pins the migrated renderer's
//! output so the buffer/frames-in-flight refactors (KE-0103/0104/0105) can prove
//! they preserved behavior.
//!
//! Two independent references are pinned, each with its own baseline constant:
//!
//! 1. **The 3D reference scene** ([`REFERENCE_HASH`]) — a lit, rotated box through
//!    the Phong pipeline and the KE-0401 look stack.
//! 2. **The 2D reference overlay** ([`OVERLAY_REFERENCE_HASH`], KE-0404) — the HUD
//!    overlay pass: a solid quad, a textured quad, an SDF quad, and a pair of
//!    overlapping partial-alpha quads whose record order the blend must respect.
//!
//! They are separate renders (separate offscreen backends) so a change to one
//! cannot shift the other's pixels, and a failure names which pass regressed.
//!
//! # Blessing the baselines
//!
//! The committed hashes below were blessed from this migrated renderer's first
//! correct frame of each pass. To re-bless after an *intended* visual change, run:
//!
//! ```text
//! BLESS=1 cargo test -p kaman-render --test pixel_hash -- --nocapture
//! ```
//!
//! Each test prints its new hash, named after the constant that holds it, and
//! passes (without asserting) so you can copy the value into that constant
//! **with a justification note in the commit message**. Blessing is deliberately
//! manual and loud.
//!
//! # CI safety (GPU-less runners)
//!
//! GitHub macOS runners are headless with no GPU, so `MTLCreateSystemDefaultDevice`
//! can return nil. When no Metal device is available the tests **skip** (print a
//! skip line and return) instead of failing, so they never break a GPU-less
//! build. On a real Mac (this dev machine) they run and assert.

use kaman_camera::Camera;
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render::MetalRenderer;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, OverlayFill, OverlayQuad, RenderDevice, TextureData,
    TextureHandle, VertexAttribute, VertexFormat, VertexLayout,
};

/// Offscreen render size for the reference scene.
const WIDTH: u32 = 64;
/// Offscreen render size for the reference scene.
const HEIGHT: u32 = 64;

/// Committed baseline hash of the reference scene's pixels (FNV-1a 64-bit).
///
/// Blessed from the migrated `kaman-render` first correct frame. Hold stable
/// across KE-0103/0104/0105; re-bless only via the documented `BLESS=1` path.
///
/// # KE-0205 re-bless review (value unchanged)
///
/// The camera **path** changed: the view now flows through the render seam
/// (`FrameRecorder::set_view_projection`) from a `kaman_camera::Camera` this test
/// builds, instead of a backend-owned inline camera. The camera **math** did not:
/// `Camera::new(WIDTH/HEIGHT)` (aspect `64/64 = 1.0`, the exact aspect the old
/// backend applied for this offscreen size) with the same
/// `set_position((0,0,3))` / `set_target((0,0,0))` and identical defaults (45°
/// FOV, `0.1..100.0` clip, `+Y` up) yields the **identical** view-projection
/// matrix. The geometry (`reference_cube`) and its transform are untouched, so the
/// rendered pixels — and thus the FNV-1a hash — are byte-for-byte the same.
///
/// # KE-0401 re-bless (value CHANGES — intentional look change)
///
/// KE-0401 intentionally changes the *look* while leaving the geometry, camera,
/// and transform byte-for-byte identical: the background is now a gradient sky
/// (not the flat clear), lighting is presented through an ACES tonemap + sRGB
/// encode, distance fog blends toward the sky horizon, a directional blob shadow
/// grounds geometry, and the scene is 4x MSAA resolved in-tile. Every one of
/// these changes the rendered pixel values, so this hash MUST be re-blessed. Run:
///
/// ```text
/// BLESS=1 cargo test -p kaman-render --test pixel_hash -- --nocapture
/// ```
///
/// and paste the printed value below. The re-bless is justified because the
/// pixel change is the *point* of the ticket (look), not a geometry regression.
// KE-0401: re-blessed. Was 0x292c5df343b5eba8 through KE-0102..0403; the modern-look
// stack (linear+ACES tonemap+sRGB, gradient sky, distance fog, blob shadow, 4x MSAA)
// intentionally changes the rendered pixels. Geometry/camera/transform are unchanged.
// Re-blessed: fixing the LightUniforms<->MSL Light struct padding mismatch made
// the look stack (fog/shadow/lighting) actually apply as intended, so the
// reference box now renders lit-red instead of fully fogged to the horizon color.
const REFERENCE_HASH: u64 = 0x90d4631260adc5cc;

/// Committed baseline hash of the **reference overlay**'s pixels (FNV-1a 64-bit).
///
/// Guards the KE-0404 2D overlay pass end to end: the pixel→NDC mapping with its
/// `Y` flip, source-over alpha blending in record order, and all three
/// [`OverlayFill`] modes (solid, textured, SDF) — see
/// [`reference_overlay_quads`] for exactly what is drawn and why.
///
/// # KE-0404 first blessing
///
/// Blessed from this machine's first correct overlay frame, on the same Apple GPU
/// that produces [`REFERENCE_HASH`]. Two things were checked before accepting it,
/// because a golden hash is worthless if either fails:
///
/// - **The frame is not blank.** `bless_or_assert` requires more than 8 distinct
///   pixel values, so a baseline can never be blessed from a render that silently
///   drew nothing.
/// - **The 3D baseline did not move.** The same blessing run reprinted
///   `REFERENCE_HASH` as `0x90d4631260adc5cc`, unchanged — confirming the overlay
///   reference is genuinely independent and the shared-helper refactor that
///   introduced it altered no 3D pixel.
///
/// From here it is a normal golden baseline: hold it stable, and re-bless only via
/// the documented path with a justification note in the commit message.
const OVERLAY_REFERENCE_HASH: u64 = 0x2b3859048070b2b6;

/// FNV-1a 64-bit hash over a byte buffer. Self-contained (no external crate) so
/// the golden hash has no dependency surface.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
    }
    hash
}

/// Print the GPU-less skip line for the `what` reference; the caller then returns.
///
/// Shared by both pixel-hash tests so the skip behavior (and its wording) cannot
/// drift apart between them: no Metal device is a **skip**, never a failure.
fn skip_no_gpu(what: &str) {
    eprintln!(
        "skipping {what} pixel-hash test: no Metal device available (GPU-less runner). \
         This is expected on headless CI; it runs and asserts on a real Mac."
    );
}

/// Hash `pixels` and either print a fresh baseline (`BLESS=1`) or assert it
/// matches `baseline`.
///
/// `constant` is the name of the `const` that holds `baseline`, so a `BLESS=1`
/// run that exercises several references says which constant each printed value
/// belongs in, and a failure says which one to look at. Shared by both tests so
/// the blessing contract has exactly one implementation.
fn bless_or_assert(pixels: &[u8], baseline: u64, constant: &str) {
    // A pixel hash only guards anything if the frame actually has content. A
    // regression that drew *nothing* would produce a perfectly stable hash, and
    // once blessed it would pass forever while testing nothing at all. Both
    // references put varied geometry over a gradient sky, so a frame this uniform
    // means the render broke — checked before blessing too, so a blank frame can
    // never be baked into a baseline.
    let distinct = pixels.chunks_exact(4).collect::<std::collections::HashSet<_>>();
    assert!(
        distinct.len() > 8,
        "{constant}: the rendered frame has only {} distinct pixel value(s) — it is \
         effectively blank, so its hash would guard nothing",
        distinct.len(),
    );

    let hash = fnv1a_64(pixels);

    if std::env::var("BLESS").is_ok() {
        println!("BLESS: new {constant} = {hash:#018x}");
        println!("Update {constant} in tests/pixel_hash.rs with a justification note.");
        return;
    }

    assert_eq!(
        hash, baseline,
        "{constant} pixel hash changed ({hash:#018x} != {baseline:#018x}). \
         If this is an intended visual change, re-bless with \
         `BLESS=1 cargo test -p kaman-render --test pixel_hash` and update {constant} \
         with a justification note."
    );
}

/// The 9-float-per-vertex layout used by `kaman-ecs` meshes, packed as raw bytes
/// for the seam: `[pos_xyz, normal_xyz, color_rgb]` == 36-byte stride.
fn vertex_layout() -> VertexLayout {
    VertexLayout::new(
        36,
        vec![
            VertexAttribute {
                location: 0,
                offset: 0,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 1,
                offset: 12,
                format: VertexFormat::Float32x3,
            },
            VertexAttribute {
                location: 2,
                offset: 24,
                format: VertexFormat::Float32x3,
            },
        ],
    )
}

/// A fixed unit cube (color-baked) as `[f32; 9]` vertices + indices, matching the
/// engine's cube geometry. Kept local so the reference scene is fully
/// deterministic and independent of other crates.
fn reference_cube() -> (Vec<[f32; 9]>, Vec<u32>) {
    let c = [0.9, 0.1, 0.1];
    let v = |p: [f32; 3], n: [f32; 3]| [p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2]];
    let vertices = vec![
        // Front (+Z)
        v([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        v([0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        v([0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        v([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        // Back (-Z)
        v([0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
        v([-0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
        v([-0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
        v([0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, // front
        4, 5, 6, 4, 6, 7, // back
    ];
    (vertices, indices)
}

/// Pack `[f32; 9]` vertices into raw bytes for the seam's `MeshData`.
fn pack_vertices(vertices: &[[f32; 9]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len() * 36);
    for v in vertices {
        for f in v {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

/// Render the fixed reference scene offscreen and return its pixel bytes, or
/// `None` if no Metal device is available.
fn render_reference() -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;

    // Fixed camera looking at the cube. The view now comes through the seam
    // (`set_view_projection`, KE-0205) instead of a backend-owned camera: build a
    // `kaman_camera::Camera` here and push its view-projection below.
    let mut camera = Camera::new(WIDTH as f32 / HEIGHT as f32);
    camera.set_position(Vec3::new(0.0, 0.0, 3.0));
    camera.set_target(Vec3::new(0.0, 0.0, 0.0));

    let (vertices, indices) = reference_cube();
    let bytes = pack_vertices(&vertices);
    let mesh = renderer.create_mesh(&MeshData {
        vertices: &bytes,
        indices: &indices,
        layout: vertex_layout(),
    });
    let pipeline = renderer.create_pipeline(&kaman_render_api::PipelineDescriptor {
        vertex_shader: "vertex_main".into(),
        fragment_shader: "fragment_main".into(),
        vertex_layout: vertex_layout(),
    });

    // A fixed, slightly-rotated transform so lighting is visible.
    let transform = Transform::from_position_rotation(
        Vec3::new(0.0, 0.0, 0.0),
        Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.3),
    );

    renderer.begin_frame();
    renderer.set_view_projection(camera.view_projection_matrix());
    renderer.set_pipeline(pipeline);
    renderer.draw_mesh(mesh, &transform, &MaterialParams::default());
    renderer.submit();

    renderer.read_pixels()
}

#[test]
fn reference_scene_matches_committed_hash() {
    let Some(pixels) = render_reference() else {
        skip_no_gpu("reference-scene");
        return;
    };

    bless_or_assert(&pixels, REFERENCE_HASH, "REFERENCE_HASH");
}

// ---------------------------------------------------------------------------
// Reference overlay (KE-0404)
// ---------------------------------------------------------------------------

/// Edge length of the hand-written RGBA pattern the `Textured` quad samples.
const PATTERN_SIZE: u32 = 4;

/// Edge length of the analytic distance field the `Sdf` quad samples.
const SDF_SIZE: u32 = 16;

/// A tiny **hand-written** RGBA8 pattern: four fixed colors in 2x2-texel
/// quadrants.
///
/// Written out in code rather than read from an asset so the overlay reference is
/// self-contained and byte-identical on every machine — a decoded PNG would make
/// the baseline hostage to an image decoder's rounding. The quadrants give the
/// `Textured` mode structure to sample, so a broken UV mapping (a swapped or
/// flipped axis) moves pixels and breaks the hash.
fn reference_pattern() -> Vec<u8> {
    // The four quadrant colors, all opaque: red-orange, green, blue, yellow.
    const A: [u8; 4] = [255, 64, 32, 255];
    const B: [u8; 4] = [32, 255, 96, 255];
    const C: [u8; 4] = [48, 96, 255, 255];
    const D: [u8; 4] = [240, 240, 64, 255];

    let mut rgba = Vec::with_capacity((PATTERN_SIZE * PATTERN_SIZE) as usize * 4);
    for y in 0..PATTERN_SIZE {
        for x in 0..PATTERN_SIZE {
            let texel = match (x / 2) + 2 * (y / 2) {
                0 => A,
                1 => B,
                2 => C,
                _ => D,
            };
            rgba.extend_from_slice(&texel);
        }
    }
    rgba
}

/// A small **analytic** signed-distance field: a centered disc.
///
/// The overlay's SDF mode reads distance from the red channel and treats `0.5` as
/// the edge, so this encodes `0.5` exactly at the disc's rim, above it inside and
/// below it outside, spread linearly over `SPREAD` texels. Computed from a closed
/// form rather than baked from a font so it needs no asset and no font parser, and
/// so every texel is reproducible by inspection. The smooth ramp through `0.5` is
/// what the shader's `fwidth`-derived antialiasing works on — a regression in the
/// SDF branch (reading the wrong channel, losing the coverage smoothstep, or
/// dropping the tint's alpha) changes the disc's edge pixels and breaks the hash.
fn reference_sdf() -> Vec<u8> {
    // Distance, in texels, that the encoded `0..=1` range spans either side of the
    // edge. Wide enough that the ramp is several texels, narrow enough that the
    // field saturates well inside and outside the disc.
    const SPREAD: f32 = 4.0;

    let n = SDF_SIZE as f32;
    let radius = n * 0.3;
    let mut rgba = Vec::with_capacity((SDF_SIZE * SDF_SIZE) as usize * 4);
    for y in 0..SDF_SIZE {
        for x in 0..SDF_SIZE {
            // Texel centers relative to the field's center.
            let dx = x as f32 + 0.5 - n * 0.5;
            let dy = y as f32 + 0.5 - n * 0.5;
            // Signed distance to the rim: positive inside, negative outside.
            let signed = radius - (dx * dx + dy * dy).sqrt();
            let encoded = (0.5 + signed / (2.0 * SPREAD)).clamp(0.0, 1.0);
            let d = (encoded * 255.0).round() as u8;
            // Expanded to RGBA for the seam's upload; the shader samples `.r`.
            rgba.extend_from_slice(&[d, d, d, 255]);
        }
    }
    rgba
}

/// The reference overlay's quads, **in record order**.
///
/// Order is part of the reference, not an implementation detail: the overlay
/// blends source-over, which is not commutative, so recording these in a
/// different order composites different pixels. Quads 0 and 1 overlap with
/// partial alpha precisely so the hash catches a blend-order regression (a
/// backend that batched, sorted, or reversed the recorded stream would change
/// the overlap region).
///
/// All three [`OverlayFill`] modes are covered, so the hash guards each shader
/// branch:
///
/// 0. `Solid`, alpha `0.6` — the lower half of the overlapping pair.
/// 1. `Solid`, alpha `0.5` — the upper half; its top-left corner lies over quad 0.
/// 2. `Textured` — samples the whole [`reference_pattern`], tinted and partly
///    transparent so the tint multiply *and* the blend are both exercised.
/// 3. `Sdf` — samples the whole [`reference_sdf`], opaque tint, so the disc's
///    antialiased edge is the only source of partial coverage.
///
/// Every rect is inside the `WIDTH` x `HEIGHT` drawable, so nothing is clipped
/// and the hash covers each quad in full.
fn reference_overlay_quads(pattern: TextureHandle, sdf: TextureHandle) -> [OverlayQuad; 4] {
    [
        OverlayQuad::solid([8.0, 8.0, 32.0, 32.0], [0.9, 0.2, 0.15, 0.6]),
        OverlayQuad::solid([24.0, 24.0, 32.0, 32.0], [0.15, 0.35, 0.95, 0.5]),
        OverlayQuad {
            rect: [4.0, 40.0, 20.0, 20.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 0.9, 0.8, 0.8],
            fill: OverlayFill::Textured(pattern),
        },
        OverlayQuad {
            rect: [36.0, 4.0, 24.0, 24.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            color: [0.96, 0.96, 0.98, 1.0],
            fill: OverlayFill::Sdf(sdf),
        },
    ]
}

/// Render the fixed reference **overlay** offscreen and return its pixel bytes,
/// or `None` if no Metal device is available.
///
/// Deliberately draws **no 3D geometry and pushes no camera**: the overlay is
/// screen-space, so leaving the scene out isolates the overlay pass (its ortho
/// mapping, blending and fill modes) over the backend's gradient sky. That also
/// keeps this reference fully independent of [`render_reference`]'s — a change to
/// the 3D scene cannot move these pixels, and vice versa.
fn render_reference_overlay() -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;

    // Both textures are built in code (see the builders above), so this render
    // touches no file and is byte-identical on any machine.
    let pattern = renderer.create_texture(&TextureData {
        width: PATTERN_SIZE,
        height: PATTERN_SIZE,
        rgba8: &reference_pattern(),
    });
    let sdf = renderer.create_texture(&TextureData {
        width: SDF_SIZE,
        height: SDF_SIZE,
        rgba8: &reference_sdf(),
    });

    renderer.begin_frame();
    for quad in reference_overlay_quads(pattern, sdf) {
        renderer.draw_overlay_quad(&quad);
    }
    renderer.submit();

    renderer.read_pixels()
}

#[test]
fn reference_overlay_matches_committed_hash() {
    let Some(pixels) = render_reference_overlay() else {
        skip_no_gpu("reference-overlay");
        return;
    };

    bless_or_assert(&pixels, OVERLAY_REFERENCE_HASH, "OVERLAY_REFERENCE_HASH");
}
