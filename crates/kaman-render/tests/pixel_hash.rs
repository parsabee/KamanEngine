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
//! Three independent references are pinned, each with its own baseline constant:
//!
//! 1. **The 3D reference scene** ([`REFERENCE_HASH`]) — a lit, rotated box through
//!    the Phong pipeline and the KE-0401 look stack.
//! 2. **The 2D reference overlay** ([`OVERLAY_REFERENCE_HASH`], KE-0404) — the HUD
//!    overlay pass: a solid quad, a textured quad, an SDF quad, and a pair of
//!    overlapping partial-alpha quads whose record order the blend must respect.
//! 3. **The sun disc** ([`SUN_REFERENCE_HASH`], KE-0406) — an empty frame (so it is
//!    purely the sky pass) with the sun placed in view, guarding the disc, its glow
//!    falloff, and the azimuth convention. The other two point *away* from the sun
//!    on purpose, which is what makes reference 2 a useful canary but also left the
//!    disc with no coverage until this one existed.
//! 4. **The shadow map** ([`SHADOW_REFERENCE_HASH`], KE-0407) — a floating cube
//!    casting onto a ground split between the textured and untextured pipelines.
//!    Alongside it, two behavioural tests prove the shadow comes from the shadow
//!    pass (disabling its casters removes it) and follows the sun (turning the sun
//!    moves it).
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
//
// KE-0406: re-bless REQUIRED (intended look change). Three deliberate changes move
// this scene's pixels, none of them geometry:
//   1. Ambient is rebalanced as sky fill (0.6 -> 0.2) against an unchanged diffuse
//      0.8, so the box's unlit faces darken and it reads sunlit instead of overcast.
//   2. Specular is view-dependent: `viewDir` now comes from the camera position
//      pushed through the seam instead of a hardcoded (0,0,1), and a highlight is
//      suppressed on faces the sun does not reach.
//   3. The default sun is expressed as elevation 60 / azimuth 120 via `SunSky`,
//      which reproduces the old (-0.5,-1.0,-0.3) direction to within ~0.01 after
//      normalisation — a sub-degree shift, but a pixel-level one.
// The sky gradient colours are unchanged, and the sun disc is nowhere near this
// frame (the default sun is ~75 degrees off the view axis, far outside the 18-degree
// glow), so the background is expected to be identical. Geometry, camera position,
// target and transform are all untouched.
//
// Blessed 2026-09-26: 0x90d4631260adc5cc -> 0xc3bc162d10925b22. The same blessing
// run reprinted OVERLAY_REFERENCE_HASH unchanged, and the frame was checked to be
// genuinely lit rather than flat (mean luminance 157, sd 66 over 34..216, 144
// distinct values, the red box covering 1421 of 4096 pixels) — the failure mode
// the KE-0401 note above describes is a low-variance frame fogged to the horizon
// colour, which this is not.
//
// KE-0407: re-bless REQUIRED (intended look change): 0xc3bc162d10925b22 ->
// 0xbb6d718f1804993d. The fake `ground_shadow` blob is deleted. It was centred on
// the world origin (radius 1.2, strength 0.5) — exactly where this box sits — so it
// was darkening the box's lit faces; without it they are brighter. The real shadow
// map adds nothing to this frame: re-rendering it with the shadow pass's casters
// disabled (`set_shadows_enabled(false)`) gives the *same* 0xbb6d718f1804993d, so
// the box does not shadow itself anywhere (no acne) and the whole change is the blob
// removal. Geometry, camera, sun and transform are untouched; OVERLAY_REFERENCE_HASH
// and SUN_REFERENCE_HASH reprinted unchanged in the same run.
const REFERENCE_HASH: u64 = 0xbb6d718f1804993d;

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
///
/// # KE-0406 (value must NOT change)
///
/// KE-0406 changes lighting and adds a sun disc to the sky, and this reference
/// renders the sky behind its quads — so it is the canary for a look change leaking
/// into screen space. It must come back **unchanged**, and the change was built so
/// that it provably does:
///
/// - The ambient rebalance and the view-dependent specular only affect *shaded
///   geometry*, and this reference deliberately draws none.
/// - The sky gradient formula and the default zenith/horizon colours are untouched.
/// - This reference pushes no camera, so the view-projection is the identity, whose
///   inverse unprojects every pixel to the same view ray `(0, 0, 1)`. The default
///   sun sits ~75° off it, well outside the 18° glow extent, so the sun term is
///   exactly zero at every pixel — a multiply by zero, not a small value.
///
/// If this hash moves, one of those three claims is false: look for lighting state
/// bleeding into the overlay pass rather than re-blessing it.
const OVERLAY_REFERENCE_HASH: u64 = 0x2b3859048070b2b6;

/// Committed baseline hash of the **sun-disc reference**'s pixels (FNV-1a 64-bit).
///
/// Guards the KE-0406 sun disc and its glow, which nothing else covers: both other
/// references deliberately point away from the sun, and the `playable-demo` camera
/// cannot frame it. See [`render_reference_sun`] for why an empty frame is the
/// right isolation here.
///
/// # KE-0406 first blessing
///
/// Blessed 2026-09-26. Unlike the other two, this frame is *mostly* sun glow, so a
/// regression that silently dropped the disc would still leave a smooth gradient
/// behind and could hash stably forever. Two things were therefore checked before
/// accepting the value, and both are recorded here because neither is obvious from
/// the constant alone:
///
/// - **The disc is really on screen.** The brightest pixel saturates at 255 at
///   `(x=31, y=20)` of 64x64 — horizontally centred and above centre, exactly where
///   a sun due north at 8 degrees lands for a level camera looking north. It sits
///   1.21x its own row's mean; the ratio is modest because the disc spans only
///   ~2.7 pixels at this resolution and the glow already lifts the whole row, which
///   is why the disable check below matters more than the ratio.
/// - **The hash actually depends on the disc.** Setting `SUN_DISC_GAIN` to `0` in
///   the shader moved this hash to `0x79a74f9a300fd87d`; restoring it brought
///   `0x93bf3a3f86062cbd` back. The guard is live, not decorative.
const SUN_REFERENCE_HASH: u64 = 0x93bf3a3f86062cbd;

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

    // Push the camera *before* opening the frame, the order the engine loop uses:
    // the sky pass runs at `begin_frame`, so the frame's camera must be in effect
    // by then for it to place the sun (KE-0406). The camera itself — position,
    // target and every default — is unchanged; only the call order is.
    renderer.set_view_projection(camera.view_projection_matrix());
    // The eye position for view-dependent specular (KE-0406); the same camera,
    // just fully described.
    renderer.set_camera_position(camera.position());

    renderer.begin_frame();
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

// ============================================================================
// Reference 3: the sun disc (KE-0406)
// ============================================================================
//
// Neither reference above frames the sun: both use a camera whose view axis is
// ~75 degrees off the default sun, well outside the 18-degree glow, so the disc
// and glow contribute *exactly* zero to their pixels. That is deliberate for
// them — it is what makes `OVERLAY_REFERENCE_HASH` a usable canary — but it left
// the disc itself with no golden coverage at all, and the `playable-demo` camera
// cannot show it either (it pitches 20.6 degrees down, so its top of frame sits
// 1.9 degrees above the horizon). This third reference exists solely to put the
// sun on screen, so the disc, the glow falloff and the azimuth convention are
// pinned by pixels and not only by unit tests on the direction math.

/// Sun/sky for the sun-disc reference.
///
/// The sun is placed **due north at 8 degrees** and viewed by a level camera
/// looking north, which puts the disc slightly above frame centre with the glow
/// falling off into the gradient all around it. Azimuth `0` is the convention's
/// north (`-Z`), so this frame also witnesses that convention: were the bearing
/// ever redefined, the sun would leave the frame and this hash would move.
///
/// The colours match the `playable-demo`'s summer-afternoon sun, so this
/// reference guards the look the game actually asks for.
fn reference_sun_sky() -> kaman_render_api::SunSky {
    kaman_render_api::SunSky {
        sun_elevation_deg: 8.0,
        sun_azimuth_deg: 0.0,
        sun_color: [1.0, 0.96, 0.88],
        sun_intensity: 1.15,
        sky_fill: 0.22,
        sky_zenith_color: [0.11, 0.27, 0.56],
        sky_horizon_color: [0.62, 0.67, 0.74],
    }
}

/// Render the sun-disc reference offscreen and return its pixel bytes, or `None`
/// if no Metal device is available.
///
/// Draws **no geometry**: the sky is a fullscreen pass inside `begin_frame`, so an
/// empty frame is exactly the sky and nothing else. That isolates the disc from
/// any lighting or mesh change — only the sky shader and the sun direction can
/// move these pixels. The camera is pushed *before* `begin_frame` because the sky
/// pass runs there and unprojects the frame's camera to get its view rays.
fn render_reference_sun() -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;

    // Level camera at the origin looking north (`-Z`), so the horizon crosses the
    // middle of the frame and the sky fills the upper half.
    let mut camera = Camera::new(WIDTH as f32 / HEIGHT as f32);
    camera.set_position(Vec3::ZERO);
    camera.set_target(Vec3::new(0.0, 0.0, -1.0));

    renderer.set_sun_sky(&reference_sun_sky());
    renderer.set_view_projection(camera.view_projection_matrix());
    renderer.set_camera_position(camera.position());

    renderer.begin_frame();
    renderer.submit();

    renderer.read_pixels()
}

#[test]
fn reference_sun_matches_committed_hash() {
    let Some(pixels) = render_reference_sun() else {
        skip_no_gpu("reference-sun");
        return;
    };

    bless_or_assert(&pixels, SUN_REFERENCE_HASH, "SUN_REFERENCE_HASH");
}

// ---------------------------------------------------------------------------
// Shadow reference (KE-0407)
// ---------------------------------------------------------------------------

/// Committed baseline hash of the **shadow reference**'s pixels (FNV-1a 64-bit).
///
/// Guards the KE-0407 shadow-map pass end to end — the depth-only caster pass, the
/// fitted light projection, and the receiver lookup in *both* lit pipelines — with
/// a scene built so a caster demonstrably darkens a receiver: see
/// [`render_reference_shadow`]. [`reference_shadow_is_cast_by_the_shadow_pass`]
/// renders the same scene with the shadow pass's casters disabled and requires the
/// frame to change (and the predicted shadow to be where the darkening is), so the
/// hash cannot be quietly guarding a frame with no shadow in it.
///
/// # KE-0407 first blessing
///
/// Blessed 2026-09-27 on this machine's Apple GPU. Checked before accepting it:
///
/// - **The shadow is really there, on both pipelines.** At the two predicted
///   shadow points the 3x3 mean luminance drops from 217 → 132 on the textured
///   (west) receiver and 204 → 111 on the untextured (east) receiver when the
///   casters are switched on; points away from the shadow are unchanged. The frame
///   was also inspected: one soft-edged shadow, east of the cube, crossing the
///   textured/untextured seam.
/// - **The hash depends on the pass.** With the shadow pass's casters disabled
///   (`set_shadows_enabled(false)`) the same scene hashes to `0xe105de70657b1d19`
///   instead — the guard is live, not decorative.
/// - **The other baselines behave.** The same blessing run reprinted
///   `OVERLAY_REFERENCE_HASH` and `SUN_REFERENCE_HASH` unchanged.
const SHADOW_REFERENCE_HASH: u64 = 0xfeea97900d873f0a;

/// The sun the shadow reference is lit by: 50° up at `azimuth_deg` (the reference
/// uses `270`, due **west**, so the light travels toward `+X` and the caster's
/// shadow falls to its east).
fn reference_shadow_sun(azimuth_deg: f32) -> kaman_render_api::SunSky {
    kaman_render_api::SunSky {
        sun_elevation_deg: 50.0,
        sun_azimuth_deg: azimuth_deg,
        ..kaman_render_api::SunSky::default()
    }
}

/// World position of the floating caster cube's centre (unit cube).
const SHADOW_CASTER_CENTER: Vec3 = Vec3::new(-0.6, 1.0, 0.0);

/// A full, closed unit cube (all six faces), untextured `[pos,normal,color]`.
fn caster_cube() -> (Vec<[f32; 9]>, Vec<u32>) {
    let c = [0.85, 0.75, 0.2];
    // (normal, u axis, v axis) per face.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (n, u, v) in faces {
        let base = vertices.len() as u32;
        for (su, sv) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            let p = [
                n[0] * 0.5 + u[0] * su + v[0] * sv,
                n[1] * 0.5 + u[1] * su + v[1] * sv,
                n[2] * 0.5 + u[2] * su + v[2] * sv,
            ];
            vertices.push([p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2]]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// An untextured `[pos,normal,color]` ground quad at `y = 0` spanning `x0..x1`,
/// `z -3..3`, facing up.
fn untextured_ground(x0: f32, x1: f32) -> (Vec<[f32; 9]>, Vec<u32>) {
    let c = [0.55, 0.6, 0.65];
    let v = |x: f32, z: f32| [x, 0.0, z, 0.0, 1.0, 0.0, c[0], c[1], c[2]];
    (
        vec![v(x0, 3.0), v(x1, 3.0), v(x1, -3.0), v(x0, -3.0)],
        vec![0, 1, 2, 0, 2, 3],
    )
}

/// The `[pos,normal,uv]` layout of the textured pipeline (32-byte stride).
fn textured_layout() -> VertexLayout {
    VertexLayout::new(
        32,
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
                format: VertexFormat::Float32x2,
            },
        ],
    )
}

/// A textured ground quad at `y = 0` spanning `x0..x1`, `z -3..3`, facing up,
/// packed `[pos,normal,uv]`.
fn textured_ground(x0: f32, x1: f32) -> (Vec<u8>, Vec<u32>) {
    let v = |x: f32, z: f32, uv: [f32; 2]| [x, 0.0, z, 0.0, 1.0, 0.0, uv[0], uv[1]];
    let verts = [
        v(x0, 3.0, [0.0, 1.0]),
        v(x1, 3.0, [1.0, 1.0]),
        v(x1, -3.0, [1.0, 0.0]),
        v(x0, -3.0, [0.0, 0.0]),
    ];
    let mut bytes = Vec::new();
    for vert in &verts {
        for f in vert {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    (bytes, vec![0, 1, 2, 0, 2, 3])
}

/// The shadow reference's camera: above and in front, looking down at the origin.
fn reference_shadow_camera() -> Camera {
    let mut camera = Camera::new(WIDTH as f32 / HEIGHT as f32);
    camera.set_position(Vec3::new(0.0, 4.0, 5.0));
    camera.set_target(Vec3::new(0.0, 0.0, 0.0));
    camera
}

/// Render the shadow reference offscreen, or `None` without a Metal device.
///
/// A floating cube (untextured pipeline) hangs over a ground split in two: the
/// west half (`x < 0`) is drawn by the **textured** pipeline, the east half by the
/// **untextured** one. Under the west sun of [`reference_shadow_sun`] the cube's
/// shadow falls east across the seam, so one frame proves both pipelines receive.
/// `shadows` toggles the shadow pass's casters; `azimuth_deg` turns the sun.
fn render_reference_shadow(shadows: bool, azimuth_deg: f32) -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;
    renderer.set_shadows_enabled(shadows);

    let untextured = renderer.create_pipeline(&kaman_render_api::PipelineDescriptor {
        vertex_shader: "vertex_main".into(),
        fragment_shader: "fragment_main".into(),
        vertex_layout: vertex_layout(),
    });
    let textured = renderer.create_pipeline(&kaman_render_api::PipelineDescriptor {
        vertex_shader: "textured_vertex_main".into(),
        fragment_shader: "textured_fragment_main".into(),
        vertex_layout: textured_layout(),
    });

    let (cube_v, cube_i) = caster_cube();
    let cube = renderer.create_mesh(&MeshData {
        vertices: &pack_vertices(&cube_v),
        indices: &cube_i,
        layout: vertex_layout(),
    });
    let (east_v, east_i) = untextured_ground(0.0, 3.0);
    let east = renderer.create_mesh(&MeshData {
        vertices: &pack_vertices(&east_v),
        indices: &east_i,
        layout: vertex_layout(),
    });
    let (west_bytes, west_i) = textured_ground(-3.0, 0.0);
    let west = renderer.create_mesh(&MeshData {
        vertices: &west_bytes,
        indices: &west_i,
        layout: textured_layout(),
    });
    // A light, even base colour (2x2 so it has a real mip chain) — the shadow is
    // what should vary across the west half, not the texture.
    let texture = renderer.create_texture(&TextureData {
        width: 2,
        height: 2,
        rgba8: &[210u8, 200, 190, 255].repeat(4),
    });

    let camera = reference_shadow_camera();
    renderer.set_sun_sky(&reference_shadow_sun(azimuth_deg));
    renderer.set_view_projection(camera.view_projection_matrix());
    renderer.set_camera_position(camera.position());

    renderer.begin_frame();
    renderer.set_pipeline(textured);
    renderer.bind_texture(texture);
    renderer.draw_mesh(west, &Transform::identity(), &MaterialParams::default());
    renderer.set_pipeline(untextured);
    renderer.draw_mesh(east, &Transform::identity(), &MaterialParams::default());
    renderer.draw_mesh(
        cube,
        &Transform::from_position(SHADOW_CASTER_CENTER),
        &MaterialParams::default(),
    );
    renderer.submit();

    renderer.read_pixels()
}

/// Mean luminance (0..255) of the 3x3 pixels around where `world` projects in the
/// shadow reference's camera.
fn luminance_at(pixels: &[u8], world: Vec3) -> f32 {
    let clip = reference_shadow_camera().view_projection_matrix() * world.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    let px = ((ndc.x * 0.5 + 0.5) * WIDTH as f32) as i32;
    let py = ((0.5 - ndc.y * 0.5) * HEIGHT as f32) as i32;
    let mut sum = 0.0;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let x = (px + dx).clamp(0, WIDTH as i32 - 1) as usize;
            let y = (py + dy).clamp(0, HEIGHT as i32 - 1) as usize;
            let i = (y * WIDTH as usize + x) * 4;
            // BGRA8.
            let (b, g, r) = (pixels[i] as f32, pixels[i + 1] as f32, pixels[i + 2] as f32);
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
        }
    }
    sum / 9.0
}

/// A ground point in the cube's shadow on the **textured** (west) half, under the
/// west sun: the ray back toward the sun passes through the cube.
const SHADOWED_TEXTURED: Vec3 = Vec3::new(-0.3, 0.0, 0.0);
/// A ground point in the cube's shadow on the **untextured** (east) half.
const SHADOWED_UNTEXTURED: Vec3 = Vec3::new(0.6, 0.0, 0.0);
/// A ground point nowhere near the shadow on the textured half.
const LIT_TEXTURED: Vec3 = Vec3::new(-2.2, 0.0, 1.8);
/// A ground point nowhere near the shadow on the untextured half.
const LIT_UNTEXTURED: Vec3 = Vec3::new(2.2, 0.0, 1.8);

#[test]
fn reference_shadow_matches_committed_hash() {
    let Some(pixels) = render_reference_shadow(true, 270.0) else {
        skip_no_gpu("reference-shadow");
        return;
    };

    bless_or_assert(&pixels, SHADOW_REFERENCE_HASH, "SHADOW_REFERENCE_HASH");
}

#[test]
fn reference_shadow_is_cast_by_the_shadow_pass() {
    // The test gate's "verify by disabling it, not by assuming": the same scene
    // with the shadow pass's casters switched off must differ, and the difference
    // must be the predicted shadow darkening both receivers — not noise elsewhere.
    let (Some(on), Some(off)) = (
        render_reference_shadow(true, 270.0),
        render_reference_shadow(false, 270.0),
    ) else {
        skip_no_gpu("reference-shadow");
        return;
    };

    assert_ne!(
        fnv1a_64(&on),
        fnv1a_64(&off),
        "disabling the shadow pass changed nothing: the reference has no shadow in it"
    );
    for (point, what) in [
        (SHADOWED_TEXTURED, "textured"),
        (SHADOWED_UNTEXTURED, "untextured"),
    ] {
        let (lit, shadowed) = (luminance_at(&off, point), luminance_at(&on, point));
        assert!(
            shadowed < lit * 0.75,
            "the {what} receiver at {point:?} is not darkened by the cube's shadow \
             (luminance {shadowed} with shadows vs {lit} without)"
        );
    }
    for point in [LIT_TEXTURED, LIT_UNTEXTURED] {
        let (a, b) = (luminance_at(&off, point), luminance_at(&on, point));
        assert!(
            (a - b).abs() < 1.0,
            "{point:?} is far from the shadow but changed ({b} vs {a}) — acne or a leak"
        );
    }
}

#[test]
fn turning_the_sun_moves_the_shadow() {
    // Shadowing is driven by the KE-0406 sun, not a second light: swing the same
    // 50° sun from due west to due east and the shadow must leave the east half and
    // appear on the west half, mirrored about the cube.
    let (Some(west_sun), Some(east_sun)) = (
        render_reference_shadow(true, 270.0),
        render_reference_shadow(true, 90.0),
    ) else {
        skip_no_gpu("reference-shadow");
        return;
    };
    // Mirror of SHADOWED_UNTEXTURED about the cube centre (x = -0.6).
    let mirrored = Vec3::new(
        2.0 * SHADOW_CASTER_CENTER.x - SHADOWED_UNTEXTURED.x,
        0.0,
        0.0,
    );
    assert!(
        luminance_at(&east_sun, SHADOWED_UNTEXTURED)
            > luminance_at(&west_sun, SHADOWED_UNTEXTURED) * 1.3,
        "with the sun in the east, the east-side point should be lit"
    );
    assert!(
        luminance_at(&east_sun, mirrored) < luminance_at(&west_sun, mirrored) * 0.75,
        "with the sun in the east, the shadow should fall west of the cube"
    );
}
