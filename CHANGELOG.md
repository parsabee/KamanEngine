# Changelog

All notable changes to KamanEngine are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is below
`1.0.0`, and especially for `-alpha` pre-releases, **any release may break the public API**.

## [Unreleased]

### Added

- **The C API.** A new `kaman-capi` crate builds `libkaman.a` and `libkaman.dylib` with a
  cbindgen-generated `kaman.h` and a Swift `module.modulemap`; the contract is
  [docs/C_API.md](docs/C_API.md). The API is **asynchronous in shape**:
  - calls return at once with a handle or a result code;
  - resources go `PENDING` → `READY` / `FAILED`;
  - completions arrive as events, through `kaman_poll_events` or per-call callbacks that run on
    the host's thread;
  - glTF models are parsed on worker threads, and `kaman_frame_acquire` never waits.

  Under the hood:
  - Handles are typed 64-bit ids with 32-bit generations and are never reissued.
  - A context is used by one thread at a time; an overlapping call returns `KAMAN_ERROR_BUSY`.
  - No panic crosses the boundary.
  - The library is graphics-only: no windowing, audio, ECS or physics crate links into it, and
    CI checks that.
- **A null backend** for the C API (`KAMAN_BACKEND_NULL`), so the API and host-side tests run
  without a GPU. It is the only backend off Apple platforms.
- **C programs in CI.** `crates/kaman-capi/tests/c/smoke.c` and `examples/c/offscreen.c` are
  compiled with `-std=c11 -pedantic -Werror` against the header and run on the macOS runner's
  Metal device (`scripts/test-c-api.sh`). CI also fails if the committed header no longer
  matches the Rust sources (`scripts/gen-header.sh --check`).
- **The release ships the C library.** The tarball and the Homebrew formula now install
  `lib/libkaman.a`, `lib/libkaman.dylib` (install name `@rpath/libkaman.dylib`),
  `include/kaman/kaman.h`, `include/kaman/module.modulemap` and `lib/pkgconfig/kaman.pc`.
  Packaging builds and runs the C programs against the unpacked tarball.
- **`kaman-render`: fallible and non-blocking entry points**, alongside the existing ones:
  - `try_new`, `try_new_offscreen` and `try_from_layer` return a `RenderError` instead of
    panicking; `try_from_layer` takes a `CAMetalLayer` the host owns.
  - `try_create_mesh` and `try_create_texture` validate their input strictly.
  - `try_begin_frame` reports a `FrameSkip` instead of blocking.
  - `submit_async` never waits.
  - `request_readback` returns a `Readback` with a frame's own pixels.
  - `discard_frame`, frame completion counts, and `draw_mesh_matrix`.
- **`PipelineKind`** (`VertexColor`, `Textured`) in `kaman-render-api`: the single source of truth
  for the two built-in pipelines and their vertex layouts.

### Changed

- **CI fails on unformatted code.** A new `cargo fmt --all -- --check` step runs first in the
  macOS CI job, and the whole workspace was reformatted with rustfmt's defaults (a
  formatting-only change). Run `cargo fmt --all` before pushing.
- **Pipelines and materials say what they do.** `PipelineDescriptor` is now
  `{ kind: PipelineKind }`; the shader-name strings it used to carry were ignored by the backend.
  `MaterialParams` keeps only `base_color`.
- **`kaman-render` no longer waits on the CPU for texture mipmaps.** The mip blit runs on the
  same queue as every later frame. Frame opening, submission and the mip blit now run inside
  autorelease pools.
- **`kaman-assets`' ECS spawn helper is behind a default `ecs` feature.** `kaman-render` drops
  its unused `winit` dependency.
- **A new roadmap.** `docs/ROADMAP.md` now lists what v0.1 (beta.1, then 0.1.0) and v0.2 contain,
  replacing the phase/OKR plan. The native scene editor is no longer planned for this repository.
- **The docs match the code.** Docs and doc comments that described things the code does not do
  were corrected, among them:
  - the uniform ring that replaced per-draw uniform buffers, and the two built-in pipelines;
  - how `MaterialParams` is used: only `base_color`, and only by the textured pipeline;
  - that bloom, ASTC transcoding and an iOS path do not exist yet;
  - that CI's macOS runner runs the GPU pixel-hash tests, rather than skipping them;
  - workspace-tree entries (`shaders/`, `platform/`, Ray/AABB/Frustum in `kaman-math`) that do
    not exist;
  - that there is no golden physics-trajectory test or coverage tooling yet.
- **`NullRenderer` accepts exactly the calls the Metal backend acts on.** A draw or HUD quad
  outside a frame, a destroyed or unknown handle, a mesh on the wrong pipeline, an empty mesh, a
  double destroy, `begin_frame` while open and `submit` without a frame are no longer recorded;
  each is listed in the new `protocol_violations()`. `live_*_count` counts live handles, so a
  double destroy no longer hides a leak.
- **GPU tests no longer pass silently without a GPU.** A skipped test prints a visible `SKIPPED`
  line, and with `KAMAN_REQUIRE_GPU=1` (set in CI) a missing Metal device fails it. `BLESS`
  re-records the golden images only when set to `1` or `true`.
- **`ImportError` is `#[non_exhaustive]`** and gains `UnsupportedPrimitiveMode` and
  `InvalidPrimitive`.
- **`$KAMAN_DEMO_ASSETS` is strict.** When set it is the only place the demo looks, and one that
  holds no assets is an error rather than a silent fall-through. Every candidate must hold
  `road.gltf`, so an unrelated `assets/` beside the binary is skipped.
- **`kaman-core` builds warning-free off macOS.**

### Removed

- **The in-repo ticket system.** `tickets/`, `scripts/check-tickets.py` and the CI ticket-format
  lint are gone, and code comments no longer cite ticket IDs, phases or key results. Open work is
  tracked in GitHub issues, starting with
  [#2 iOS support (v0.2)](https://github.com/parsabee/KamanEngine/issues/2).

- **`MaterialParams::metallic` and `roughness`**, and `PipelineDescriptor`'s
  `vertex_shader` / `fragment_shader` / `vertex_layout` fields. No pipeline read any of them.

- **The `raytracer` feature.** KamanEngine is rasterization-only. The off-by-default
  `kaman-render` feature, its `raytracer` module (a CPU `trace_ray` and GPU triangle layouts that
  nothing called) and `shaders/raytracing.metal` (which no build ever compiled or loaded) are
  gone, along with the CI step that built them. Default builds are unchanged.

### Fixed

- **`create_texture` could read past the end of its pixel data.** It took any `rgba8` slice, even
  one shorter than `4 * width * height` bytes. Short data is now rejected: the seam method panics
  with a message, and `try_create_texture` returns an error.
- **Calling `begin_frame` twice without `submit`** leaked a frames-in-flight permit, and the
  third time deadlocked. A second call is now a no-op.
- **HUD quads recorded outside a frame** (before the first frame, after `submit`, or while a frame
  was skipped for want of a drawable) used to stack into the next frame. They are now ignored.
- **A mesh drawn with the other pipeline** made the GPU fetch vertices at the wrong stride, past
  the end of its buffer. Such a draw is now rejected and counted (`rejected_draw_count`).
- **The HUD vertex buffer was shared by all frames in flight,** so a frame could draw the next
  frame's HUD. Each in-flight frame now has its own buffer.
- **Physics reset every moving entity's scale to 1.** `Scene::step_physics` copied the whole
  body transform back, and rigid bodies have no scale; it now syncs only position and rotation.
- **Despawning through `Scene::world_mut()` left a ghost collider.** The entity's physics body
  stayed in the world and kept colliding. `step_physics` now frees the body of any entity the
  scene has seen that is gone (unless a live entity took the body over).
- **glTF triangle strips and fans were imported as triangle lists,** losing or inventing
  triangles. They now expand to proper lists; points and lines return
  `ImportError::UnsupportedPrimitiveMode`.
- **Malformed glTF could panic instead of returning an error:** float indices, a zero-count or
  VEC2 `POSITION` accessor, a stride shorter than one element, an accessor or image past the end
  of its buffer, an index past the last vertex. All are now `ImportError::InvalidPrimitive`.
- **A textured `.gltf` with `data:` images could not be loaded from memory** (`import_slice`,
  `kaman_model_load_memory`). Embedded images now decode; external file references are still
  refused.
- **One bad mesh index shifted every later triangle.** The upload skipped just that index; it
  now drops the whole triangle it belongs to.
- **A stale mesh handle could alias a live mesh** after 4096 reuses of one registry slot. A slot
  is now retired before its generation wraps.
- **A settings change or resize between `begin_frame` and `submit` was half-applied** (the shadow
  pass skipped while the frame still sampled the map, or the HUD laid out for the new size). The
  frame now keeps what it opened with.
- **Keys stuck down after the window lost focus,** swallowing the next press, and **a tap
  shorter than one fixed step was lost.** Focus loss now releases everything, and press and
  release edges last until the next update sees them.
- **The cursor position was in physical pixels,** 2× the documented logical points on Retina.
- **The fixed-timestep loop dropped a legitimate sub-step remainder** whenever exactly
  `MAX_STEPS_PER_FRAME` steps fit, snapping the interpolation alpha to 0.
- **`Game::init` saw the drawable in points** with a UI scale of 1; it is now sized first.
- **`FontAtlas::measure` counted `\n` as an advance;** it now returns the widest line.
- **A music start that failed still claimed the loop channel,** so asking again did nothing.
- **The demo's replay message printed the best score labelled "score".**
- **Tests that could not fail.** `NullRenderer` now refuses the calls the Metal backend ignores
  (see Changed); demo tests that sat on the title screen now drive the game; render and ECS
  tests that only re-derived their own formula, drew nothing or asserted a tautology now check
  real output; and the HUD is part of the zero-allocation test.

## [0.1.0-alpha.2] - 2026-09-27

### Added

- **Homebrew install (Apple Silicon).** `brew install parsabee/kaman/kaman-engine` installs a
  prebuilt `kaman-demo` (the playable demo), its assets, and the engine source under
  `share/kaman-engine/src` for use as Cargo path dependencies. The formula
  (`packaging/homebrew/kaman-engine.rb`) requires arm64 macOS and refuses Intel Macs.
- **Release workflow.** `.github/workflows/release.yml` runs on every `v*` tag (or by hand for an
  existing tag): it builds `playable-demo` in release mode with the pinned toolchain, checks the
  binary is arm64 and passes `--smoke`, packages
  `kaman-engine-<version>-aarch64-apple-darwin.tar.gz` plus a `.sha256` with
  `scripts/package-release.sh`, and attaches both to the GitHub release.
- **`kaman-demo --print-assets-dir`** prints where the demo found its assets.

### Changed

- **The demo finds its assets at runtime** instead of through compile-time `CARGO_MANIFEST_DIR`
  paths, so an installed binary works without a source checkout. It tries, in order,
  `$KAMAN_DEMO_ASSETS`, `<exe dir>/../share/kaman-engine/assets` (Homebrew and the release
  tarball), `<exe dir>/assets`, then the crate's own `assets/` (so `cargo run` is unchanged). If
  none exists it prints what it tried and exits 1.

## [0.1.0-alpha.1] - 2026-09-27

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

[Unreleased]: https://github.com/parsabee/KamanEngine/compare/v0.1.0-alpha.2...HEAD
[0.1.0-alpha.2]: https://github.com/parsabee/KamanEngine/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/parsabee/KamanEngine/releases/tag/v0.1.0-alpha.1
