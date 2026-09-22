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
//!    `new_buffer*` calls. Meshes are persistent (KE-0103) and per-draw uniforms
//!    are written into the persistent uniform ring (KE-0104), so the per-frame
//!    allocation delta is exactly `0` — this is the KR1.2 "zero allocations in
//!    the per-frame path" proof.
//! 2. **Stale-handle invariant.** Drawing a `destroy_mesh`'d handle is a defined
//!    no-op (the generational registry rejects it), never a silent wrong-buffer
//!    draw.
//!
//! Like the pixel-hash test, these **skip** when no Metal device is available
//! (GPU-less CI), and run + assert on a real Mac.

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

    // Load time: upload two meshes once.
    let m0 = make_mesh(&mut r);
    let m1 = make_mesh(&mut r);
    assert_eq!(r.live_mesh_count(), 2);

    // Warm one frame so any lazy per-frame state (e.g. depth texture) is created
    // before we measure — we want to prove *steady-state* per-frame allocation.
    draw_one(&mut r, m0);

    // Measure a frame that draws both persistent meshes. Meshes are persistent
    // (KE-0103) and per-draw uniforms come from the persistent ring (KE-0104),
    // so the per-frame path must allocate NOTHING.
    let before = r.allocation_count();
    r.begin_frame();
    r.draw_mesh(m0, &Transform::identity(), &MaterialParams::default());
    r.draw_mesh(m1, &Transform::identity(), &MaterialParams::default());
    r.submit();
    let per_frame = r.allocation_count() - before;

    // KR1.2: zero allocations on the per-frame path. If a mesh were re-uploaded
    // or a per-draw uniform buffer allocated, this would be > 0.
    assert_eq!(
        per_frame, 0,
        "per-frame path allocated {per_frame} buffers for 2 draws; expected 0 \
         (persistent meshes + uniform ring, ZERO hot-path allocation)"
    );
}

#[test]
fn uniform_ring_slot_offsets_are_256_byte_aligned() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    // The per-slot stride is the Apple GPU offset requirement.
    assert_eq!(
        r.ring_stride_for_test() % 256,
        0,
        "uniform ring stride must be a multiple of 256"
    );

    let mesh = make_mesh(&mut r);

    // Draw several meshes in one frame and check that EACH bound uniform offset
    // is 256-byte aligned (the Apple GPU `set_vertex_buffer` offset rule).
    r.begin_frame();
    for _ in 0..8 {
        let offset = r.ring_next_offset_for_test();
        assert_eq!(
            offset % 256,
            0,
            "uniform ring slot offset {offset} is not 256-byte aligned"
        );
        r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    }
    r.submit();
}

#[test]
fn drawing_a_destroyed_mesh_is_a_defined_no_op() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    let mesh = make_mesh(&mut r);
    assert_eq!(r.live_mesh_count(), 1);

    // Destroy it; the handle is now stale.
    r.destroy_mesh(mesh);
    assert_eq!(r.live_mesh_count(), 0);

    // Drawing the stale handle must not draw — the registry lookup fails cleanly
    // before any GPU work. The per-frame allocation delta is 0 either way now
    // (uniforms come from the ring), so correctness of the no-op is pinned by
    // the ring-cursor: a rejected draw must NOT consume a ring slot. Measured
    // inside one frame (begin_frame resets the cursor to 0).
    r.begin_frame();
    let before_cursor = r.ring_cursor_for_test();
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    let after_cursor = r.ring_cursor_for_test();
    r.submit();
    assert_eq!(
        after_cursor, before_cursor,
        "a stale-handle draw must be a no-op — it must not consume a ring slot"
    );

    // A brand-new mesh reusing the freed slot gets a DIFFERENT handle and draws
    // fine — the stale handle never resolves to it. A live draw consumes exactly
    // one ring slot (and, per KR1.2, allocates nothing).
    let fresh = make_mesh(&mut r);
    assert_ne!(fresh, mesh, "reused slot must yield a new (bumped) handle");
    let before_alloc = r.allocation_count();
    r.begin_frame();
    let before_cursor = r.ring_cursor_for_test();
    r.draw_mesh(fresh, &Transform::identity(), &MaterialParams::default());
    let after_cursor = r.ring_cursor_for_test();
    r.submit();
    assert_eq!(
        after_cursor - before_cursor,
        1,
        "a live mesh should draw, consuming exactly one uniform-ring slot"
    );
    assert_eq!(
        r.allocation_count(),
        before_alloc,
        "a live mesh draw must not allocate (uniform ring, KR1.2)"
    );

    // The stale handle is still a no-op even after slot reuse.
    r.begin_frame();
    let before_cursor = r.ring_cursor_for_test();
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    let after_cursor = r.ring_cursor_for_test();
    r.submit();
    assert_eq!(
        after_cursor, before_cursor,
        "stale handle must stay a no-op after its slot is reused"
    );
}
