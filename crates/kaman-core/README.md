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

The engine runs a **fixed timestep** (`FIXED_DT = 1/60 s`, the single source of
truth in `timestep.rs`) decoupled from the display rate. For a driver that runs
`N` frames: `init` once, then per frame `(update × k, render)`, where `k` is the
whole number of fixed steps drained from the accumulator that frame — **0..N**,
not necessarily 1. `init` never runs twice; `render` never runs before `init` and
runs **exactly once per frame**; each `update` receives the constant `FIXED_DT`.
A game must not assume one `update` per `render`.

### Fixed timestep (KE-0201)

Each frame banks the real elapsed time in a shared `Accumulator` and runs
`update(FIXED_DT)` a whole number of times (draining full steps), then `render`
once. Consequences:

- **Framerate independence:** the `update` count per second of simulated time is
  identical at 60 and 120 Hz (± the sub-step remainder). Asserted by
  `driver::tests::same_update_count_at_60hz_and_120hz_through_the_loop` (and at
  the accumulator level in `timestep::tests`), both feeding a *synthetic* clock —
  no wall time.
- **Spiral-of-death guard:** catch-up is capped at `MAX_STEPS_PER_FRAME` (5)
  steps per frame; surplus accumulated time is discarded so a long stall slows the
  sim rather than wedging the loop. Tested by
  `driver::tests::loop_clamps_catch_up_on_a_long_stall`.
- **Interpolation alpha:** `EngineCtx::alpha()` (`0.0..=1.0`) is exposed on the
  render path (`remainder / FIXED_DT`) so `render` can lerp between the previous
  and current fixed states. Phase 2 games may ignore it.

### What `EngineCtx` exposes (and only this)

- `world()` / `world_mut()` — the ECS `hecs::World`.
- `renderer()` — a `&mut dyn Renderer` (`RenderDevice` + `FrameRecorder`), the
  `kaman-render-api` seam. `NullRenderer` in Phase 1; the Metal backend
  (KE-0102) later. No Metal type is ever visible.
- `input()` — a read-only `InputState` snapshot (pressed keys, mouse buttons,
  cursor position). Backend-agnostic `Key`/`MouseButton` enums, not `winit`
  types. The full input abstraction is KE-0304.
- `perf()` — a `PerfSnapshot` (frame timing) from `kaman-perf`.
- `alpha()` — the fixed-timestep interpolation factor in `0.0..=1.0`, meaningful
  on the render path (`0.0` in `init`/`update`).

`EngineCtx` is the borrow-checker chokepoint: the engine builds a fresh
`EngineCtx` borrowing its state for each hook call and hands out `&mut EngineCtx`.
The game reaches each service through a short-lived accessor borrow, so it cannot
alias engine state across a frame or stash a handle past the call.

## Two drivers, one loop

Both drivers own a shared `driver::Loop` (world, input, perf, `Accumulator`) and
run the *same* `driver::drive_frame` — the one implementation of the
fixed-timestep cadence. Only the clock source differs: the headless driver
advances a **synthetic clock** by one `FIXED_DT` per frame (deterministic, one
step per frame); the windowed driver measures a real monotonic `Instant` delta
(variable 0..N steps per frame).

- **`headless`** — no window, no GPU, against a `NullRenderer`. Used by the
  `--smoke` oracle and all tests; runs on headless CI. `headless::run(game, n)`
  returns a `Headless` harness for inspecting the resulting `World` and recorded
  draws.
- **`run`** / **`run_with_backend`** (windowed) — a `winit` 0.30 window on macOS
  driving the identical hook sequence. `run` wires the GPU-free `NullRenderer`;
  `run_with_backend` takes a **backend factory**
  (`FnOnce(&Window, u32, u32) -> Box<dyn Renderer>`) that the game binary uses to
  inject the Metal backend, so `kaman-core` never depends on `metal` (KE-0102).
  The factory runs once in `resumed`, after the window exists. macOS-only bits
  (window creation, event translation) are kept in small free functions to ease
  the KE-0301 `#[cfg]` split.

## The render backend (KE-0102)

The real Metal renderer lives below the seam in `kaman-render` and is injected
through the backend factory above; `kaman-core` stays metal-free (a firewall CI
check enforces `cargo tree -p kaman-core | grep metal` is empty). The headless
driver and the plain `run` entry keep the `NullRenderer`.

## Provenance

The loop/input skeleton is adapted from the prototype's application layer (winit
`ApplicationHandler` + keyboard/mouse tracking). The camera, scene, and menu-bar
entanglement was intentionally dropped; input keys were generalized to
backend-agnostic `Key`/`MouseButton` enums. See `docs/ARCHITECTURE.md` §6.
