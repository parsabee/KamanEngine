// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Persistent-mesh-buffer guards (KE-0103).
//!
//! These tests pin the two load-bearing invariants of the persistent mesh
//! registry against the real Metal backend:
//!
//! 1. **Allocation-free per-frame path (KR1.2).** After meshes are uploaded once
//!    at load time, the per-frame path (`begin_frame` … `submit`) issues **zero**
//!    `new_buffer*` calls *for the mesh geometry*. The only allocation left is the
//!    per-draw uniform buffer (KE-0104 removes it); this test asserts the mesh
//!    contribution is gone by drawing with and without extra meshes and checking
//!    the per-frame delta equals exactly one uniform buffer per draw — no mesh
//!    upload.
//! 2. **Stale-handle invariant.** Drawing a `destroy_mesh`'d handle is a defined
//!    no-op (the generational registry rejects it), never a silent wrong-buffer
//!    draw.
//!
//! Like the pixel-hash test, these **skip** when no Metal device is available
//! (GPU-less CI), and run + assert on a real Mac.

use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render::MetalRenderer;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, MeshHandle, RenderDevice, VertexAttribute,
    VertexFormat, VertexLayout,
};

/// Offscreen render size.
const WIDTH: u32 = 64;
/// Offscreen render size.
const HEIGHT: u32 = 64;

/// The `[pos_xyz, normal_xyz, color_rgb]` 36-byte layout the raster backend uses.
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

/// A minimal two-triangle quad (front face of a cube), packed to seam bytes.
fn quad() -> (Vec<u8>, Vec<u32>) {
    let c = [0.9, 0.1, 0.1];
    let v = |p: [f32; 3], n: [f32; 3]| [p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2]];
    let verts = [
        v([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        v([0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
        v([0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        v([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
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

/// Create one persistent mesh on `r` from the quad geometry.
fn make_mesh(r: &mut MetalRenderer) -> MeshHandle {
    let (bytes, indices) = quad();
    r.create_mesh(&MeshData {
        vertices: &bytes,
        indices: &indices,
        layout: vertex_layout(),
    })
}

/// Record a one-draw frame for `handle`.
fn draw_one(r: &mut MetalRenderer, handle: MeshHandle) {
    r.begin_frame();
    r.draw_mesh(handle, &Transform::identity(), &MaterialParams::default());
    r.submit();
}

#[test]
fn per_frame_path_allocates_no_mesh_buffers() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };
    r.camera_mut().set_position(Vec3::new(0.0, 0.0, 3.0));
    r.camera_mut().set_target(Vec3::ZERO);

    // Load time: upload two meshes once.
    let m0 = make_mesh(&mut r);
    let m1 = make_mesh(&mut r);
    assert_eq!(r.live_mesh_count(), 2);

    // Warm one frame so any lazy per-frame state (e.g. depth texture) is created
    // before we measure — we want to prove *steady-state* per-frame allocation.
    draw_one(&mut r, m0);

    // Measure a frame that draws both persistent meshes. The only allocation on
    // the per-frame path is one uniform buffer per draw (KE-0104 removes it);
    // crucially, NO mesh buffer is re-uploaded.
    let before = r.allocation_count();
    r.begin_frame();
    r.draw_mesh(m0, &Transform::identity(), &MaterialParams::default());
    r.draw_mesh(m1, &Transform::identity(), &MaterialParams::default());
    r.submit();
    let per_frame = r.allocation_count() - before;

    // Exactly one uniform buffer per draw, and zero mesh uploads. If a mesh were
    // re-uploaded, this would be > 2.
    assert_eq!(
        per_frame, 2,
        "per-frame path allocated {per_frame} buffers for 2 draws; expected 2 \
         (one uniform each) and ZERO mesh uploads"
    );
}

#[test]
fn drawing_a_destroyed_mesh_is_a_defined_no_op() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };
    r.camera_mut().set_position(Vec3::new(0.0, 0.0, 3.0));
    r.camera_mut().set_target(Vec3::ZERO);

    let mesh = make_mesh(&mut r);
    assert_eq!(r.live_mesh_count(), 1);

    // Destroy it; the handle is now stale.
    r.destroy_mesh(mesh);
    assert_eq!(r.live_mesh_count(), 0);

    // Drawing the stale handle must not allocate a uniform buffer or draw — the
    // registry lookup fails cleanly before any GPU work. So the per-frame
    // allocation delta is 0 (no uniform buffer for a rejected draw).
    let before = r.allocation_count();
    draw_one(&mut r, mesh);
    let per_frame = r.allocation_count() - before;
    assert_eq!(
        per_frame, 0,
        "a stale-handle draw must be a no-op (no uniform buffer), got {per_frame} allocations"
    );

    // A brand-new mesh reusing the freed slot gets a DIFFERENT handle and draws
    // fine — the stale handle never resolves to it.
    let fresh = make_mesh(&mut r);
    assert_ne!(fresh, mesh, "reused slot must yield a new (bumped) handle");
    let before = r.allocation_count();
    draw_one(&mut r, fresh);
    assert!(
        r.allocation_count() > before,
        "a live mesh should draw (allocating its uniform buffer)"
    );

    // The stale handle is still a no-op even after slot reuse.
    let before = r.allocation_count();
    draw_one(&mut r, mesh);
    assert_eq!(
        r.allocation_count() - before,
        0,
        "stale handle must stay a no-op after its slot is reused"
    );
}
