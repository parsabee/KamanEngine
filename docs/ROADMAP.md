# KamanEngine Roadmap

KamanEngine is a lean, **rasterization-only** Metal graphics engine for Apple Silicon, written in
Rust and usable from other languages through a C API. The host application owns the window or
view, the run loop and input; the engine renders.

Open work is tracked as [GitHub issues](https://github.com/parsabee/KamanEngine/issues). Release
history is in [CHANGELOG.md](../CHANGELOG.md).

## v0.1 — macOS library

### `v0.1.0-beta.1`

- Remove the in-repo ticket system and the old phase plan; track work in GitHub issues.
- Make the docs and API docs match what the code actually does.
- **C API:** build the engine as a C library (static and dynamic) with a generated `kaman.h`
  header, a renderer created from a host-supplied `CAMetalLayer`, and errors instead of panics at
  the boundary. CI compiles and runs a C program against the header.
- A test and bug-fix pass over everything shipped so far.
- **Correct colour and normals:** colour textures decoded from sRGB, flat normals for glTF meshes
  that have none, the inverse-transpose normal matrix under non-uniform scale, and exact glTF
  node matrices.

The ray tracer was removed for v0.1: the engine is rasterization-only.

### `v0.1.0`

Everything in the beta, plus:

- **Precompiled shaders:** compile the Metal shaders to a `.metallib` at build time and embed it,
  so the library never compiles shaders at runtime (building then needs full Xcode).
- **Benchmarking:** GPU frame timing and a benchmark scene, with results recorded on real
  hardware, and any fixes they call for.

## v0.2 — iOS

Release the library for iOS as well as macOS: iOS build targets, an XCFramework, a Swift sample
app, tile-memory (TBDR) tuning, compressed textures, precompiled shaders for the iOS SDKs and
on-device performance validation. Tracked in
[#2 iOS support (v0.2)](https://github.com/parsabee/KamanEngine/issues/2) and its sub-issues.

## Not planned

- **A scene editor.** Not part of this repository.
- **Ray tracing.** The engine is rasterization-only.
