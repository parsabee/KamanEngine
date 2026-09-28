# KE-0408 — Graphics settings + native macOS Graphics menu

Phase:         4
Priority:      P1
Status:        Done
Integration:   New
Size:          L · A2
Time:          M
Risk:          Med
Depends on:    KE-0407, KE-0404      Blocks: —
Serves:        KR4.1

## Problem / Motivation
Every graphics quality knob is either hardcoded or unreachable from a game:

- Shadow quality is a compile-time map size.
- Shadow distance and fog are backend-only (`MetalRenderer::set_shadow_distance`, `LightUniforms`
  defaults).
- The drawable is sized once, at startup, to the window's bounds in *points*. Nothing resizes it,
  and there is no way to trade resolution for fill rate.

Players have no way to adjust any of this, and a game that wants an options screen has no API for
it.

Parsa asked for an Apple-native drop-down menu for graphics settings: shadows, draw distance, and
resolution, plus a separate shadow distance. The menu is the engine's default, but games must be
able to opt out and drive the same settings from their own UI.

## Scope & Acceptance
- [x] A platform-neutral, public `GraphicsSettings` model in `kaman-core` with presets:
      Shadows **Off / Low (2048²) / High (4096², default)**; Shadow Distance **Match Draw Distance
      (default) / Medium 0.5× / Near 0.25×** of the draw distance; Draw Distance **Near 0.5× / Medium
      0.75× / Far 1.0× (default)** of the game's reach; Resolution **50% (default) / 75% / 100%** of
      the window's native pixels.
- [x] Metal-free seam additions in `kaman-render-api`, with `NullRenderer` support:
      `RenderSettings` + `ShadowQuality` and `RenderDevice::set_render_settings`, and
      `RenderDevice::resize_surface` / `surface_scale` (UI scale).
- [x] `MetalRenderer` implements them:
      - Off skips the shadow pass, and Low/High re-create the map only on a size change.
      - The shadow range is resolved as a fraction of the draw distance.
      - The draw-distance scale scales the fog from its defaults, without compounding.
      - The drawable is sized explicitly.
- [x] One apply path for every settings UI:
      - `EngineCtx::graphics_settings` / `set_graphics_settings` queue a change.
      - The loop applies it at the start of the next frame, before any `update`: it pushes the
        seam settings and rescales `Scene` streaming reach from the reach the game set in `init`.
      - `Game::graphics_settings_changed` (default no-op) is called afterwards.
      - Lowering the draw distance never despawns ahead, so seeded spawn sequences stay
        deterministic.
- [x] The windowed runner installs a native **Graphics** menu in the macOS menu bar by default:
      - it is an `NSMenu` via `objc2` with a submenu per setting, checkmarks, and Shadow Distance
        disabled while shadows are off;
      - it adds *Enter/Exit Full Screen* (⌃⌘F) and *Reset to Defaults*;
      - it is appended after winit's app menu, which stays intact.

      Clicks reach the loop through winit's `EventLoopProxy`, with no globals.
- [x] Opt-out: `run_with_config(game, factory, RunConfig { native_settings_menu: false, .. })`, and
      `persist_graphics_settings` separately.
- [x] Persistence: settings are restored at launch (before `Game::init`) and saved on every change,
      in `NSUserDefaults`. This happens in the windowed run only; headless runs, tests and `--smoke`
      never read or write them.
- [x] Default render scale 50% of native, the same resolution as before this ticket on Retina. The
      demo HUD multiplies its point-authored sizes by `surface_scale`, so text keeps its size at
      every resolution.
- [x] `kaman-core` stays metal-free (`cargo tree -p kaman-core -e normal | grep -i metal` is empty),
      and it uses the objc2 0.5 / objc2-app-kit 0.2 generation winit 0.30 already builds.
- [x] Windowed manual check on a Retina Mac (accepted by the owner, 2026-09-28):
      - the menu appears, and every item has its visible effect;
      - resizing no longer stretches a stale drawable;
      - full screen works;
      - the settings persist across a relaunch.

## Technical notes
- The seam carries only **relative** quantities (tier, fraction, scale) because the fog and the
  shadow-map sizes are backend look defaults that nothing above the seam owns.
- A `CAMetalLayer` does not track its view once `drawableSize` is set, so the runner resizes the
  drawable explicitly on `Resized`, `ScaleFactorChanged` and render-scale changes. The MSAA and
  depth attachments were already re-created lazily on a size change.
- AppKit menu tracking holds the main thread while a menu is open. After a menu command the runner
  restarts the frame clock, so the frozen time is not simulated as a catch-up burst.
- The menu model (`settings_menu.rs`) is pure data: layout, labels, checked/enabled rules, and tag
  encoding. `platform/macos.rs` only renders it, so an iOS `UIMenu` can reuse it (Phase 3).

## Out of scope
- An in-window settings panel. Exclusive full-screen video modes. Window-size presets.
- Per-game custom menu items. iOS UI (Phase 3).
- Migrating `kaman-render`'s `cocoa`/`objc` usage to objc2.

## Test gate
- `kaman-core`:
  - `graphics::tests`: preset factors, `render_settings`, code round-trip for every combination,
    unknown and missing codes, the surface math;
  - `graphics::loop_tests`: init-time apply without a change event, pre-init restore seen by
    `init`, a game request applying next frame with one hook call, streaming rescale with no
    despawn-ahead and no double-spawn;
  - `settings_menu::tests`: layout, labels, tags, checkmarks, enable rules, reset.
- `kaman-render-api`: settings defaults, and `NullRenderer` recording.
- `kaman-render`:
  - `shadow` tier sizes, and snapping stability on the 2048² map;
  - `tests/render_settings.rs` (GPU; skips without a device): fog scaling, shadow range fraction,
    tier resize/disable, Off byte-identical to casters-off, resize/UI scale.
- `playable-demo`: `hud_keeps_its_on_screen_size_at_every_render_scale`.
- Workspace build, test, clippy `-D warnings`, `--smoke`, and `cargo doc` with `-D warnings`.

## Doc gate
`#![deny(missing_docs)]` holds. `docs/GETTING_STARTED.md` documents both paths (built-in menu vs
own UI); `docs/ARCHITECTURE.md` §2 notes the seam additions; crate READMEs (`kaman-core`,
`kaman-render-api`, `kaman-render`), `docs/PLAYABLE_DEMO.md` and `README.md` mention the menu.
