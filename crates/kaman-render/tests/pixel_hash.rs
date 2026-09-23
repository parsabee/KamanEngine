// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Render pixel-hash guard (KR1.3).
//!
//! Deterministically renders a fixed reference scene into an offscreen Metal
//! texture, reads the pixels back, hashes them, and asserts the hash equals a
//! committed baseline. This is the golden net that pins the migrated renderer's
//! output so the buffer/frames-in-flight refactors (KE-0103/0104/0105) can prove
//! they preserved behavior.
//!
//! # Blessing the baseline
//!
//! The committed hash below was blessed from this migrated renderer's first
//! correct frame. To re-bless after an *intended* visual change, run:
//!
//! ```text
//! BLESS=1 cargo test -p kaman-render --test pixel_hash -- --nocapture
//! ```
//!
//! The test prints the new hash and passes (without asserting) so you can copy
//! it into `REFERENCE_HASH` below **with a justification note in the commit
//! message**. Blessing is deliberately manual and loud.
//!
//! # CI safety (GPU-less runners)
//!
//! GitHub macOS runners are headless with no GPU, so `MTLCreateSystemDefaultDevice`
//! can return nil. When no Metal device is available the test **skips** (prints a
//! skip line and returns) instead of failing, so it never breaks a GPU-less
//! build. On a real Mac (this dev machine) it runs and asserts.

use kaman_camera::Camera;
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render::MetalRenderer;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, RenderDevice, VertexAttribute, VertexFormat,
    VertexLayout,
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
        eprintln!(
            "skipping pixel-hash test: no Metal device available (GPU-less runner). \
             This is expected on headless CI; it runs and asserts on a real Mac."
        );
        return;
    };

    let hash = fnv1a_64(&pixels);

    if std::env::var("BLESS").is_ok() {
        println!("BLESS: new reference hash = {hash:#018x}");
        println!("Update REFERENCE_HASH in tests/pixel_hash.rs with a justification note.");
        return;
    }

    assert_eq!(
        hash, REFERENCE_HASH,
        "reference-scene pixel hash changed ({hash:#018x} != {REFERENCE_HASH:#018x}). \
         If this is an intended visual change, re-bless with \
         `BLESS=1 cargo test -p kaman-render --test pixel_hash` and update REFERENCE_HASH \
         with a justification note."
    );
}
