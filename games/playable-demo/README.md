# playable-demo

KamanEngine's first title — a playable **endless runner** — and the host for the
headless **smoke oracle**.

## Play it

```sh
cargo run -p playable-demo
```

Opens a Metal window on macOS. You drive a **red car** forward down a three-lane
road; **white traffic cars** stream toward you. Dodge them.

The cars are real, authored glTF models imported through `kaman-assets`, not
primitive boxes — see [Car assets](#car-assets) below.

A **chase camera** (`kaman_camera::ChaseController`) trails the box from behind
and above and keeps it framed as it drives — so the runner is visually playable,
not viewed from a fixed point. The game positions the engine-owned camera each
`update`; the engine pushes its view-projection across the render seam before the
frame is drawn (KE-0205).

### Controls

| Key            | Action                          |
| -------------- | ------------------------------- |
| `Left` / `A`   | Move one lane left              |
| `Right` / `D`  | Move one lane right             |
| `Space`        | Restart the run (also after a crash) |
| `Escape`       | Quit                            |

Your **score** climbs with distance travelled and is printed to stdout as you
go; a crash prints the final score and best-so-far, then the run restarts.

## The game loop

The engine drives the game at a **fixed timestep** (`kaman_core::FIXED_DT`), so
behavior is framerate-independent. Each fixed `update`:

1. **Restart** on `Space` (clears nearby obstacles, zeroes the score).
2. **Lane input** — `Left/A` / `Right/D` move the box between three discrete
   lanes (clamped at the edges). Movement is **kinematic**: the box's transform
   is set directly, never driven by the physics solver.
3. **Advance** forward one step at a constant speed.
4. **Rebase then stream** — `Scene::maybe_rebase` runs first (floating-origin
   shift, between physics steps), then `Scene::stream` spawns road tiles and
   deterministically-placed obstacles ahead of the box and despawns whatever has
   fallen behind. The box is the streaming focus.
5. **Collision** — a game-side AABB check (same lane + overlap along the travel
   axis) resets the run on a hit. Contact detection is done in the game from
   entity transforms; no physics contact-query API is used.

Obstacle placement is a seeded PRNG (SplitMix64) — no wall-clock time is read —
so a run is fully reproducible.

## Car assets

The player and traffic cars are authored **glTF** models committed under
`assets/` and imported at startup through `kaman-assets` (KE-0402/KE-0703):

- `assets/player_car.gltf` — the red player car.
- `assets/traffic_car.gltf` — the white traffic (obstacle) car.

Each is a complex, multi-material mesh (chassis + skirt, cabin with glass, four
wheels with hubcaps, head/tail-lights, bumpers). Every material is its own glTF
**primitive**; the importer packs each material's base-color factor as the
primitive's per-vertex color, so the car renders in its authored colors on the
untextured Phong pipeline with no texture. Both cars are parsed and uploaded
**once** via the asset cache (`AssetCache`) and shared by handle (KE-0103); a car
entity draws one mesh per primitive at its transform. The road is a textured
asphalt tile — see [Road / asphalt texture](#road--asphalt-texture) below.

No DCC tool is available in-environment, so — as KE-0703 allows — the committed
`.gltf` files are emitted by a procedural generator; regenerate them with:

```sh
cargo run -p playable-demo --example gen_cars
```

The deliverable is the committed asset plus the import path, not the generator
(a normal build never runs it).

### Road / asphalt texture

The road is a **textured asphalt** quad (KE-0704), not a black box. It is a
committed glTF imported once at startup through `kaman-assets`:

- `assets/asphalt_src.jpg` — the asphalt itself: a real, free **CC0** photographic
  texture, [Poly Haven `asphalt_02`](https://polyhaven.com/a/asphalt_02) diffuse
  (CC0 — public domain, no attribution required, safe to redistribute).
- `assets/road.gltf` — a flat road-tile quad on the `[pos,normal,uv]` textured
  layout, with an **embedded base-color PNG** baked from the asphalt photo (tiled
  to the road aspect) plus our own **dashed white lane lines** composited at the
  two interior lane boundaries.

The importer decodes the embedded PNG to RGBA8; the demo uploads it once via
`create_texture` and draws the road on the **textured pipeline**
(`textured_vertex_main` / `textured_fragment_main`, KE-0403) — the free mip chain
+ trilinear-repeat sampler keep the receding road from shimmering. Each frame's
draws are grouped by pipeline: a **textured pass** binds the asphalt texture and
draws the road tiles, then an **untextured pass** draws the cars on the Phong
pipeline.

**UVs / tiling.** The quad bakes `U = 0..1` across the full 12-unit road width
(so the texture spans the road exactly once and the two lane lines land at the
interior lane boundaries `x = ±1.5` ⇒ `U = 0.375 / 0.625`) and `V = 0..3` along
the tile's `spawn_interval` (6-unit) length — three integer repeats. The baked
texture is **6:1** and the seamless photo is tiled across the width, so the
asphalt keeps a natural, isotropic scale (each copy ≈ 2 world units) with no
horizontal smear. Every tile-to-tile streaming seam lands on a texture-wrap
boundary and, the asphalt being seamless in `V`, is invisible as the road scrolls.

The road PNG is baked from the committed CC0 photo + our lane lines (the
deliverable is the committed asset, not the tool); regenerate `assets/road.gltf`
with:

```sh
cargo run -p playable-demo --example gen_asphalt
```

`image` is a **dev-dependency only** (used by that example to load the photo and
encode the PNG); the runtime never gains an `image` dep — it decodes the embedded
PNG via `kaman-assets`.

### City skyline backdrop

A distant city skyline sits on the horizon (KE-0705), so the freeway has a sense
of place instead of an empty sky. It is a real, free **CC0** photo — a New York
City skyline (Wikimedia Commons, public domain) — committed as
`assets/skyline_src.jpg`, cropped to the skyline band and baked onto a **curved
billboard** in `assets/skyline.gltf`.

The technique is a **camera-locked far billboard**:

- The mesh is a shallow horizontal **arc** (a quad whose strips bow forward toward
  the camera at the edges) so the skyline **wraps around the road** rather than
  reading as a flat wall. The bow depth is baked by the generator.
- Each frame the demo draws it **first** (behind the gameplay, in front of the
  gradient sky) at a transform locked to the **player's XZ** and a fixed distance
  ahead — so a streaming/floating-origin rebase never shifts it, and it stays well
  inside the camera's far plane. Gameplay depth-tests over it.
- The skyline crop is contrast-boosted (the source is a foggy, low-contrast day)
  and its picture is nudged down within the billboard so the building tops sit in
  frame and the base lines up with the road's vanishing point.

Regenerate `assets/skyline.gltf` with:

```sh
cargo run -p playable-demo --example gen_skyline
```

> Fog/blend polish (how strongly the distance fog washes the skyline) is a planned
> follow-up, to be tuned with the fog work.

### Elevated freeway: roadside buildings, guardrails, terrain

The road is an **elevated freeway** running through a city (KE-0706):

- **Building prefabs** — 8 committed **CC0** Kenney City Kit models
  (`assets/{skyscraper_a,skyscraper_b,large_a,large_b,large_c,small_a,small_b,low_a}.glb`),
  imported through `kaman-assets` and uploaded **once** each, then shared by handle
  across every instance on screen.
- **Streaming** — `spawn_slot` spawns one building per side per streaming slot,
  reported via `SpawnCtx::spawned` so they despawn behind the player (bounded, no
  per-frame allocation). Each is **non-colliding decoration** — no physics body, so
  only the traffic cars can end a run.
- **Weighted variety** — the prefab is chosen by a weighted per-slot hash
  (`building_for_slot`): skyscrapers are rare (8%), mid/large common (55%), and
  small/low the rest (37%). The lateral offset is jittered per slot so the rows
  aren't a flat wall, and each building is turned 90° to face the freeway.
- **Guardrails** — a procedural rail-and-posts segment (`guardrail_geometry`), one
  `spawn_interval` long so segments abut seamlessly, streamed along both road edges
  at `x = ±5.7` and sitting on the deck.
- **Ground terrain** — a camera-locked ground sheet (`terrain_geometry`): a level
  valley floor at the buildings' `GROUND_Y` under the road and both rows, climbing
  into rolling hills on the flanks (cresting above the horizon). This is what the
  buildings stand on, and it stops the sky showing through the mid-ground. Being
  camera-locked, a streaming/floating-origin rebase never slides it.

**Determinism.** Prefab choice, lateral jitter and traffic-car variant all derive
from a `hash_u64` of the streaming slot — reproducible (same seed → same city) and
deliberately **independent of the lane PRNG**, so adding scenery never perturbs the
obstacle world or the headless smoke run.

## The smoke oracle

```sh
cargo run -p playable-demo -- --smoke
```

Boots the same game via the engine's headless driver, simulates 120 frames
offscreen against a `NullRenderer`, prints `smoke: 120 frames OK`, and exits 0.
It requires **no GPU/Metal device**, so it runs on headless CI runners. Headless
input is empty, so the box just runs straight down the middle lane.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
