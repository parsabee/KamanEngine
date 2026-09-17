// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Headless unit tests for the render seam via [`NullRenderer`].
//!
//! These exercise the exact workflow scene/game code will use: create resources at load time,
//! then record a frame (`begin_frame` → `set_pipeline` → `bind_texture` → `draw_mesh`×N →
//! `submit`) and assert the recorded counts, handles, and bound state — all with no GPU.

use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, NullRenderer, PipelineDescriptor, RenderDevice,
    TextureData, VertexAttribute, VertexFormat, VertexLayout,
};

fn sample_layout() -> VertexLayout {
    VertexLayout::new(
        12,
        vec![VertexAttribute {
            location: 0,
            offset: 0,
            format: VertexFormat::Float32x3,
        }],
    )
}

fn make_mesh(r: &mut NullRenderer) -> kaman_render_api::MeshHandle {
    let layout = sample_layout();
    // 1 vertex worth of bytes (stride = 12).
    let vertices = [0u8; 12];
    r.create_mesh(&MeshData {
        vertices: &vertices,
        indices: &[0, 0, 0],
        layout,
    })
}

fn make_pipeline(r: &mut NullRenderer) -> kaman_render_api::PipelineHandle {
    r.create_pipeline(&PipelineDescriptor {
        vertex_shader: "vs_main".into(),
        fragment_shader: "fs_main".into(),
        vertex_layout: sample_layout(),
    })
}

fn make_texture(r: &mut NullRenderer) -> kaman_render_api::TextureHandle {
    r.create_texture(&TextureData {
        width: 1,
        height: 1,
        rgba8: &[255, 255, 255, 255],
    })
}

#[test]
fn create_resources_hand_out_sequential_unique_handles() {
    let mut r = NullRenderer::new();

    let m0 = make_mesh(&mut r);
    let m1 = make_mesh(&mut r);
    let t0 = make_texture(&mut r);
    let p0 = make_pipeline(&mut r);

    // Sequential per kind.
    assert_eq!(r.created_meshes(), &[m0, m1]);
    assert_eq!(r.created_textures(), &[t0]);
    assert_eq!(r.created_pipelines(), &[p0]);

    // Unique within a kind.
    assert_ne!(m0, m1);

    // Handles are independent across kinds (both may wrap 0).
    assert_eq!(m0.0, 0);
    assert_eq!(t0.0, 0);
    assert_eq!(p0.0, 0);

    // Live counts before any destroy.
    assert_eq!(r.live_mesh_count(), 2);
    assert_eq!(r.live_texture_count(), 1);
    assert_eq!(r.live_pipeline_count(), 1);
}

#[test]
fn destroy_updates_live_counts_and_records_handles() {
    let mut r = NullRenderer::new();
    let m0 = make_mesh(&mut r);
    let _m1 = make_mesh(&mut r);

    r.destroy_mesh(m0);

    assert_eq!(r.destroyed_meshes(), &[m0]);
    assert_eq!(r.live_mesh_count(), 1);
    assert!(r.destroyed_textures().is_empty());
    assert!(r.destroyed_pipelines().is_empty());
}

#[test]
fn records_a_full_frame_with_bound_state_per_draw() {
    let mut r = NullRenderer::new();

    let mesh_a = make_mesh(&mut r);
    let mesh_b = make_mesh(&mut r);
    let tex = make_texture(&mut r);
    let pipeline = make_pipeline(&mut r);

    let t_a = Transform::from_position(Vec3::new(1.0, 0.0, 0.0));
    let t_b = Transform::from_position(Vec3::new(0.0, 2.0, 0.0));
    let mat_a = MaterialParams {
        base_color: [1.0, 0.0, 0.0, 1.0],
        metallic: 0.0,
        roughness: 0.5,
    };
    let mat_b = MaterialParams::default();

    // begin → set_pipeline → bind → draw×N → submit
    assert!(!r.frame_open());
    r.begin_frame();
    assert!(r.frame_open());
    r.set_pipeline(pipeline);
    r.bind_texture(tex);
    r.draw_mesh(mesh_a, &t_a, &mat_a);
    r.draw_mesh(mesh_b, &t_b, &mat_b);
    r.submit();
    assert!(!r.frame_open());

    // Frame protocol bookkeeping.
    assert_eq!(r.frames_begun(), 1);
    assert_eq!(r.frames_submitted(), 1);

    // Recorded draws in order, each carrying the state bound at record time.
    assert_eq!(r.draw_count(), 2);
    let draws = r.draws();

    assert_eq!(draws[0].mesh, mesh_a);
    assert_eq!(draws[0].pipeline, Some(pipeline));
    assert_eq!(draws[0].texture, Some(tex));
    assert_eq!(draws[0].transform.position, t_a.position);
    assert_eq!(draws[0].material, mat_a);

    assert_eq!(draws[1].mesh, mesh_b);
    assert_eq!(draws[1].pipeline, Some(pipeline));
    assert_eq!(draws[1].texture, Some(tex));
    assert_eq!(draws[1].transform.position, t_b.position);
    assert_eq!(draws[1].material, mat_b);
}

#[test]
fn per_frame_state_resets_across_frames() {
    let mut r = NullRenderer::new();
    let mesh = make_mesh(&mut r);
    let pipeline = make_pipeline(&mut r);
    let tex = make_texture(&mut r);

    // Frame 1 binds a pipeline and texture.
    r.begin_frame();
    r.set_pipeline(pipeline);
    r.bind_texture(tex);
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.submit();

    // Frame 2: begin_frame must reset bound state — a draw before set_pipeline/bind
    // records None for both.
    r.begin_frame();
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.submit();

    assert_eq!(r.frames_begun(), 2);
    assert_eq!(r.frames_submitted(), 2);
    assert_eq!(r.draw_count(), 2);
    assert_eq!(r.draws()[0].pipeline, Some(pipeline));
    assert_eq!(r.draws()[0].texture, Some(tex));
    assert_eq!(r.draws()[1].pipeline, None);
    assert_eq!(r.draws()[1].texture, None);
}

#[test]
fn switching_pipeline_and_texture_mid_frame_is_captured_per_draw() {
    let mut r = NullRenderer::new();
    let mesh = make_mesh(&mut r);
    let p0 = make_pipeline(&mut r);
    let p1 = make_pipeline(&mut r);
    let t0 = make_texture(&mut r);
    let t1 = make_texture(&mut r);

    r.begin_frame();
    r.set_pipeline(p0);
    r.bind_texture(t0);
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.set_pipeline(p1);
    r.bind_texture(t1);
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.submit();

    let draws = r.draws();
    assert_eq!(draws[0].pipeline, Some(p0));
    assert_eq!(draws[0].texture, Some(t0));
    assert_eq!(draws[1].pipeline, Some(p1));
    assert_eq!(draws[1].texture, Some(t1));
}
