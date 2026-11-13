// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Normals under non-uniform scale.
//!
//! A sloped quad is drawn scaled `(1, 3, 1)` and rotated. The renderer must
//! light it exactly like the same geometry baked on the CPU with correctly
//! transformed normals (the inverse-transpose of the model's 3×3) and drawn at
//! the identity. A bake that transforms normals by the model itself, as the
//! shaders used to, must come out visibly different, so the comparison cannot
//! pass by accident. No golden image: both sides are rendered here.
//!
//! Skips (prints a line, returns) when no Metal device is available, unless
//! `KAMAN_REQUIRE_GPU` is set.

use kaman_camera::Camera;
use kaman_math::glam::{Mat3, Quat, Vec3};
use kaman_math::Transform;
use kaman_render::MetalRenderer;
use kaman_render_api::{
    FrameRecorder, MaterialParams, MeshData, PipelineDescriptor, PipelineKind, RenderDevice,
};

mod common;

/// Offscreen render size.
const WIDTH: u32 = 64;
/// Offscreen render size.
const HEIGHT: u32 = 64;

/// A 45° slope facing `+Y+Z`: corners and their (shared) normal.
fn slope() -> ([Vec3; 4], Vec3) {
    let corners = [
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
    ];
    (corners, Vec3::new(0.0, 1.0, 1.0).normalize())
}

/// The slope's transform: stretched vertically and turned, so its normal
/// depends on whether it is carried correctly.
fn transform() -> Transform {
    Transform {
        position: Vec3::ZERO,
        rotation: Quat::from_rotation_y(0.4),
        scale: Vec3::new(1.0, 3.0, 1.0),
    }
}

/// Pack the slope's corners with one normal into `[pos, normal, colour]` bytes.
fn pack(corners: &[Vec3; 4], normal: Vec3) -> Vec<u8> {
    let colour = [0.8f32, 0.8, 0.8];
    let mut bytes = Vec::new();
    for p in corners {
        for f in p
            .to_array()
            .into_iter()
            .chain(normal.to_array())
            .chain(colour)
        {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

/// Draw `vertices` (one quad) with `model` and return the frame, or `None`
/// without a Metal device.
fn render(vertices: &[u8], model: &Transform) -> Option<Vec<u8>> {
    let mut renderer = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;
    let mut camera = Camera::new(WIDTH as f32 / HEIGHT as f32);
    camera.set_position(Vec3::new(0.0, 0.5, 5.0));
    camera.set_target(Vec3::ZERO);
    let mesh = renderer.create_mesh(&MeshData {
        vertices,
        indices: &[0, 1, 2, 0, 2, 3],
        layout: PipelineKind::VertexColor.vertex_layout(),
    });
    let pipeline = renderer.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor));
    renderer.set_view_projection(camera.view_projection_matrix());
    renderer.set_camera_position(camera.position());
    renderer.begin_frame();
    renderer.set_pipeline(pipeline);
    renderer.draw_mesh(mesh, model, &MaterialParams::default());
    renderer.submit();
    renderer.read_pixels()
}

/// The share of pixels whose channels differ by more than `threshold`.
fn differing_share(a: &[u8], b: &[u8], threshold: u8) -> f32 {
    let differing = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .any(|(p, q)| p.abs_diff(*q) > threshold)
        })
        .count();
    differing as f32 / (a.len() / 4) as f32
}

#[test]
fn non_uniform_scale_lights_like_correctly_transformed_normals() {
    let (corners, normal) = slope();
    let model = transform().to_matrix();
    let m3 = Mat3::from_mat4(model);
    let baked_corners = corners.map(|p| model.transform_point3(p));
    let correct = (m3.inverse().transpose() * normal).normalize();
    let naive = (m3 * normal).normalize();

    let (Some(scaled), Some(baked_correct), Some(baked_naive)) = (
        render(&pack(&corners, normal), &transform()),
        render(&pack(&baked_corners, correct), &Transform::identity()),
        render(&pack(&baked_corners, naive), &Transform::identity()),
    ) else {
        common::no_gpu();
        return;
    };

    // Same geometry, same normals: only rounding at the quad's edges differs.
    let against_correct = differing_share(&scaled, &baked_correct, 3);
    assert!(
        against_correct < 0.02,
        "{:.1}% of pixels differ from the correctly-normalled bake",
        against_correct * 100.0
    );
    // The model-transformed normals light the face differently.
    let against_naive = differing_share(&baked_correct, &baked_naive, 3);
    assert!(
        against_naive > 0.05,
        "the naive bake differs on only {:.1}% of pixels, so this test can't tell",
        against_naive * 100.0
    );
}
