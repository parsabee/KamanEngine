// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Give `libkaman.dylib` an `@rpath` install name on macOS, so hosts that link
//! it find it through their own rpath (an app bundle's Frameworks directory, or
//! `-Wl,-rpath,<lib dir>`) instead of the build machine's path.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libkaman.dylib");
    }
}
