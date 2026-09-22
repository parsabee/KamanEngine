// Build script for kaman-render.
//
// The DEFAULT build needs no toolchain here: shaders compile at runtime via
// `new_library_with_source` (see src/backend.rs). This script only does work when
// the `precompiled-shaders` feature is enabled, in which case it compiles the MSL
// sources to a `.metallib` ahead of time using the Metal toolchain.
//
// The Metal shader compiler ships with **full Xcode**, NOT the Command Line Tools,
// so this path is gated behind the feature and fails with a clear message when the
// toolchain is absent. See KE-0107 and `scripts/preflight.sh --ios`.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Feature off (the default): nothing to do — runtime shader compilation is used.
    if std::env::var_os("CARGO_FEATURE_PRECOMPILED_SHADERS").is_none() {
        return;
    }

    println!("cargo:rerun-if-changed=shaders/rasterization.metal");
    println!("cargo:rerun-if-changed=build.rs");

    // Select the SDK from the build target. macOS is supported today; iOS is wired
    // here for Phase 3 (it will need the iphoneos SDK + rustup iOS targets).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let sdk = match target_os.as_str() {
        "macos" => "macosx",
        "ios" => "iphoneos",
        other => panic!(
            "precompiled-shaders: unsupported target_os `{other}` — KamanEngine is Apple-only"
        ),
    };

    // Command Line Tools do NOT include the Metal shader compiler; only full Xcode does.
    // Fail clearly instead of emitting a confusing xcrun error mid-compile.
    let metal_available = Command::new("xcrun")
        .args(["-sdk", sdk, "metal", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !metal_available {
        panic!(
            "the `precompiled-shaders` feature requires the Metal toolchain (full Xcode); \
             `xcrun -sdk {sdk} metal` was not found.\n\
             Install Xcode and verify with `./scripts/preflight.sh --ios`, or build without \
             this feature to compile shaders at runtime."
        );
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let air = out_dir.join("rasterization.air");
    let metallib = out_dir.join("rasterization.metallib");

    // rasterization.metal -> .air -> .metallib
    run(Command::new("xcrun")
        .args(["-sdk", sdk, "metal", "-c", "shaders/rasterization.metal", "-o"])
        .arg(&air));
    run(Command::new("xcrun")
        .args(["-sdk", sdk, "metallib"])
        .arg(&air)
        .arg("-o")
        .arg(&metallib));

    // Hand the compiled library path to the crate (src/backend.rs include_bytes! it).
    println!(
        "cargo:rustc-env=KAMAN_RASTER_METALLIB={}",
        metallib.display()
    );
}

fn run(cmd: &mut Command) {
    let status = cmd.status().expect("failed to spawn xcrun");
    assert!(status.success(), "shader compile step failed: {cmd:?}");
}
