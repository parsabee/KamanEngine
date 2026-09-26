// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Drivable sun + per-frame light upload guards (KE-0406).
//!
//! These pin the three properties that make the sun *reachable* and *safe*, all
//! against the real Metal backend and without needing to read a pixel back:
//!
//! 1. **The sun arrives from above the seam.** A [`SunSky`] pushed with
//!    `set_sun_sky` becomes the exact bytes the fragment shader reads, and the
//!    light direction is the one `SunSky::direction()` derives — the single source
//!    of truth the sky's sun disc also uses. Before KE-0406 the light was written
//!    once at construction and nothing above the seam could change it.
//! 2. **It is honoured per frame, in a rotating ring slot.** The light is uploaded
//!    every frame into one slot per in-flight frame (`0, 256, 512, 0, …`), so the
//!    CPU never rewrites bytes a queued frame is still reading — the same argument
//!    as the per-draw uniform ring (KE-0105).
//! 3. **Per-frame upload costs no allocation.** The ring is created once, so the
//!    hot path's `new_buffer*` delta stays 0 (KR1.2).
//!
//! Like the other backend tests these **skip** when no Metal device is available
//! (GPU-less CI) and run + assert on a real Mac.

use kaman_math::glam::Vec3;
use kaman_render::backend::MAX_FRAMES_IN_FLIGHT;
use kaman_render::MetalRenderer;
use kaman_render_api::{FrameRecorder, SunSky};

/// Offscreen render size.
const WIDTH: u32 = 64;
/// Offscreen render size.
const HEIGHT: u32 = 64;

/// A low western sun, deliberately unlike the default so every field is proven to
/// travel (a value equal to the default would pass even if the push were dropped).
fn western_afternoon_sun() -> SunSky {
    SunSky {
        sun_elevation_deg: 32.0,
        sun_azimuth_deg: 284.0,
        sun_color: [1.0, 0.96, 0.88],
        sun_intensity: 1.15,
        sky_fill: 0.22,
        sky_zenith_color: [0.11, 0.27, 0.56],
        sky_horizon_color: [0.62, 0.67, 0.74],
    }
}

#[test]
fn pushed_sun_becomes_the_light_the_shader_reads() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    // Until something pushes a sun, the backend uses the seam's own default.
    let default_light = r.light_for_test();
    assert_eq!(
        default_light.direction,
        SunSky::default().direction().to_array()
    );

    let sun = western_afternoon_sun();
    r.set_sun_sky(&sun);

    let light = r.light_for_test();
    // The property the sun disc depends on: the shaded direction *is*
    // `SunSky::direction()`, so the disc drawn at that direction cannot disagree
    // with the lighting.
    assert_eq!(light.direction, sun.direction().to_array());
    assert_eq!(light.color, sun.sun_color);
    assert_eq!(light.ambient_intensity, sun.sky_fill);
    assert_eq!(light.diffuse_intensity, sun.sun_intensity);
    assert_eq!(light.sky_top_color, sun.sky_zenith_color);
    assert_eq!(light.sky_horizon_color, sun.sky_horizon_color);

    // And it really is a different sun than the default, so the assertions above
    // are not passing by coincidence.
    assert_ne!(light.direction, default_light.direction);
}

#[test]
fn camera_reaches_the_fragment_shader_and_the_sun_is_sticky() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    let sun = western_afternoon_sun();
    let eye = Vec3::new(2.0, 6.0, 14.0);
    r.set_sun_sky(&sun);
    r.set_camera_position(eye);

    // The camera block is folded in as each frame opens, so it is the frame — not
    // the push — that publishes it (KE-0406).
    r.begin_frame();
    assert_eq!(
        r.light_for_test().camera_position,
        eye.to_array(),
        "specular needs the eye position; it must reach the light block"
    );
    r.submit();

    // The sun survives the frame boundary: a game with a fixed sun pushes once at
    // load and every later frame is still lit by it.
    r.begin_frame();
    assert_eq!(r.light_for_test().direction, sun.direction().to_array());
    r.submit();
}

#[test]
fn light_upload_rotates_one_slot_per_in_flight_frame() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    let stride = r.ring_stride_for_test();

    // Run past a full triple-buffer cycle: the light must land in a different slot
    // each frame and only reuse a slot after `MAX_FRAMES_IN_FLIGHT` frames — by
    // which point the semaphore has proven that frame retired.
    let frames = (2 * MAX_FRAMES_IN_FLIGHT) + 1;
    for i in 0..frames {
        let expected = (i % MAX_FRAMES_IN_FLIGHT) * stride;
        assert_eq!(
            r.light_offset_for_test(),
            expected,
            "frame {i}: the light must be written to slot {} of the ring",
            i % MAX_FRAMES_IN_FLIGHT
        );
        assert_eq!(
            r.light_offset_for_test() % 256,
            0,
            "fragment-buffer offsets must be 256-byte aligned on Apple GPUs"
        );
        r.begin_frame();
        r.submit();
    }
}

#[test]
fn per_frame_light_upload_allocates_nothing() {
    let Some(mut r) = MetalRenderer::new_offscreen(WIDTH, HEIGHT) else {
        eprintln!("skipping: no Metal device (GPU-less runner)");
        return;
    };

    r.set_sun_sky(&western_afternoon_sun());

    // Warm one frame so lazily-created per-frame state (the depth/MSAA textures)
    // exists before measuring steady state.
    r.begin_frame();
    r.submit();

    let before = r.allocation_count();
    for _ in 0..(2 * MAX_FRAMES_IN_FLIGHT) {
        // A per-frame *upload* must not become a per-frame *allocation*: the light
        // is written into the ring created at construction, never a fresh buffer.
        r.set_sun_sky(&western_afternoon_sun());
        r.begin_frame();
        r.submit();
    }
    let delta = r.allocation_count() - before;
    assert_eq!(
        delta, 0,
        "per-frame light upload allocated {delta} buffers; expected 0"
    );
}
