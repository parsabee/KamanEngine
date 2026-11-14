// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Helpers shared by the GPU integration tests: what a test does when there is
//! no Metal device, and when the golden images are being re-recorded.

// Each test binary compiles this module and uses only part of it.
#![allow(dead_code)]

use std::io::Write;

/// Report that the calling test cannot run because there is no Metal device.
///
/// The notice goes straight to the stderr handle, past the test harness's output
/// capture, so a plain `cargo test` shows which tests did not run. With
/// `STICKSHIFT_REQUIRE_GPU=1` (CI sets it) a missing device **fails** the test
/// instead: a runner that lost its GPU must not turn the render suite green with
/// nothing checked.
pub fn no_gpu() {
    let test = std::thread::current()
        .name()
        .unwrap_or("a GPU test")
        .to_owned();
    assert!(
        !env_flag("STICKSHIFT_REQUIRE_GPU"),
        "{test}: no Metal device, and STICKSHIFT_REQUIRE_GPU is set"
    );
    let _ = writeln!(
        std::io::stderr(),
        "SKIPPED {test}: no Metal device (set STICKSHIFT_REQUIRE_GPU=1 to fail instead)"
    );
}

/// Whether `BLESS` asks to re-record the golden images. Only `1` or `true` do,
/// so `BLESS=0` (or an empty value) still checks them.
pub fn blessing() -> bool {
    env_flag("BLESS")
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}
