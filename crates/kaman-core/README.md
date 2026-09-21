# kaman-core

The application lifecycle and the **engine/game boundary** for KamanEngine — the
crate that owns the loop and defines the seam every game plugs into.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.

## Responsibility

`kaman-core` sits at the top of the engine stack. It owns the event loop and
frame cadence and drives a game through a narrow, backend-agnostic seam. It
depends on `kaman-ecs`, `kaman-render-api`, `kaman-perf`, `kaman-math`, and
`winit` — but **never** on `metal` or the concrete renderer. All rendering goes
through the `kaman-render-api` traits (ARCHITECTURE §2), so the engine loop is
provably insulated from the Metal backend.

Game concepts (car, road, score, obstacle, lane) never enter this crate. A unit
test (`boundary_tests::no_game_specific_symbols`) scans every source file for
those words and fails the build if one leaks in, mirroring the guard in
`kaman-ecs`.

## The boundary (ARCHITECTURE §3)

The engine owns the loop and calls into the game through two types:

| Type | Role |
|---|---|
| `Game` (trait) | Implemented by a game. Three lifecycle hooks — `init`, `update`, `render` — with a fixed call order. All game-specific code lives behind it. |
| `EngineCtx` | The narrow handle passed into each hook. Exposes *only* engine services through short-lived accessors. |

### `Game` call ordering (invariant)

For a driver that runs `N` frames: `init` once, then `(update, render)` × `N`.
`init` never runs twice; `render` never runs before `init`; `update` and
`render` are always paired in that order. The doc comment states it, the driver
enforces it, and `headless::tests::init_once_then_update_render_pairs` tests it.

### What `EngineCtx` exposes (and only this)

- `world()` / `world_mut()` — the ECS `hecs::World`.
- `renderer()` — a `&mut dyn Renderer` (`RenderDevice` + `FrameRecorder`), the
  `kaman-render-api` seam. `NullRenderer` in Phase 1; the Metal backend
  (KE-0102) later. No Metal type is ever visible.
- `input()` — a read-only `InputState` snapshot (pressed keys, mouse buttons,
  cursor position). Backend-agnostic `Key`/`MouseButton` enums, not `winit`
  types. The full input abstraction is KE-0304.
- `perf()` — a `PerfSnapshot` (frame timing) from `kaman-perf`.

`EngineCtx` is the borrow-checker chokepoint: the engine builds a fresh
`EngineCtx` borrowing its state for each hook call and hands out `&mut EngineCtx`.
The game reaches each service through a short-lived accessor borrow, so it cannot
alias engine state across a frame or stash a handle past the call.

## Two drivers, one loop

- **`headless`** — no window, no GPU, against a `NullRenderer`. Used by the
  `--smoke` oracle and all tests; runs on headless CI. `headless::run(game, n)`
  returns a `Headless` harness for inspecting the resulting `World` and recorded
  draws.
- **`run`** (windowed) — a `winit` 0.30 window on macOS driving the identical
  hook sequence. Per-frame drawing is behind `present_frame`, an isolated
  function KE-0102 fills with the Metal backend. macOS-only bits (window
  creation, event translation) are kept in small free functions to ease the
  KE-0301 `#[cfg]` split.

## Deferred to KE-0102

- The real Metal renderer behind the seam. `present_frame` is a no-op in Phase 1
  (the window is created but shows nothing).
- Renderer/scene/camera wiring from the prototype's `app.rs` (which is entangled
  with the not-yet-migrated `renderer`/`scene`/`camera`/`ui` modules). Only the
  loop + input *skeleton* was extracted here; the draw/present body is stubbed.

## Provenance

The loop/input skeleton is adapted from the `ProjectRigor` prototype's `app.rs`
(winit `ApplicationHandler` + keyboard/mouse tracking). The camera, scene, and
menu-bar entanglement was intentionally dropped; input keys were generalized to
backend-agnostic `Key`/`MouseButton` enums. See `docs/ARCHITECTURE.md` §6.
