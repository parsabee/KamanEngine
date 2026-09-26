// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Textured-quad pixel-hash guard (KE-0403).
//!
//! Renders a fixed quad on the **textured** pipeline into an offscreen texture,
//! binding a small non-uniform (checkerboard) base-color texture, then:
//!
//! 1. asserts the rendered image is **non-uniform** — proving the shader actually
//!    sampled the bound texture at the mesh UV rather than emitting a flat color;
//! 2. hashes the pixels and pins them to a committed baseline, so the textured
//!    path (upload + mipmaps + sampling) stays stable.
//!
//! This complements `pixel_hash.rs` (the untextured box), which must stay at its
//! own committed hash — the two pipelines are distinct.
//!
//! # Blessing the baseline
//!
//! ```text
//! BLESS=1 cargo test -p kaman-render --test textured_pixel_hash -- --nocapture
//! ```
//!
//! prints the new hash and passes without asserting.
//!
//! # CI safety
//!
//! Skips (prints a line, returns) when no Metal device is available, so a
//! GPU-less runner never fails this test. It runs and asserts on a real Mac.

use kaman_camera::Camera;
use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render::MetalRenderer;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, PipelineDescriptor, RenderDevice, TextureData,
    VertexAttribute, VertexFormat, VertexLayout,
};

/// Offscreen render size.
const WIDTH: u32 = 64;
/// Offscreen render size.
const HEIGHT: u32 = 64;

/// Committed baseline hash of the textured-quad pixels (FNV-1a 64-bit).
///
/// Blessed from this renderer's first correct textured frame. Re-bless only via
/// the documented `BLESS=1` path with a justification note.
///
/// # KE-0401 re-bless (value CHANGES — intentional look change)
///
/// The textured pipeline now runs the same KE-0401 present stack (ACES tonemap +
/// sRGB encode), distance fog, blob shadow, and 4x MSAA resolve as the untextured
/// path. The mesh, texture (checkerboard), camera, and transform are unchanged,
/// so this is a pure look change — the non-uniformity assertion below still holds
/// — but the pixel bytes shift, so the hash MUST be re-blessed:
///
/// ```text
/// BLESS=1 cargo test -p kaman-render --test textured_pixel_hash -- --nocapture
/// ```
// KE-0401: re-blessed. Was 0x2bb6c070d635c975 (KE-0403); the modern-look stack
// (linear+ACES tonemap+sRGB, sky, fog, shadow, 4x MSAA) intentionally changes the
// pixels. Geometry/camera/transform and the checkerboard texture are unchanged.
// Re-blessed: the LightUniforms<->MSL Light padding fix makes fog/shadow/lighting
// apply correctly, changing the textured quad's shaded pixels.
//
// KE-0406: re-bless REQUIRED (intended look change). The textured pipeline shares
// `lit_linear` with the untextured one, so it moves with the 3D reference for the
// same reasons: ambient rebalanced as sky fill (0.6 -> 0.2), specular now derived
// from the camera position pushed through the seam instead of a constant (0,0,1),
// and the default sun re-expressed as elevation 60 / azimuth 120 (the same
// direction to ~0.01). Geometry, camera, transform and the checkerboard texture are
// unchanged; the sun disc is far outside this frame.
//
// Blessed 2026-09-26: 0xdb98efc9ab5028c9 -> 0x4aa6c2b1732c4ac7.
const REFERENCE_HASH: u64 = 0x4aa6c2b1732c4ac7;

/// FNV-1a 64-bit over a byte buffer (self-contained, no external crate).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
    }
    hash
}

/// The textured `[pos_xyz, normal_xyz, uv]` layout (32-byte stride).
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

/// A front-facing unit quad centered at the origin, packed `[pos,normal,uv]`.
fn textured_quad() -> (Vec<u8>, Vec<u32>) {
    // pos, normal (+Z), uv spanning the full 0..1 range.
    let v = |p: [f32; 3], uv: [f32; 2]| {
        let n = [0.0f32, 0.0, 1.0];
        [p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]]
    };
    let verts = [
        v([-0.8, -0.8, 0.0], [0.0, 1.0]),
        v([0.8, -0.8, 0.0], [1.0, 1.0]),
        v([0.8, 0.8, 0.0], [1.0, 0.0]),
        v([-0.8, 0.8, 0.0], [0.0, 0.0]),
    ];
    let mut bytes = Vec::new();
    for vert in &verts {
        for f in vert {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    let indices = vec![0u32, 1, 2, 0, 2, 3];
    (bytes, indices)
}

/// An 8x8 RGBA checkerboard of two contrasting colors.
fn checkerboard() -> (u32, u32, Vec<u8>) {
    const N: u32 = 8;
    let a = [230u8, 60, 40, 255];
    let b = [40u8, 90, 220, 255];
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            px.extend_from_slice(if (x + y) % 2 == 0 { &a } else { &b });
        }
    }
    (N, N, px)
}

/// Render the textured quad offscreen; return its pixel bytes, or `None` when no
/// Metal device is available.
fn render_textured() -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;

    let mut camera = Camera::new(WIDTH as f32 / HEIGHT as f32);
    camera.set_position(Vec3::new(0.0, 0.0, 2.0));
    camera.set_target(Vec3::new(0.0, 0.0, 0.0));

    let (vertices, indices) = textured_quad();
    let mesh = renderer.create_mesh(&MeshData {
        vertices: &vertices,
        indices: &indices,
        layout: textured_layout(),
    });
    let pipeline = renderer.create_pipeline(&PipelineDescriptor {
        vertex_shader: "textured_vertex_main".into(),
        fragment_shader: "textured_fragment_main".into(),
        vertex_layout: textured_layout(),
    });

    let (tw, th, pixels) = checkerboard();
    let texture = renderer.create_texture(&TextureData {
        width: tw,
        height: th,
        rgba8: &pixels,
    });

    // Camera pushed before the frame opens, matching the engine loop: the sky pass
    // runs inside `begin_frame` and needs the frame's camera to place the sun
    // (KE-0406). The camera's position/target/defaults are unchanged.
    renderer.set_view_projection(camera.view_projection_matrix());
    renderer.set_camera_position(camera.position());

    renderer.begin_frame();
    renderer.set_pipeline(pipeline);
    renderer.bind_texture(texture);
    renderer.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    renderer.submit();

    renderer.read_pixels()
}

/// Count distinct RGBA pixels in a BGRA8 buffer (a coarse non-uniformity probe).
fn distinct_pixels(pixels: &[u8]) -> usize {
    let mut set = std::collections::HashSet::new();
    for p in pixels.chunks_exact(4) {
        set.insert([p[0], p[1], p[2], p[3]]);
    }
    set.len()
}

#[test]
fn textured_quad_is_non_uniform_and_matches_committed_hash() {
    let Some(pixels) = render_textured() else {
        eprintln!(
            "skipping textured pixel-hash test: no Metal device available (GPU-less runner). \
             Runs and asserts on a real Mac."
        );
        return;
    };

    // Proves the shader sampled the bound texture: a checkerboard base color makes
    // the rendered quad non-uniform. A flat-color (unsampled) result would have
    // very few distinct colors (just background + one fill).
    let distinct = distinct_pixels(&pixels);
    assert!(
        distinct > 3,
        "textured quad should be non-uniform (sampled texture); got {distinct} distinct colors"
    );

    let hash = fnv1a_64(&pixels);
    if std::env::var("BLESS").is_ok() {
        println!("BLESS: new textured reference hash = {hash:#018x}");
        return;
    }
    assert_eq!(
        hash, REFERENCE_HASH,
        "textured-quad pixel hash changed ({hash:#018x} != {REFERENCE_HASH:#018x}). \
         Re-bless with `BLESS=1 cargo test -p kaman-render --test textured_pixel_hash` \
         and update REFERENCE_HASH with a justification note."
    );
}
