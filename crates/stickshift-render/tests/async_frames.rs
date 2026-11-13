// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The non-blocking frame path: `try_begin_frame`, `submit_async`, completion
//! counting, read-backs, `discard_frame`, and per-frame HUD buffers.
//!
//! Tests that could hang on a regression run on a worker thread under a
//! timeout, so a deadlock fails the test instead of the whole suite. All of
//! them skip when no Metal device is available.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use kaman_render::{FrameSkip, MetalRenderer};
use kaman_render_api::{FrameRecorder, OverlayQuad};

mod common;

const WIDTH: u32 = 32;
const HEIGHT: u32 = 32;

fn renderer() -> Option<MetalRenderer> {
    let r = MetalRenderer::new_offscreen(WIDTH, HEIGHT);
    if r.is_none() {
        common::no_gpu();
    }
    r
}

/// Run `f` on a worker thread; fail if it takes longer than `secs`.
fn with_timeout<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("timed out: the non-blocking frame path blocked")
}

/// Spin (with short sleeps) until `done()` or the deadline.
fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    done()
}

/// A full-height bar `x` pixels from the left, so each variant is distinct.
fn bar(x: f32) -> OverlayQuad {
    OverlayQuad::solid([x, 0.0, 4.0, HEIGHT as f32], [1.0, 1.0, 1.0, 1.0])
}

#[test]
fn try_begin_frame_reports_an_open_frame_instead_of_replacing_it() {
    let Some(mut r) = renderer() else { return };
    assert_eq!(r.try_begin_frame(), Ok(()));
    assert_eq!(r.try_begin_frame(), Err(FrameSkip::AlreadyOpen));
    assert_eq!(r.submit_async(), Some(1));
    assert_eq!(r.submit_async(), None, "no frame is open any more");
}

#[test]
fn the_non_blocking_path_never_blocks_and_every_frame_completes() {
    let result = with_timeout(30, || {
        let mut r = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;
        let mut opened = 0u64;
        let mut skipped = 0u64;
        for _ in 0..200 {
            match r.try_begin_frame() {
                Ok(()) => {
                    opened += 1;
                    let number = r.submit_async().expect("a frame was open");
                    assert_eq!(number, opened, "frame numbers count submitted frames");
                }
                Err(FrameSkip::FramesInFlight) => skipped += 1,
                Err(other) => panic!("unexpected skip {other:?}"),
            }
            assert!(r.frames_in_flight() <= kaman_render::backend::MAX_FRAMES_IN_FLIGHT);
        }
        assert!(
            wait_until(|| r.completed_frame_count() == r.submitted_frame_count()),
            "{} of {} frames completed",
            r.completed_frame_count(),
            r.submitted_frame_count()
        );
        Some((opened, skipped, r.frames_in_flight()))
    });
    let Some((opened, skipped, in_flight)) = result else {
        common::no_gpu();
        return;
    };
    assert!(opened > 0, "no frame was ever opened ({skipped} skipped)");
    assert_eq!(opened + skipped, 200);
    assert_eq!(in_flight, 0, "every permit came back");
}

#[test]
fn a_readback_holds_its_own_frames_pixels() {
    let Some(mut reference) = renderer() else {
        return;
    };
    reference.begin_frame();
    reference.draw_overlay_quad(&bar(6.0));
    reference.submit();
    let expected = reference.read_pixels().expect("offscreen read-back");

    let Some(mut r) = renderer() else { return };
    assert!(r.request_readback().is_none(), "no frame is open");
    r.try_begin_frame().expect("a free frame slot");
    r.draw_overlay_quad(&bar(6.0));
    let readback = r.request_readback().expect("offscreen frame");
    assert!(r.request_readback().is_none(), "one read-back per frame");
    r.submit_async();
    assert!(
        wait_until(|| readback.is_complete()),
        "the read-back never completed"
    );

    let mut pixels = vec![0u8; readback.byte_len()];
    assert!(readback.copy_to(&mut pixels));
    assert_eq!(pixels, expected);
    assert!(
        !readback.copy_to(&mut [0u8; 4]),
        "a too-short destination copies nothing"
    );
}

#[test]
fn back_to_back_frames_each_draw_their_own_hud() {
    // Each in-flight frame has its own HUD vertex buffer; with a single shared
    // buffer a later frame's HUD could overwrite an earlier frame's vertices
    // before the GPU drew them.
    let positions = [2.0, 12.0, 22.0];
    let mut expected = Vec::new();
    for &x in &positions {
        let Some(mut reference) = renderer() else {
            return;
        };
        reference.begin_frame();
        reference.draw_overlay_quad(&bar(x));
        reference.submit();
        expected.push(reference.read_pixels().expect("offscreen read-back"));
    }

    let Some(mut r) = renderer() else { return };
    let mut readbacks = Vec::new();
    for &x in &positions {
        r.try_begin_frame().expect("a free frame slot");
        r.draw_overlay_quad(&bar(x));
        readbacks.push(r.request_readback().expect("offscreen frame"));
        r.submit_async();
    }
    for (i, readback) in readbacks.iter().enumerate() {
        assert!(
            wait_until(|| readback.is_complete()),
            "frame {i} never completed"
        );
        let mut pixels = vec![0u8; readback.byte_len()];
        assert!(readback.copy_to(&mut pixels));
        assert_eq!(pixels, expected[i], "frame {i} drew another frame's HUD");
    }
}

#[test]
fn discarding_a_frame_returns_its_permit() {
    let result = with_timeout(30, || {
        let mut r = MetalRenderer::new_offscreen(WIDTH, HEIGHT)?;
        for _ in 0..10 {
            r.try_begin_frame()
                .expect("discarded frames hold no permit");
            r.draw_overlay_quad(&bar(0.0));
            r.discard_frame();
        }
        r.discard_frame(); // without an open frame: a no-op
        Some((r.frames_in_flight(), r.submitted_frame_count()))
    });
    match result {
        None => common::no_gpu(),
        Some((in_flight, submitted)) => {
            assert_eq!(in_flight, 0);
            assert_eq!(submitted, 0, "a discarded frame is not submitted");
        }
    }
}
