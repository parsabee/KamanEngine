# playable-demo

KamanEngine's first title — a playable **endless runner** — and the host for the
headless **smoke oracle**.

You drive a car forward down a three-lane elevated freeway through a city. Traffic
streams toward you; dodge it. The road, the traffic, the roadside buildings, the
guardrails and the terrain are all **streamed** as you go, so the run is endless,
and the score climbs with distance travelled until you hit something.

This crate is also the **reference example** for building a game on this engine: it
touches nothing but the engine's public API. For the guided walkthrough of how it is
built — the `Game`/`EngineCtx` boundary, the fixed-timestep loop, streaming and the
floating-origin rebase, asset loading, the render seam and the HUD — see
[docs/PLAYABLE_DEMO.md](../../docs/PLAYABLE_DEMO.md). To stand up your own game,
see [docs/GETTING_STARTED.md](../../docs/GETTING_STARTED.md).

## Run it

```sh
cargo run -p playable-demo             # windowed, Metal backend (macOS)
cargo run -p playable-demo -- --smoke  # headless oracle: 120 frames, exits 0
```

The windowed path opens a Metal window. The `--smoke` path boots the same game under
the engine's headless driver against a `NullRenderer`, simulates 120 frames with **no
GPU and no Metal device**, prints `smoke: 120 frames OK`, and exits 0 — so it runs on
headless CI runners. Headless input is empty after the opening start tap, so the car
runs straight down the middle lane: a fully deterministic run.

### Controls

| Key | Action |
| --- | --- |
| `Left` | Move one lane left (one lane per press, clamped at the edge) |
| `Right` | Move one lane right |
| `Space` | Start the run from the title screen; replay after a crash |
| `Escape` | Quit (handled by the engine's windowed entry) |

Lane input is **edge-triggered**: each press moves exactly one lane, so holding a key
does not glide you across the road. `Space` is only meaningful in the two frozen
states — it does nothing mid-run.

The HUD shows the live score while you drive, and a centered `GAME OVER` banner with
the final score, the session best and the replay prompt after a crash. Progress and
the crash report are also printed to stdout.

## Code layout

One line per module in [`src/`](src), and what it owns:

| Module | Owns |
| --- | --- |
| [`main.rs`](src/main.rs) | The binary: CLI parsing, the windowed entry (builds the Metal backend and injects it via a factory), and the `--smoke` oracle. |
| [`config.rs`](src/config.rs) | Every tuning constant, grouped by area — lanes/speed/camera, car fit, asset paths, buildings, guardrail, terrain, backdrop, HUD. No logic. |
| [`rng.rs`](src/rng.rs) | Deterministic randomness: the seeded SplitMix64 lane PRNG, and the independent per-slot hash that picks cosmetic variants without perturbing it. |
| [`assets.rs`](src/assets.rs) | glTF import through `kaman-assets` (load once, share by handle), the fit transforms that place a model on the road, and the vertex-layout helpers. |
| [`scenery.rs`](src/scenery.rs) | Procedural geometry built once in `init`: the guardrail segment and the hill-terrain sheet, packed as raw `[pos,normal,color]` vertex bytes. |
| [`components.rs`](src/components.rs) | The game-side ECS components (`TrafficVariant`, `BuildingVariant`, `GuardrailTag`) and the `PlacedModel` enum. These name game concepts, which is why they live here and not in `kaman-ecs`. |
| [`game.rs`](src/game.rs) | `CarRunner`, the `kaman_core::Game` implementation: lane input, the state machine, the difficulty ramp, scoring, streaming, the rebase bookkeeping, and collision. |
| [`render.rs`](src/render.rs) | The render pass: gathers this frame's transforms from the ECS world and records the draws grouped by pipeline and texture. Allocates no GPU resources. |
| [`hud.rs`](src/hud.rs) | The on-screen HUD: loads the SDF font atlas, and draws the score, the title/game-over banners and the screen washes through the engine's 2D overlay seam. Allocation-free text via `StackStr` + `FontAtlas::layout`. |

## Asset bakers (`examples/`)

Three assets are *derived*, and each has a one-shot generator. **The generated assets
are committed**, so a clean checkout builds and runs with no asset step — a normal
build never runs these. That is why they are examples: it keeps `image` and `fontdue`
as **dev-dependencies only**, so the shipped binary gains neither an image codec nor
a font parser.

| Example | Produces | From |
| --- | --- | --- |
| [`gen_asphalt`](examples/gen_asphalt.rs) | `assets/road.gltf` — a road-tile quad with an embedded asphalt PNG and dashed lane lines | `assets/asphalt_src.jpg` |
| [`gen_skyline`](examples/gen_skyline.rs) | `assets/skyline.gltf` — a billboard quad with an embedded skyline PNG | `assets/skyline_src.jpg` |
| [`gen_font`](examples/gen_font.rs) | `assets/font.bin` — a self-contained `KFNT` SDF atlas (metrics + distance field) | `assets/font.ttf` |

```sh
cargo run -p playable-demo --example gen_asphalt
cargo run -p playable-demo --example gen_skyline
cargo run -p playable-demo --example gen_font
```

## Asset licences

Everything under [`assets/`](assets) is free to redistribute. Credits below; the
font's full licence text is committed alongside it as `assets/font-OFL.txt`.

| Asset | Source | Licence |
| --- | --- | --- |
| `sports_car.glb` (player), `car.glb`, `car2.glb`, `police_car.glb` (traffic) | **Quaternius** car models | CC0 1.0 — public domain, no attribution required |
| `skyscraper_a.glb`, `skyscraper_b.glb`, `large_a.glb`, `large_b.glb`, `large_c.glb`, `small_a.glb`, `small_b.glb`, `low_a.glb` | **Kenney** City Kit | CC0 1.0 — public domain, no attribution required |
| `asphalt_src.jpg` (baked into `road.gltf`) | **Poly Haven** [`asphalt_02`](https://polyhaven.com/a/asphalt_02) diffuse | CC0 1.0 — public domain, no attribution required |
| `skyline_src.jpg` (baked into `skyline.gltf`) | New York City skyline photo, **Wikimedia Commons** | CC0 1.0 / public domain |
| `font.ttf` (baked into `font.bin`) | **Roboto**, © 2011 The Roboto Project Authors | SIL Open Font License 1.1 — see `assets/font-OFL.txt` |

`cube.gltf` is no longer imported by the demo — it was the placeholder player mesh
before the real car models landed (KE-0703). Do not delete it: it is still a live
test fixture. `kaman-assets`' `committed_game_asset_imports_from_path`
([import.rs:205](../../crates/kaman-assets/tests/import.rs#L205)) imports it from
this directory to prove the glTF path works against a real committed file, and
[`gen_cube.rs`](../../crates/kaman-assets/examples/gen_cube.rs) regenerates it.

The credits above are given as courtesy; the CC0 assets impose no attribution
requirement. Only the OFL font ships with a licence file, because only the OFL
requires one.

---

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
