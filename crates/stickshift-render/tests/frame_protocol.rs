// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Frame-protocol and input-validation guards for the Metal backend.
//!
//! Each test pins a caller error the seam says must not corrupt state:
//!
//! - a second `begin_frame` without `submit` is a no-op (it used to leak a
//!   frames-in-flight permit, and the third such call deadlocked);
//! - overlay quads recorded outside a frame are ignored (they used to stack into
//!   the next frame);
//! - a mesh whose vertex layout does not match the selected pipeline is rejected
//!   (the GPU would otherwise fetch past the end of its vertex buffer);
//! - short texture data and malformed meshes are reported by the `try_*` paths;
//! - the fallible constructors reject bad sizes and surfaces.
//!
//! Like the other backend tests they skip when no Metal device is available,
//! except the constructor checks that fail before a device is needed.

use std::sync::mpsc;
use std::time::Duration;

use stickshift_math::Transform;
use stickshift_render::{MetalRenderer, RenderError};
use stickshift_render_api::{
    ColorSpace, FrameRecorder, MaterialParams, MeshData, OverlayQuad, PipelineDescriptor,
    PipelineKind, RenderDevice, TextureData, VertexAttribute, VertexFormat, VertexLayout,
};

mod common;

const WIDTH: u32 = 32;
const HEIGHT: u32 = 32;

fn layout(stride: u32, third: VertexFormat) -> VertexLayout {
    let attr = |location, offset, format| VertexAttribute {
        location,
        offset,
        format,
    };
    VertexLayout::new(
        stride,
        vec![
            attr(0, 0, VertexFormat::Float32x3),
            attr(1, 12, VertexFormat::Float32x3),
            attr(2, 24, third),
        ],
    )
}

/// The untextured `[pos, normal, color]` layout (36 bytes).
fn color_layout() -> VertexLayout {
    layout(36, VertexFormat::Float32x3)
}

/// The textured `[pos, normal, uv]` layout (32 bytes).
fn textured_layout() -> VertexLayout {
    layout(32, VertexFormat::Float32x2)
}

/// One triangle's worth of `floats_per_vertex`-float vertices, as bytes.
fn triangle_bytes(floats_per_vertex: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for corner in [[-0.5f32, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]] {
        let mut vertex = vec![0.0f32; floats_per_vertex];
        vertex[..3].copy_from_slice(&corner);
        vertex[5] = 1.0; // normal +Z
        for f in vertex {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    }
    bytes
}

fn pipeline(r: &mut MetalRenderer, kind: PipelineKind) -> stickshift_render_api::PipelineHandle {
    r.create_pipeline(&PipelineDescriptor::new(kind))
}

fn renderer() -> Option<MetalRenderer> {
    let r = MetalRenderer::new_offscreen(WIDTH, HEIGHT);
    if r.is_none() {
        common::no_gpu();
    }
    r
}

#[test]
fn a_second_begin_frame_is_a_no_op_and_never_deadlocks() {
    // Run on a worker so a regression (a leaked permit blocks `begin_frame`
    // forever) fails the test instead of hanging the whole suite.
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
            let _ = done_tx.send(None);
            return;
        };
        for _ in 0..4 {
            // Each iteration opens the frame once and "re-opens" it three times.
            for _ in 0..4 {
                r.begin_frame();
            }
            r.submit();
        }
        let _ = done_tx.send(Some(r.frame_index_for_test()));
    });
    match done_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(None) => common::no_gpu(),
        Ok(Some(frames)) => assert_eq!(frames, 4, "one submitted frame per begin/submit pair"),
        Err(_) => {
            panic!("begin_frame deadlocked: a repeated call leaked a frames-in-flight permit")
        }
    }
}

#[test]
fn overlay_quads_outside_a_frame_are_ignored() {
    let Some(mut clean) = renderer() else { return };
    clean.begin_frame();
    clean.submit();
    let expected = clean.read_pixels().expect("offscreen read-back");

    let Some(mut r) = renderer() else { return };
    let red = OverlayQuad::solid(
        [0.0, 0.0, WIDTH as f32, HEIGHT as f32],
        [1.0, 0.0, 0.0, 1.0],
    );
    // Before the first frame, and after a submitted one: neither may leak into
    // the frame that follows.
    r.draw_overlay_quad(&red);
    r.begin_frame();
    r.submit();
    r.draw_overlay_quad(&red);
    r.begin_frame();
    r.submit();
    assert_eq!(
        r.read_pixels().expect("offscreen read-back"),
        expected,
        "a quad recorded outside a frame was drawn"
    );
}

#[test]
fn a_mesh_on_the_wrong_pipeline_is_rejected() {
    let Some(mut r) = renderer() else { return };
    let untextured = pipeline(&mut r, PipelineKind::VertexColor);
    let textured_mesh = r.create_mesh(&MeshData {
        vertices: &triangle_bytes(8),
        indices: &[0, 1, 2],
        layout: textured_layout(),
    });

    r.begin_frame();
    r.set_pipeline(untextured);
    r.draw_mesh(
        textured_mesh,
        &Transform::identity(),
        &MaterialParams::default(),
    );
    assert_eq!(
        r.rejected_draw_count(),
        1,
        "the mismatched draw was not rejected"
    );
    assert_eq!(
        r.ring_cursor_for_test(),
        0,
        "a rejected draw must not use a ring slot"
    );
    r.submit();
}

#[test]
fn a_mesh_on_its_own_pipeline_is_drawn() {
    let Some(mut r) = renderer() else { return };
    let untextured = pipeline(&mut r, PipelineKind::VertexColor);
    let mesh = r.create_mesh(&MeshData {
        vertices: &triangle_bytes(9),
        indices: &[0, 1, 2],
        layout: color_layout(),
    });
    r.begin_frame();
    r.set_pipeline(untextured);
    r.draw_mesh(mesh, &Transform::identity(), &MaterialParams::default());
    assert_eq!(r.rejected_draw_count(), 0);
    assert_eq!(r.ring_cursor_for_test(), 1);
    r.submit();
}

#[test]
fn try_create_texture_rejects_short_or_oversized_data() {
    let Some(mut r) = renderer() else { return };
    let short = r.try_create_texture(&TextureData {
        width: 1024,
        height: 1024,
        rgba8: &[0; 4],
        color_space: ColorSpace::Srgb,
    });
    assert!(
        matches!(short, Err(RenderError::InvalidTexture(_))),
        "{short:?}"
    );
    let zero = r.try_create_texture(&TextureData {
        width: 0,
        height: 1,
        rgba8: &[],
        color_space: ColorSpace::Srgb,
    });
    assert!(
        matches!(zero, Err(RenderError::InvalidSize { .. })),
        "{zero:?}"
    );
    assert!(r
        .try_create_texture(&TextureData {
            width: 2,
            height: 2,
            rgba8: &[255; 16],
            color_space: ColorSpace::Srgb,
        })
        .is_ok());
}

#[test]
fn create_texture_panics_instead_of_reading_past_short_data() {
    let Some(mut r) = renderer() else { return };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        r.create_texture(&TextureData {
            width: 64,
            height: 64,
            rgba8: &[0; 4],
            color_space: ColorSpace::Srgb,
        })
    }));
    let message = result
        .expect_err("short pixel data must panic, not be read past its end")
        .downcast::<String>()
        .map(|s| *s)
        .unwrap_or_default();
    assert!(message.contains("create_texture"), "{message}");
}

#[test]
fn try_create_mesh_rejects_malformed_meshes() {
    let Some(mut r) = renderer() else { return };
    let mesh = |r: &mut MetalRenderer, vertices: &[u8], indices: &[u32], layout| {
        r.try_create_mesh(&MeshData {
            vertices,
            indices,
            layout,
        })
    };
    let ok = triangle_bytes(9);
    assert!(mesh(&mut r, &ok, &[0, 1, 2], color_layout()).is_ok());
    for (what, result) in [
        (
            "unknown layout",
            mesh(&mut r, &ok, &[0, 1, 2], VertexLayout::default()),
        ),
        (
            "partial vertex",
            mesh(&mut r, &ok[..ok.len() - 4], &[0, 1, 2], color_layout()),
        ),
        (
            "partial triangle",
            mesh(&mut r, &ok, &[0, 1], color_layout()),
        ),
        (
            "index out of range",
            mesh(&mut r, &ok, &[0, 1, 3], color_layout()),
        ),
    ] {
        assert!(
            matches!(result, Err(RenderError::InvalidMesh(_))),
            "{what}: {result:?}"
        );
    }
}

#[test]
fn constructors_reject_bad_sizes_and_surfaces_before_touching_the_gpu() {
    assert!(matches!(
        MetalRenderer::try_new_offscreen(0, 16),
        Err(RenderError::InvalidSize { .. })
    ));
    assert!(matches!(
        MetalRenderer::try_new_offscreen(16, stickshift_render::MAX_TEXTURE_DIMENSION + 1),
        Err(RenderError::InvalidSize { .. })
    ));
    // SAFETY: a null pointer is explicitly allowed (and rejected).
    let null = unsafe { MetalRenderer::try_from_layer(std::ptr::null_mut(), 16, 16, 1.0) };
    assert!(matches!(null, Err(RenderError::InvalidSurface(_))));
}

#[test]
fn try_from_layer_accepts_a_metal_layer_and_rejects_other_objects() {
    use metal::foreign_types::ForeignType;
    use objc::{class, msg_send, sel, sel_impl};

    if metal::Device::system_default().is_none() {
        common::no_gpu();
        return;
    }
    // SAFETY: `NSObject new` returns a live, retained object we release below.
    let object: *mut objc::runtime::Object = unsafe { msg_send![class!(NSObject), new] };
    let not_a_layer = unsafe { MetalRenderer::try_from_layer(object.cast(), 16, 16, 1.0) };
    assert!(matches!(not_a_layer, Err(RenderError::InvalidSurface(_))));
    // SAFETY: balances the `new` above.
    unsafe {
        let _: () = msg_send![object, release];
    }

    let layer = metal::MetalLayer::new();
    // SAFETY: `layer` is a live CAMetalLayer for the duration of the call; the
    // renderer retains it.
    let r = unsafe { MetalRenderer::try_from_layer(layer.as_ptr().cast(), 40, 20, 2.0) }
        .expect("a real CAMetalLayer is accepted");
    assert_eq!(r.surface_size(), (40, 20));
    assert_eq!(r.surface_scale(), 2.0);
}
