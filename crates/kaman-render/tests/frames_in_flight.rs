// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Frames-in-flight + ring-region rotation guards (KE-0105).
//!
//! These tests pin the triple-buffered pacing model against the real Metal
//! backend:
//!
//! 1. **No deadlock under the semaphore.** Running more than
//!    `MAX_FRAMES_IN_FLIGHT` frames through `begin_frame` / `draw` / `submit`
//!    must complete: the CPU acquires a permit per frame and each command
//!    buffer's completion handler releases one, so the semaphore stays balanced
//!    and the loop never blocks forever. The offscreen `submit` waits for GPU
//!    completion (and thus the completion handler) inline, which makes the whole
//!    sequence deterministic.
//! 2. **Ring region rotates `0,1,2,0,1,2,…`.** Each frame's ring region base
//!    must cycle through the `MAX_FRAMES_IN_FLIGHT` disjoint regions so an
//!    in-flight frame's uniform slots are never overwritten.
//! 3. **Zero per-frame allocation still holds** across many frames — the
//!    frames-in-flight machinery adds no `new_buffer*` to the hot path.
//!
//! Like the other backend tests, these **skip** when no Metal device is
//! available (GPU-less CI) and run + assert on a real Mac.

use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render::backend::MAX_FRAMES_IN_FLIGHT;
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

/// A minimal two-triangle quad, packed to seam bytes.
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

#[test]
fn many_frames_do_not_deadlock_and_ring_region_rotates() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };
    r.camera_mut().set_position(Vec3::new(0.0, 0.0, 3.0));
    r.camera_mut().set_target(Vec3::ZERO);

    let mesh = make_mesh(&mut r);

    // Per-frame capacity is fixed (a single draw never grows the ring), so the
    // region index is `region_base / draws_per_frame`. Snapshot it once.
    let draws_per_frame = r.ring_draws_per_frame_for_test();

    // Run well past a full triple-buffer cycle. If the completion handler failed
    // to release the semaphore, the 4th `begin_frame` would block forever and
    // this test would hang (the CI timeout would catch the deadlock) — reaching
    // the asserts *is* the no-deadlock proof.
    let frames = (2 * MAX_FRAMES_IN_FLIGHT) as usize + 1; // 7 > 2*3
    for i in 0..frames {
        // frame_index counts *submitted* frames, so it equals `i` at begin_frame.
        assert_eq!(
            r.frame_index_for_test(),
            i as u64,
            "frame_index should equal the number of frames submitted so far"
        );

        r.begin_frame();

        // The region base selected for this frame must be the expected slot of
        // the rotation, and its region index must cycle 0,1,2,0,1,2,...
        let region_base = r.ring_region_base_for_test();
        let expected_region = (i as u64) % MAX_FRAMES_IN_FLIGHT;
        assert_eq!(
            region_base,
            expected_region * draws_per_frame,
            "frame {i}: ring region base should be region {expected_region}"
        );
        assert_eq!(
            region_base / draws_per_frame,
            expected_region,
            "frame {i}: ring region index must rotate 0,1,2,0,1,2,..."
        );

        r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
        r.submit();
    }

    // All frames completed without a hang, and the index advanced once per frame.
    assert_eq!(
        r.frame_index_for_test(),
        frames as u64,
        "frame_index must advance exactly once per submitted frame"
    );
}

#[test]
fn per_frame_path_allocates_nothing_across_the_in_flight_cycle() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };
    r.camera_mut().set_position(Vec3::new(0.0, 0.0, 3.0));
    r.camera_mut().set_target(Vec3::ZERO);

    let mesh = make_mesh(&mut r);

    // Warm one frame so lazy per-frame state (depth texture) exists before we
    // measure steady state.
    r.begin_frame();
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.submit();

    // Measure several frames spanning a full triple-buffer cycle. The
    // frames-in-flight semaphore + completion handler must not add any
    // `new_buffer*` to the hot path — the per-frame allocation delta stays 0.
    let before = r.allocation_count();
    for _ in 0..(2 * MAX_FRAMES_IN_FLIGHT) {
        r.begin_frame();
        r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
        r.submit();
    }
    let delta = r.allocation_count() - before;
    assert_eq!(
        delta, 0,
        "frames-in-flight path allocated {delta} buffers across the cycle; expected 0"
    );
}
