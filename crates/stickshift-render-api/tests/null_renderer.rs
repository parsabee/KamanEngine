// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Headless unit tests for the render seam via [`NullRenderer`].
//!
//! These exercise the exact workflow scene/game code will use: create resources at load time,
//! then record a frame (`begin_frame` → `set_pipeline` → `bind_texture` → `draw_mesh`×N →
//! `submit`) and assert the recorded counts, handles, and bound state — all with no GPU.

use stickshift_math::glam::{Mat4, Vec3};
use stickshift_math::Transform;
use stickshift_render_api::{
    ColorSpace, FrameRecorder, MaterialParams, MeshData, MeshHandle, NullRenderer, OverlayQuad,
    PipelineDescriptor, PipelineHandle, PipelineKind, ProtocolViolation, RenderDevice, TextureData,
    TextureHandle, VertexLayout,
};

/// A one-triangle mesh on `kind`'s vertex layout: something the Metal backend
/// would actually draw with that pipeline.
fn make_mesh_for(r: &mut NullRenderer, kind: PipelineKind) -> MeshHandle {
    let layout = kind.vertex_layout();
    let vertices = vec![0u8; 3 * layout.stride as usize];
    r.create_mesh(&MeshData {
        vertices: &vertices,
        indices: &[0, 1, 2],
        layout,
    })
}

fn make_mesh(r: &mut NullRenderer) -> MeshHandle {
    make_mesh_for(r, PipelineKind::VertexColor)
}

fn make_pipeline(r: &mut NullRenderer) -> PipelineHandle {
    r.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor))
}

fn make_texture(r: &mut NullRenderer) -> stickshift_render_api::TextureHandle {
    r.create_texture(&TextureData {
        width: 1,
        height: 1,
        rgba8: &[255, 255, 255, 255],
        color_space: ColorSpace::Srgb,
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
    assert_eq!(draws[0].model, t_a.to_matrix());
    assert_eq!(draws[0].material, mat_a);

    assert_eq!(draws[1].mesh, mesh_b);
    assert_eq!(draws[1].pipeline, Some(pipeline));
    assert_eq!(draws[1].texture, Some(tex));
    assert_eq!(draws[1].model, t_b.to_matrix());
    assert_eq!(draws[1].material, mat_b);
}

#[test]
fn a_matrix_draw_records_its_matrix_exactly() {
    // A sheared model matrix (no position/rotation/scale can express it) must
    // reach the draw unchanged.
    let mut r = NullRenderer::new();
    let mesh = make_mesh(&mut r);
    let pipeline = make_pipeline(&mut r);
    let sheared = Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0))
        * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4);
    r.begin_frame();
    r.set_pipeline(pipeline);
    r.draw_mesh_matrix(mesh, sheared, &MaterialParams::default());
    r.submit();
    assert_eq!(r.draws()[0].model, sheared);
    assert!(r.protocol_violations().is_empty());
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

#[test]
fn correct_use_records_no_protocol_violations() {
    let mut r = NullRenderer::new();
    let mesh = make_mesh(&mut r);
    let pipeline = make_pipeline(&mut r);
    r.begin_frame();
    r.set_pipeline(pipeline);
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    r.draw_overlay_quad(&OverlayQuad::solid([0.0, 0.0, 1.0, 1.0], [1.0; 4]));
    r.submit();
    assert!(r.protocol_violations().is_empty());
    assert_eq!((r.draw_count(), r.overlay_quad_count()), (1, 1));
}

#[test]
fn a_double_destroy_does_not_hide_a_live_mesh() {
    let mut r = NullRenderer::new();
    let a = make_mesh(&mut r);
    let _b = make_mesh(&mut r);
    r.destroy_mesh(a);
    r.destroy_mesh(a);
    assert_eq!(r.live_mesh_count(), 1, "b is still live");
    assert_eq!(
        r.protocol_violations(),
        &[ProtocolViolation::UnknownMesh(a)]
    );
}

#[test]
fn destroying_unknown_handles_is_reported_not_a_panic() {
    let mut r = NullRenderer::new();
    r.destroy_mesh(MeshHandle(7));
    r.destroy_texture(TextureHandle(7));
    r.destroy_pipeline(PipelineHandle(7));
    assert_eq!(
        (
            r.live_mesh_count(),
            r.live_texture_count(),
            r.live_pipeline_count()
        ),
        (0, 0, 0)
    );
    assert_eq!(
        r.protocol_violations(),
        &[
            ProtocolViolation::UnknownMesh(MeshHandle(7)),
            ProtocolViolation::UnknownTexture(TextureHandle(7)),
            ProtocolViolation::UnknownPipeline(PipelineHandle(7)),
        ]
    );
}

#[test]
fn draws_the_metal_backend_drops_are_not_counted() {
    let mut r = NullRenderer::new();
    let mesh = make_mesh(&mut r);
    let textured = make_mesh_for(&mut r, PipelineKind::Textured);
    let empty = r.create_mesh(&MeshData {
        vertices: &[],
        indices: &[],
        layout: PipelineKind::VertexColor.vertex_layout(),
    });
    let wrong_layout = r.create_mesh(&MeshData {
        vertices: &[0; 36],
        indices: &[0, 0, 0],
        layout: VertexLayout::default(),
    });
    let identity = Transform::identity();
    let material = MaterialParams::default();

    // Before any frame is open.
    r.draw_mesh(mesh, &identity, &material);
    r.draw_overlay_quad(&OverlayQuad::solid([0.0; 4], [1.0; 4]));

    r.begin_frame();
    // A textured mesh on the default (untextured) pipeline, an empty mesh and a
    // layout no pipeline draws.
    r.draw_mesh(textured, &identity, &material);
    r.draw_mesh(empty, &identity, &material);
    r.draw_mesh(wrong_layout, &identity, &material);
    r.destroy_mesh(mesh);
    r.draw_mesh(mesh, &identity, &material);
    r.submit();

    assert_eq!(r.draw_count(), 0);
    assert_eq!(r.overlay_quad_count(), 0);
    assert_eq!(
        r.protocol_violations(),
        &[
            ProtocolViolation::OutsideFrame,
            ProtocolViolation::OutsideFrame,
            ProtocolViolation::MeshPipelineMismatch(textured),
            ProtocolViolation::EmptyMesh(empty),
            ProtocolViolation::MeshPipelineMismatch(wrong_layout),
            ProtocolViolation::UnknownMesh(mesh),
        ]
    );
}

#[test]
fn frame_protocol_mistakes_are_no_ops() {
    let mut r = NullRenderer::new();
    r.submit();
    r.begin_frame();
    r.begin_frame();
    r.set_pipeline(PipelineHandle(3));
    r.bind_texture(TextureHandle(3));
    r.submit();
    assert_eq!((r.frames_begun(), r.frames_submitted()), (1, 1));
    assert!(!r.frame_open());
    assert_eq!(
        r.protocol_violations(),
        &[
            ProtocolViolation::SubmitWithoutFrame,
            ProtocolViolation::BeginWhileOpen,
            ProtocolViolation::UnknownPipeline(PipelineHandle(3)),
            ProtocolViolation::UnknownTexture(TextureHandle(3)),
        ]
    );
}

#[test]
fn set_view_projection_is_recorded_and_readable() {
    use stickshift_camera::Camera;

    let mut r = NullRenderer::new();

    // No camera pushed yet.
    assert_eq!(r.view_projection(), None);

    // Push a camera's view-projection before the first draw, as the engine loop
    // does, and read it back.
    let mut cam = Camera::new(16.0 / 9.0);
    cam.set_position(Vec3::new(0.0, 5.0, 10.0));
    cam.set_target(Vec3::ZERO);
    let vp = cam.view_projection_matrix();

    r.begin_frame();
    r.set_view_projection(vp);
    assert_eq!(r.view_projection(), Some(vp));
    r.submit();
}

#[test]
fn view_projection_is_sticky_across_frames() {
    use stickshift_math::glam::Mat4;

    let mut r = NullRenderer::new();

    // Engine pushes the camera before the game opens its frame; the game's
    // begin_frame must NOT clear it (sticky seam contract).
    r.set_view_projection(Mat4::IDENTITY);
    r.begin_frame();
    assert_eq!(
        r.view_projection(),
        Some(Mat4::IDENTITY),
        "begin_frame must not clear the sticky view-projection"
    );
    r.submit();

    // It persists into the next frame until replaced.
    r.begin_frame();
    assert_eq!(r.view_projection(), Some(Mat4::IDENTITY));
    r.submit();
}

#[test]
fn camera_position_is_recorded_and_sticky() {
    let mut r = NullRenderer::new();

    // Nothing pushed yet.
    assert_eq!(r.camera_position(), None);

    let eye = Vec3::new(0.0, 6.0, 12.0);
    r.set_camera_position(eye);
    r.begin_frame();
    assert_eq!(
        r.camera_position(),
        Some(eye),
        "begin_frame must not clear the sticky camera position"
    );
    r.submit();

    // Survives the frame boundary, like the view-projection it accompanies.
    r.begin_frame();
    assert_eq!(r.camera_position(), Some(eye));
    r.submit();
}

#[test]
fn sun_sky_is_recorded_sticky_and_assertable() {
    use stickshift_render_api::SunSky;

    let mut r = NullRenderer::new();

    // Nothing pushed yet: the backend would light the frame with its own default.
    assert_eq!(r.sun_sky(), None);

    // A game's own choice of sun — here a low western one — pushed once at load.
    let sun = SunSky {
        sun_elevation_deg: 32.0,
        sun_azimuth_deg: 284.0,
        sun_color: [1.0, 0.96, 0.88],
        sun_intensity: 1.15,
        sky_fill: 0.22,
        sky_zenith_color: [0.12, 0.28, 0.55],
        sky_horizon_color: [0.62, 0.68, 0.76],
    };
    r.set_sun_sky(&sun);

    r.begin_frame();
    assert_eq!(
        r.sun_sky(),
        Some(sun),
        "begin_frame must not clear the sticky sun/sky"
    );
    r.submit();

    // Still in effect for the next frame — a fixed sun is pushed once, not every
    // frame — and the recorded angles imply the light vector the shader will use.
    r.begin_frame();
    let recorded = r.sun_sky().expect("sun pushed above");
    assert!(recorded.toward_sun().x < 0.0, "the sun is in the west");
    assert!(recorded.direction().y < 0.0, "its light travels downward");
    r.submit();
}
