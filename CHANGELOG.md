# Changelog

All notable changes to KamanEngine are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is below
`1.0.0`, and especially for `-alpha` pre-releases, **any release may break the public API**.

## [Unreleased]

## [0.1.0-alpha.1] - YYYY-MM-DD

The first public pre-release: a source release of the engine workspace and its reference game,
the playable demo. macOS on Apple Silicon is the supported platform. All crates share the
workspace version and are not published to crates.io.

### Added

- **Workspace and CI (Phase 0).** A Cargo workspace of focused `kaman-*` crates plus
  `games/playable-demo`, built with Rust 1.91 (pinned in `rust-toolchain.toml`; MSRV enforced
  through `rust-version`). A macOS CI oracle builds, tests, lints (clippy `-D warnings`), checks
  rustdoc, lints the ticket format and runs the headless smoke oracle. `scripts/preflight.sh`
  checks the required toolchain. The engine was migrated from an earlier prototype, with its
  identifiers removed from shipped code.
- **Engine/game boundary and loop (Phases 1-2).** The `Game` trait and `EngineCtx` in
  `kaman-core`; a fixed-timestep loop that drives both the windowed (winit) app and a headless
  driver, with `Game::update` on a fixed cadence.
- **Render seam and raw-Metal renderer (Phase 1).** `kaman-render-api` defines the
  backend-agnostic `RenderDevice` / `FrameRecorder` traits, opaque handles and a `NullRenderer`
  test double; it never depends on `metal`, and CI enforces that. `kaman-render` implements the
  seam in raw Metal, with persistent mesh buffers behind a generational handle registry, a
  uniform ring (no per-frame allocations), triple-buffered frames in flight, offscreen rendering
  with pixel read-back, and an off-by-default `raytracer` feature.
- **ECS, physics and streaming (Phase 2).** `kaman-ecs` (a `hecs` wrapper and shared
  components), `kaman-physics` (a `rapier3d` wrapper with a body/collider removal API),
  `kaman-scene` (world streaming with spawn-ahead / despawn-behind and a floating-origin rebase)
  and `kaman-camera` (perspective camera and chase controller).
- **Look and content (Phase 4).**
  - Static glTF (`.gltf` / `.glb`) import and an asset cache in `kaman-assets`, with a flexible
    vertex layout.
  - Base-color textures with mipmaps (RGBA8; ASTC is behind an off-by-default feature).
  - A modern look stack: MSAA, a gradient sky with a drivable sun disc, specular lighting,
    ground-hugging horizon fog and ACES tonemapping.
  - Real fitted shadow maps with PCF filtering (4096² at the High setting). The shadow maps
    follow the draw distance.
  - A 2D HUD overlay seam with SDF text.
  - Audio in `kaman-audio` over `kira`: load-once sounds, one-shots, looping music and master
    volume.
  - `kaman-perf` frame-timing instrumentation with an injectable clock.
- **The playable demo (Phase 7).** `games/playable-demo`, an endless three-lane freeway runner
  and the reference example for building a game on the engine's public API. It has discrete lane
  control, title, game-over and replay states, and a score. Content: CC0 Quaternius car models,
  CC0 Kenney City Kit buildings, a textured asphalt road with yellow solid edge lines, a city
  skyline backdrop, guardrails and hill terrain. The draw distance and fog reach are three times
  the original. Also a HUD that scales with the render resolution, driving music and a crash
  sound effect.
- **Graphics settings and a native macOS Graphics menu (KE-0408).**
  - The windowed runner adds a **Graphics** menu to the macOS menu bar by default. It offers:
    - Shadows: Off / Low / High.
    - Shadow Distance: Match Draw Distance / Medium / Near.
    - Draw Distance: Near / Medium / Far.
    - Resolution: 50% / 75% / 100% of native pixels.
    - Enter/Exit Full Screen (⌃⌘F) and Reset to Defaults.
  - Choices persist across launches in `NSUserDefaults`.
  - Games can turn the menu off with `RunConfig { native_settings_menu: false, .. }` and drive the
    same public `GraphicsSettings` model from their own UI, via
    `EngineCtx::graphics_settings` / `set_graphics_settings` and the
    `Game::graphics_settings_changed` hook.
  - The render seam gains `RenderSettings`, `set_render_settings` and `resize_surface`.
- **Documentation.** Every engine crate compiles under `#![deny(missing_docs)]`, and the rustdoc
  API reference is published to <https://parsabee.github.io/KamanEngine/>. Guides:
  `docs/GETTING_STARTED.md` (build your own game), `docs/PLAYABLE_DEMO.md` (the demo as a
  reference example), `docs/DESIGN.md` (Mermaid diagrams), `docs/ARCHITECTURE.md`,
  `docs/ROADMAP.md` and `docs/INTEGRATION.md`.
- **Testing.** 316 tests: unit tests throughout; golden pixel-hash tests that pin the renderer's
  output (skipped when no Metal device is available); and a headless smoke oracle
  (`cargo run -p playable-demo -- --smoke`) that runs 120 frames with no GPU.

### Known limitations

- **macOS only.** iOS bring-up (Phase 3) has not started. It needs full Xcode and the
  `aarch64-apple-ios` rustup targets.
- **Shaders compile at runtime.** Precompiled `.metallib` shaders (KE-0107) are blocked on full
  Xcode, because the Command Line Tools do not ship the `metal` / `metallib` compiler.
- **Pre-release API.** The public API is unstable and may change in any release. The crates are
  not published to crates.io.
- **Source release only.** No prebuilt binaries. The demo resolves its assets through
  compile-time `CARGO_MANIFEST_DIR` paths, so it must be built and run from a clone of the
  repository (`cargo run -p playable-demo`).
- **Phases 5 and 6 have not started.** Phase 5 was re-scoped from the KamanScript scripting
  layer to a native macOS scene editor for Apple Silicon. Phase 6 is the App Store release.
  Physics uses `rapier3d` for now; a custom arcade-physics layer is planned.
- **The Graphics menu is macOS-only.** It is installed only in windowed runs. Headless runs,
  tests and `--smoke` never read or write the saved settings.
- **Demo asset licences.** The demo's third-party models and textures are CC0. The music and
  crash WAVs were generated for this project and dedicated CC0 1.0. The HUD font is OFL. See
  `games/playable-demo/README.md`.

[Unreleased]: https://github.com/parsabee/KamanEngine/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/parsabee/KamanEngine/releases/tag/v0.1.0-alpha.1
