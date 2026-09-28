// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Render quality settings through the seam (KE-0408).
//!
//! These pin what `RenderDevice::set_render_settings` and `resize_surface` do to
//! the real Metal backend: the fog scales with the draw distance, the shadow range
//! is a fraction of it, the shadow tier re-sizes (or skips) the shadow map, and
//! the UI scale is reported back. Like the other backend tests they **skip** when
//! no Metal device is available.

use kaman_render::shadow::{shadow_distance_for_fog, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE_LOW};
use kaman_render::MetalRenderer;
use kaman_render_api::{FrameRecorder, RenderDevice, RenderSettings, ShadowQuality};

/// Offscreen render size.
const SIZE: u32 = 64;

fn renderer() -> Option<MetalRenderer> {
    let r = MetalRenderer::new_offscreen(SIZE, SIZE);
    if r.is_none() {
        eprintln!("skipping: no Metal device (GPU-less runner)");
    }
    r
}

/// The fog's opaque depth (the draw distance) in the light block right now.
fn draw_distance(r: &MetalRenderer) -> f32 {
    let l = r.light_for_test();
    shadow_distance_for_fog(l.fog_start, l.fog_density)
}

#[test]
fn defaults_leave_the_look_unchanged() {
    let Some(mut r) = renderer() else { return };
    let before = r.light_for_test();
    r.set_render_settings(&RenderSettings::default());
    let after = r.light_for_test();
    assert_eq!(after.fog_start, before.fog_start);
    assert_eq!(after.fog_density, before.fog_density);
    assert_eq!(after.shadow_strength, before.shadow_strength);
    assert_eq!(r.shadow_distance(), None);
    assert_eq!(r.shadow_map_size(), SHADOW_MAP_SIZE);
}

#[test]
fn draw_distance_scale_scales_the_fog_and_does_not_compound() {
    let Some(mut r) = renderer() else { return };
    let base = draw_distance(&r);
    let base_start = r.light_for_test().fog_start;

    for scale in [0.5, 0.75, 1.0, 0.5] {
        r.set_render_settings(&RenderSettings {
            draw_distance_scale: scale,
            ..RenderSettings::default()
        });
        let l = r.light_for_test();
        assert!((l.fog_start - base_start * scale).abs() < 1e-3);
        assert!(
            (draw_distance(&r) - base * scale).abs() < 1e-2,
            "at {scale}x the fog is opaque at {} (base {base})",
            draw_distance(&r)
        );
    }

    // A nonsensical scale falls back to 1.
    r.set_render_settings(&RenderSettings {
        draw_distance_scale: 0.0,
        ..RenderSettings::default()
    });
    assert!((draw_distance(&r) - base).abs() < 1e-2);
}

#[test]
fn shadow_range_is_a_fraction_of_the_scaled_draw_distance() {
    let Some(mut r) = renderer() else { return };
    let base = draw_distance(&r);

    r.set_render_settings(&RenderSettings {
        shadow_distance: Some(0.25),
        draw_distance_scale: 0.5,
        ..RenderSettings::default()
    });
    let expected = 0.25 * 0.5 * base;
    let got = r.shadow_distance().expect("a fixed shadow range");
    assert!((got - expected).abs() < 1e-2, "{got} != {expected}");

    // The frame actually fits that slab.
    r.begin_frame();
    let fitted = r.light_for_test().shadow_distance;
    r.submit();
    assert!((fitted - expected).abs() < 1e-2);

    // Back to matching the draw distance.
    r.set_render_settings(&RenderSettings::default());
    assert_eq!(r.shadow_distance(), None);
}

#[test]
fn shadow_tiers_resize_or_disable_the_map() {
    let Some(mut r) = renderer() else { return };

    r.set_render_settings(&RenderSettings {
        shadows: ShadowQuality::Low,
        ..RenderSettings::default()
    });
    assert_eq!(r.shadow_map_size(), SHADOW_MAP_SIZE_LOW);
    assert_eq!(r.light_for_test().shadow_strength, 1.0);
    r.begin_frame();
    r.submit();

    r.set_render_settings(&RenderSettings {
        shadows: ShadowQuality::Off,
        ..RenderSettings::default()
    });
    assert_eq!(
        r.light_for_test().shadow_strength,
        0.0,
        "off disables shadowing"
    );
    // A frame with the shadow pass skipped still renders.
    r.begin_frame();
    r.submit();
    assert!(r.read_pixels().is_some());

    r.set_render_settings(&RenderSettings::default());
    assert_eq!(r.shadow_map_size(), SHADOW_MAP_SIZE);
    assert_eq!(r.light_for_test().shadow_strength, 1.0);
    assert_eq!(r.render_settings(), RenderSettings::default());
}

#[test]
fn shadows_off_renders_like_casters_off() {
    // Off skips the pass and zeroes the strength; a scene then looks exactly as it
    // does with the pass running but casting nothing. Both are empty scenes
    // here (only the sky), so the frames must be byte-identical.
    let (Some(mut off), Some(mut no_casters)) = (renderer(), renderer()) else {
        return;
    };
    off.set_render_settings(&RenderSettings {
        shadows: ShadowQuality::Off,
        ..RenderSettings::default()
    });
    no_casters.set_shadows_enabled(false);
    for r in [&mut off, &mut no_casters] {
        r.begin_frame();
        r.submit();
    }
    assert_eq!(off.read_pixels(), no_casters.read_pixels());
}

#[test]
fn resize_reports_the_ui_scale_and_keeps_the_offscreen_size() {
    let Some(mut r) = renderer() else { return };
    assert_eq!(r.surface_scale(), 1.0);
    r.resize_surface(32, 16, 2.0);
    assert_eq!(r.surface_scale(), 2.0);
    // An offscreen target has a fixed size.
    assert_eq!(r.surface_size(), (SIZE, SIZE));
    // A non-positive scale is ignored.
    r.resize_surface(32, 16, 0.0);
    assert_eq!(r.surface_scale(), 2.0);
}
