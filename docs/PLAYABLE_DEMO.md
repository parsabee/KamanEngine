# The Playable Demo — a guided walkthrough

`games/playable-demo` is KamanEngine's first title (an endless runner) and the **reference
example** for building a game on this engine. It is a plain workspace member that depends only
on the engine's published crates: it implements one trait, receives one context type, and
reaches every engine service through that context. Nothing in it reaches past the public API.

This document walks through how it is built, with a pointer into the real code for each claim.
To **start your own** game rather than read this one, see
[GETTING_STARTED.md](GETTING_STARTED.md). For the constraints behind these seams see
[ARCHITECTURE.md](ARCHITECTURE.md); for the diagrams see [DESIGN.md](DESIGN.md).

> Scope: the demo as of Phase 7 (KE-0701 … KE-0707). Everything described here exists in the
> code today. Where the demo deliberately *doesn't* use an engine facility (physics contact
> queries, `EngineCtx::alpha`, a non-default streaming axis), that is called out rather than
> glossed over.

---

## 1. The `Game` / `EngineCtx` boundary

The engine owns the loop. A game is any type implementing
[`kaman_core::Game`](../crates/kaman-core/src/game.rs#L95) — three hooks, no more:

| Hook | Called | The demo's implementation |
|---|---|---|
| `init(&mut EngineCtx)` | exactly once, before the first `update` | [game.rs:439](../games/playable-demo/src/game.rs#L439) |
| `update(&mut EngineCtx, dt)` | 0..N times per frame, `dt` always `FIXED_DT` | [game.rs:541](../games/playable-demo/src/game.rs#L541) |
| `render(&mut EngineCtx)` | exactly once per frame, after that frame's updates | [game.rs:662](../games/playable-demo/src/game.rs#L662) → [render.rs:38](../games/playable-demo/src/render.rs#L38) |

[`EngineCtx`](../crates/kaman-core/src/context.rs#L77) is the only channel into the engine. It
is built fresh by the driver for each hook call and dropped immediately after, so a game cannot
stash it across frames. Its whole surface is: the [`Scene`](../crates/kaman-scene/src/lib.rs#L141)
(`scene`/`scene_mut`), the ECS world (`world`/`world_mut`, delegating to the scene), the render
seam (`renderer`), the [`Camera`](../crates/kaman-camera/src/camera.rs) (`camera`/`camera_mut`),
the read-only `InputState` (`input`), a `PerfSnapshot` (`perf`), and the fixed-timestep
interpolation factor (`alpha`). There is no accessor for a window, a platform handle, a Metal
device, or game state — by design.

**What the engine owns.** The event loop and the clock; the `Scene` (ECS `World` + `PhysicsWorld`
+ streaming bookkeeping); the `Camera`; input collection; the render backend; the frame cadence.
The demo's own struct holds none of these — see the field list on
[`CarRunner`](../games/playable-demo/src/game.rs#L48): it holds lane, travel, score, best, PRNG,
game state, and the resource *handles* it created in `init`. That is all.

**What the game owns.** Its rules and its vocabulary. Lanes, obstacles, score, the state machine,
the collision test, the difficulty ramp, and the game-side ECS components
([components.rs](../games/playable-demo/src/components.rs)) all live in the game crate. See
[§7](#7-the-boundary-and-what-enforces-it) for why they cannot live anywhere else.

**Backend injection.** `kaman-core` must not depend on `metal`, so it does not construct the
renderer. The *game binary* does: `main.rs` calls
[`run_with_backend`](../crates/kaman-core/src/app.rs#L128) with a factory closure that builds a
`kaman_render::MetalRenderer` and boxes it as `dyn Renderer`
([main.rs:109](../games/playable-demo/src/main.rs#L109)). `metal` therefore reaches the process
through `kaman-render` and the game binary only, never through `kaman-core` (ARCHITECTURE §2).
The `kaman-render` dependency is even `cfg`-gated to macOS in
[Cargo.toml](../games/playable-demo/Cargo.toml#L35).

---

## 2. The fixed-timestep loop

`FIXED_DT` is `1.0 / 60.0` and lives in exactly one place:
[timestep.rs:50](../crates/kaman-core/src/timestep.rs#L50). `Game::update` is *always* called
with that constant — never a measured wall-clock delta.

Each display frame, the driver banks the real elapsed time in the
[`Accumulator`](../crates/kaman-core/src/timestep.rs#L72), which returns the whole number of
fixed steps to run and keeps the sub-step remainder. Both drivers then run the same
[`drive_frame`](../crates/kaman-core/src/driver.rs#L122): `update × k`, then `render` once.
`k` may be 0 (a display faster than the sim rate) or several (a slow frame catching up), capped
at [`MAX_STEPS_PER_FRAME = 5`](../crates/kaman-core/src/timestep.rs#L63) so a stall slows the
simulation instead of wedging the loop.

**Why `update` and `render` are separate hooks.** Because they run at different rates. `update`
is the only place game state advances, so the amount of simulation per second of game time is
independent of how fast the display is. `render` is the only place draws are recorded, so it can
run once per *displayed* frame regardless of how many steps preceded it. The demo respects the
split strictly: `update` moves the car, streams, and resolves collisions and records nothing;
[render.rs](../games/playable-demo/src/render.rs) reads state and records draws and mutates
nothing. (`EngineCtx::alpha`, the interpolation factor for lerping between fixed states inside
`render`, exists and is wired through; the demo does not use it — at 60 Hz fixed and a 60 Hz
display there is nothing visible to gain, and ignoring it keeps the render pass a pure read.)

**Two clocks, one loop.** The headless driver
([headless.rs](../crates/kaman-core/src/headless.rs)) advances a **synthetic** clock by exactly
one `FIXED_DT` per frame, so every frame runs exactly one `update` and one `render` — it never
reads wall time. The windowed entry reads a real monotonic clock. Only the clock source differs;
the accumulator and `drive_frame` are shared.

**Why runs are deterministic.** Under the headless driver, nothing that affects simulation is
allowed to vary: the timestep is constant, the clock is synthetic, streaming decisions are a pure
function of the focus position and the `StreamingConfig`
([kaman-scene](../crates/kaman-scene/src/lib.rs#L37)), and every random choice the demo makes
comes from a seeded PRNG ([§6](#6-determinism-and-testing)). Same seed and same inputs ⇒ the
same frames, on any machine, with or without a GPU.

---

## 3. Scene streaming and the floating-origin rebase

The world is endless, so the demo never builds it — it streams it. The engine half is
[`Scene::stream`](../crates/kaman-scene/src/lib.rs#L315): the *game* supplies a **focus point**
and a **spawn callback**; the scene owns the bookkeeping (which slots ahead are filled, which
streamed entities have fallen behind).

The demo's focus is the player's position
([`player_position`](../games/playable-demo/src/game.rs#L154)) and its callback is
[`spawn_slot`](../games/playable-demo/src/game.rs#L333), called from `update` at
[game.rs:645](../games/playable-demo/src/game.rs#L645). Per slot it spawns a road tile, an
obstacle on a cadence, two roadside buildings and two guardrail segments — reporting each via
[`SpawnCtx::spawned`](../crates/kaman-scene/src/lib.rs#L471) so the scene despawns it once it
falls behind. Despawn is **atomic**: the physics rigid body is removed with the ECS entity, so no
live `PhysicsBodyComponent` ever holds a freed handle
([`Scene::despawn`](../crates/kaman-scene/src/lib.rs#L262)).

The demo runs on the **engine-default** `StreamingConfig`
([kaman-scene:107](../crates/kaman-scene/src/lib.rs#L107)) — axis `-Z`, `spawn_interval` 6,
`spawn_ahead` 60, `despawn_behind` 12, `rebase_threshold` 1000 — because the loop creates the
scene and the default axis is already the demo's travel direction. Road tiles and the guardrail
mesh are sized from `ctx.scene().config().spawn_interval` rather than a literal
([game.rs:462](../games/playable-demo/src/game.rs#L462),
[game.rs:517](../games/playable-demo/src/game.rs#L517)), so streamed segments abut flush whatever
the interval is.

### The rebase, and what the game must carry through it

[`Scene::maybe_rebase`](../crates/kaman-scene/src/lib.rs#L395) keeps world coordinates out of
float-precision trouble: once the focus is more than `rebase_threshold` from the origin, **every**
position — every `TransformComponent` and every physics body translation — is shifted by one
offset back toward the origin, and the offset actually applied is returned. Relative positions are
preserved exactly, so gameplay is unaffected. It must be called *between* physics steps, never
inside one.

The catch, and the single most important thing to copy from this demo: **anything the game holds
in world space is not in the ECS, so the rebase does not move it. The game must move it.** The
demo holds two such things.

1. **`travel`**, the monotonic along-axis distance the player's transform is rebuilt from. The
   demo folds the offset into it: `self.travel += offset.dot(axis)`
   ([game.rs:596](../games/playable-demo/src/game.rs#L596)). Without this the player would be
   rebuilt at its old coordinate in a world that has moved.
2. **The camera.** The engine owns the `Camera`, but its position and target are world
   coordinates, so a rebase strands it a full threshold away. The demo translates both by the
   same offset — [game.rs:598–613](../games/playable-demo/src/game.rs#L598), which carries a
   worked comment on exactly this. Note *why* it translates rather than re-deriving the pose from
   the car: a smoothed follow always trails its desired pose slightly, and snapping to the
   desired pose would erase that lag in one frame — a visible jolt. Shifting preserves the lag
   exactly, so the rebase is invisible.

The demo's ordering within `update` is: **rebase → write the player transform → follow the camera
→ stream** ([game.rs:588](../games/playable-demo/src/game.rs#L588) onward). Everything downstream
of the rebase therefore sees one consistent coordinate space for the whole step.

A third pattern worth noting: content that should *not* move with the world at all is drawn
**camera-locked** instead of streamed — the ground terrain
([render.rs:163](../games/playable-demo/src/render.rs#L163)) and the skyline backdrop
([render.rs:172](../games/playable-demo/src/render.rs#L172)) are positioned from the player's `Z`
every frame, so neither a stream nor a rebase can slide them.

Two headless tests pin all of this: `a_dodging_run_never_ends_without_an_actual_collision`
([game.rs:860](../games/playable-demo/src/game.rs#L860)) drives far enough to cross the rebase
threshold while steering clear of traffic and asserts the run never ends, and
`the_camera_stays_with_the_car_across_a_rebase`
([game.rs:919](../games/playable-demo/src/game.rs#L919)) asserts the camera's pose *relative to
the car* changes no more on a rebase step than on an ordinary one. Both fail if the rebase
bookkeeping above is dropped.

---

## 4. Asset loading

Every model the demo draws is a committed glTF file imported exactly once through
`kaman-assets` and thereafter referenced by handle. The two load paths are in
[assets.rs](../games/playable-demo/src/assets.rs):

- [`load_model`](../games/playable-demo/src/assets.rs#L41) — a multi-part model. Returns one
  `(fitted transform, MeshHandle, Option<TextureHandle>)` per mesh-bearing node. Parts whose
  material carries a base-color image get a texture and draw on the textured pipeline; parts
  without one draw on the Phong pipeline with the material color the importer packed as vertex
  color.
- [`load_textured_mesh`](../games/playable-demo/src/assets.rs#L149) — a single always-textured
  mesh (the road tile, the skyline billboard).

Both go through [`AssetCache::load`](../crates/kaman-assets/src/lib.rs), which dedups by path so
one file is parsed and uploaded once (KE-0103's "upload once, reference by handle"). The *fit*
functions turn a model's authored bounds into a placement transform:
[`fit_transform`](../games/playable-demo/src/assets.rs#L110) scales a car so its longer horizontal
extent is `CAR_LEN`, yaws it to face `-Z`, and lifts its underside to the road;
[`building_fit`](../games/playable-demo/src/assets.rs#L80) scales a prefab to a common footprint
(heights preserved) with its base at the model origin. Both work from baked world-space bounds, so
they are robust to whatever scale and orientation a model was authored at.

### Committed assets

All of these live in [`games/playable-demo/assets/`](../games/playable-demo/assets) and are
committed, so a clean checkout runs with no asset build step. Paths are resolved against
`CARGO_MANIFEST_DIR` ([config.rs:92](../games/playable-demo/src/config.rs#L92) onward) so they
hold regardless of the process working directory.

| Asset | Role | Provenance |
|---|---|---|
| `sports_car.glb` | the player's car | Quaternius, CC0 |
| `car.glb`, `car2.glb`, `police_car.glb` | traffic cars | Quaternius, CC0 |
| `skyscraper_{a,b}.glb`, `large_{a,b,c}.glb`, `small_{a,b}.glb`, `low_a.glb` | roadside buildings | Kenney City Kit, CC0 |
| `road.gltf` | textured asphalt road tile | baked by `gen_asphalt` from `asphalt_src.jpg` (Poly Haven `asphalt_02`, CC0) |
| `skyline.gltf` | distant skyline billboard | baked by `gen_skyline` from `skyline_src.jpg` (Wikimedia Commons, CC0) |
| `font.bin` | SDF font atlas for the HUD | baked by `gen_font` from `font.ttf` (Roboto, SIL OFL 1.1 — licence text in `assets/font-OFL.txt`) |

### Derived assets are baked by `examples/`, not at runtime

Three assets are *derived* — a PNG-carrying glTF and a font atlas — and each has a one-shot
generator under [`examples/`](../games/playable-demo/examples):

```sh
cargo run -p playable-demo --example gen_asphalt   # → assets/road.gltf
cargo run -p playable-demo --example gen_skyline   # → assets/skyline.gltf
cargo run -p playable-demo --example gen_font      # → assets/font.bin
```

The deliverable is the committed output, not the tool: a normal build never runs these. That is
the point of shipping them as examples — `image` and `fontdue` stay **dev-dependencies**
([Cargo.toml:18](../games/playable-demo/Cargo.toml#L18)), so the shipped binary gains neither an
image codec nor a font parser. At runtime the embedded road/skyline PNGs are decoded by
`kaman-assets`, and `font.bin` is a self-contained `KFNT` blob of metrics plus a single-channel
distance field that [`load_font`](../games/playable-demo/src/hud.rs#L89) parses by hand.

---

## 5. The render seam and the HUD

### Recording a frame

The seam is two traits in `kaman-render-api`, bundled for the game as one
`&mut dyn Renderer` ([context.rs:44](../crates/kaman-core/src/context.rs#L44)):

- [`RenderDevice`](../crates/kaman-render-api/src/device.rs#L87) — load-time resource ownership:
  `create_mesh` / `create_texture` / `create_pipeline` (and their `destroy_*`), plus
  `surface_size` and `safe_area_insets`. Every call returns an **opaque handle** — a newtype over
  an id. No GPU type crosses the seam in either direction.
- [`FrameRecorder`](../crates/kaman-render-api/src/recorder.rs#L59) — per-frame recording:
  `begin_frame` → `set_pipeline` / `bind_texture` / `draw_mesh` / `draw_overlay_quad` → `submit`.
  (`set_view_projection` is also on this trait, but the *driver* calls it, pushing the engine
  camera's matrix across the seam before `Game::render` runs — the game never does.)

The demo creates everything in `init`: two pipelines (`vertex_main`/`fragment_main` on the
`[pos,normal,color]` layout, `textured_vertex_main`/`textured_fragment_main` on
`[pos,normal,uv]`), the imported meshes and textures, the two procedural meshes, and the font
atlas — [game.rs:466–531](../games/playable-demo/src/game.rs#L466). It keeps only the handles.

[`render_frame`](../games/playable-demo/src/render.rs#L38) then records a frame with **no
allocation of GPU resources and no pipeline thrash**: it first gathers this frame's transforms out
of the ECS world by role (road tiles, guardrails, model placements), then draws them grouped so
each pipeline switch and texture bind happens once per group:

1. the skyline backdrop (textured pipeline),
2. the road tiles, then every textured model part,
3. the ground terrain, the guardrails, then every untextured model part (Phong pipeline),
4. the HUD.

Role is read off the entity, not tracked separately: the player is `self.player`, an obstacle is
any entity with a `PhysicsBodyComponent`, a building carries `BuildingVariant`, a guardrail
`GuardrailTag`, and whatever is left is a road tile.

### The HUD

[hud.rs](../games/playable-demo/src/hud.rs) is the worked example of a game drawing its UI
through the engine. It owns no game logic — it reads `CarRunner`'s state and turns it into
overlay quads.

The seam is the **2D overlay** (KE-0404,
[overlay.rs](../crates/kaman-render-api/src/overlay.rs)): a game records
[`OverlayQuad`](../crates/kaman-render-api/src/overlay.rs#L71)s during its frame with
`draw_overlay_quad`, and the backend batches them and flushes them at `submit` in its own
orthographic, depth-disabled, alpha-blended pass **after** every 3D draw. So the HUD composites
on top regardless of record order — which is why
[`draw_hud`](../games/playable-demo/src/hud.rs#L161) can simply be the last thing
`render_frame` records. Coordinates are pixels with the origin top-left. A quad's fill is
`Solid`, `Textured`, or `Sdf`.

**Text is `Sdf` against the font atlas.** [`FontAtlas`](../crates/kaman-render-api/src/overlay.rs#L126)
is plain data — glyph metrics in em units plus the uploaded atlas texture handle; the backend
never parses a font. [`FontAtlas::layout`](../crates/kaman-render-api/src/overlay.rs#L169) walks a
`&str` and hands one `OverlayQuad` per visible glyph to a callback, and
[`measure`](../crates/kaman-render-api/src/overlay.rs#L156) returns a string's pixel width for
centering. Because the metrics are in em units, one atlas serves any pixel size — the demo draws
the score at 30 px and the banner headline at 64 px from the same texture.

**Allocation-free.** `layout` allocates nothing, and the demo formats its numbers into
[`StackStr`](../games/playable-demo/src/hud.rs#L40), a fixed-capacity `fmt::Write` buffer on the
stack (overflow is dropped rather than panicking — HUD text is short and cosmetic). So the whole
per-frame HUD path is heap-free (KR1.2).

**Safe area.** Positions are derived from `surface_size` shrunk by
[`safe_area_insets`](../crates/kaman-render-api/src/device.rs#L146) (`[top, right, bottom, left]`
in pixels), so the HUD stays clear of notches, rounded corners and home indicators. A desktop
window reports zeros; the iOS path reports real insets. The score sits at
`inset + HUD_MARGIN`, and the game-over banner is centered in the *safe* rectangle, not the raw
drawable ([hud.rs:244](../games/playable-demo/src/hud.rs#L244)).

The HUD also carries the demo's full-screen washes, recorded first so everything composites over
them: opaque black on `Ready` (the title screen), that black retiring over `HUD_FADE_SECONDS`
once a run starts, and a partial dim behind the `GAME OVER` banner.

---

## 6. Determinism and testing

**The seeded PRNG.** [rng.rs](../games/playable-demo/src/rng.rs) holds two deliberately separate
sources of randomness, and neither reads wall-clock time:

- [`Rng`](../games/playable-demo/src/rng.rs#L32), a SplitMix64 seeded once from
  [`config::SEED`](../games/playable-demo/src/config.rs#L55), advanced *only* when `spawn_slot`
  picks an obstacle lane. This is the sequence tests depend on.
- [`hash_u64`](../games/playable-demo/src/rng.rs#L61) and the `*_for_slot` helpers, which derive
  **cosmetic** choices (traffic-car variant, building prefab, lateral jitter) straight from the
  streaming slot index without touching `Rng`.

The split is the point: adding scenery variety can never perturb the obstacle world. The same
seed always produces the same lane sequence and the same obstacle positions whether or not a
building is drawn beside them.

**The `--smoke` oracle.** `cargo run -p playable-demo -- --smoke` boots the same `Game` under the
headless driver, runs **120 frames** against a `NullRenderer` with no GPU and no Metal device,
asserts the driver ran exactly that many frames, prints `smoke: 120 frames OK`, and exits 0
([main.rs:128](../games/playable-demo/src/main.rs#L128)). It taps `Space` on the first frame —
without that it would replay 120 frames of a motionless title screen and assert nothing — then
releases it, so the car runs straight down the middle lane on empty input. It is the workspace's
continuous oracle: CI runs it on every push with Metal API Validation enabled
(`.github/workflows/ci.yml`), and two unit tests pin both the run and the exact stdout contract
line ([main.rs:156](../games/playable-demo/src/main.rs#L156)).

**The headless harness.** [`Headless`](../crates/kaman-core/src/headless.rs#L86) is what makes
gameplay testable at all. It owns the engine state a `Game` runs against — scene, input, perf,
accumulator, and a `NullRenderer` — runs `init` once, and steps frames on the synthetic clock. A
test can seed input before a run (`input_mut`), step in chunks, and afterwards inspect the ECS
world, the `Scene`, the recorded draws, **and the engine-owned
[`camera`](../crates/kaman-core/src/headless.rs#L148)**. That last accessor is why the demo can
test *view* behaviour with no GPU — `the_camera_stays_with_the_car_across_a_rebase` asserts on
real camera poses in a plain `cargo test`.

The demo's own suite ([game.rs:668](../games/playable-demo/src/game.rs#L668) onward) covers, all
headlessly: lane geometry and edge clamping, edge-triggered input (a held key moves exactly one
lane), the frozen title state and the opening fade retiring on the fixed timestep, the difficulty
ramp and its cap (including that the cap keeps per-step motion below the AABB overlap window, so
collisions cannot tunnel), same-lane-only overlap, crash → `GameOver` → replay, obstacle streams
identical across two runs of the same seed, streaming staying bounded over 5 000 frames, and the
two long-run rebase tests from [§3](#3-scene-streaming-and-the-floating-origin-rebase).

Two things the demo deliberately does *not* do, so you don't go looking for them: collision is a
game-side AABB test over entity transforms
([`overlaps`](../games/playable-demo/src/game.rs#L245)), not a physics contact query — the
obstacles carry static bodies and colliders to exercise streaming's atomic despawn, not to drive
the player; and player motion is **kinematic**, its transform written directly, never solved
(ARCHITECTURE §5).

---

## 7. The boundary, and what enforces it

The demo names cars, roads, lanes, obstacles and scores freely. No engine crate does — and that
is not a convention, it is a test.

Four crates carry a self-scanning **guard test** that embeds the crate's own source with
`include_str!` and fails if a forbidden game word appears as a whole token anywhere in it:

| Guard test | Crate |
|---|---|
| `boundary_tests::no_game_specific_symbols` ([lib.rs:104](../crates/kaman-core/src/lib.rs#L104)) | `kaman-core` |
| `tests::no_game_specific_symbols` ([lib.rs:723](../crates/kaman-scene/src/lib.rs#L723)) | `kaman-scene` |
| `guard_tests::no_game_specific_symbols` ([lib.rs:72](../crates/kaman-assets/src/lib.rs#L72)) | `kaman-assets` |
| `tests::test_no_game_specific_symbols` ([lib.rs:695](../crates/kaman-ecs/src/lib.rs#L695)) | `kaman-ecs` |

The forbidden words are assembled from ASCII byte codes rather than written as literals, so each
test's own body contains no occurrence of them and can scan every file in its crate — itself
included — with no exclusion window. Matching is case-insensitive and whole-word, so an
incidental substring inside a larger identifier does not trip the guard.

The practical consequence for a game author: **if a concept has a game name, it belongs in your
crate.** The demo's per-entity variant tags are the clearest illustration —
[components.rs](../games/playable-demo/src/components.rs) defines `TrafficVariant`,
`BuildingVariant` and `GuardrailTag` as ordinary hecs components that ride on the *same* entities
as the engine-generic `TransformComponent` / `RenderComponent` / `PhysicsBodyComponent`. They
compose freely; they just cannot be *defined* in `kaman-ecs`, because the guard would reject them
there.

Two more CI guards bear on the demo (`.github/workflows/ci.yml`):

- **Render-seam firewall** — `cargo tree -p kaman-render-api -e normal` must not mention `metal`.
  Combined with the factory injection in [§1](#1-the-game--enginectx-boundary), `metal` reaches
  the process only through `kaman-render` and the game binary.
- **De-brand guard** — `grep -rniI projectrigor crates games` must find nothing; prototype
  provenance lives in [ARCHITECTURE.md](ARCHITECTURE.md) §6 only.

For the layer diagram, the crate dependency graph, and the sequence diagrams of the frame loop,
streaming and asset load, see [DESIGN.md](DESIGN.md) §1, §2, §5, §8, §9 — this document does not
duplicate them. The invariant table in [DESIGN.md](DESIGN.md) §11 lists every cross-cutting
invariant and the mechanism that enforces it.

---

## 8. Running it

```sh
cargo run -p playable-demo            # windowed, Metal backend (macOS)
cargo run -p playable-demo -- --smoke # headless oracle: 120 frames, prints "smoke: 120 frames OK", exits 0
```

| Key | Action |
|---|---|
| `Left` / `Right` | Move one lane left / right (one lane per press) |
| `Space` | Start the run from the title screen; replay after a crash |
| `Escape` | Quit |

Controls are handled at [game.rs:228](../games/playable-demo/src/game.rs#L228) (lanes) and
[game.rs:541](../games/playable-demo/src/game.rs#L541) (`Space`, per state); `Escape` is handled
by the engine's windowed entry.

---

## See also

- [GETTING_STARTED.md](GETTING_STARTED.md) — the minimal steps to stand up your own game.
- [games/playable-demo/README.md](../games/playable-demo/README.md) — the demo's own README:
  how to run it, the module map, and the asset licences.
- [ARCHITECTURE.md](ARCHITECTURE.md) — authoritative constraints (the *why*).
- [DESIGN.md](DESIGN.md) — diagrams. · [INTEGRATION.md](INTEGRATION.md) — test/doc discipline.
- Inline API docs: `cargo doc --workspace --no-deps --open`.
