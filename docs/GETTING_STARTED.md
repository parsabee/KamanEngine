# Build your own game on Stick Shift Engine

The minimal path from an empty crate to a game the engine drives, windowed and headless. Each
step names the file in [`games/playable-demo`](../games/playable-demo) that is the worked example
for it; for the full walkthrough of that demo see [PLAYABLE_DEMO.md](PLAYABLE_DEMO.md).

Prerequisites are the engine's own: macOS, Rust ≥ 1.91, Command Line Tools. Run
`./scripts/preflight.sh` if you haven't.

---

## 1. A new crate in the workspace

Games live under `games/`. Add the directory, then add it to the workspace `members` list in the
root [Cargo.toml](../Cargo.toml#L3).

`games/my-game/Cargo.toml`:

```toml
[package]
name = "my-game"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[[bin]]
name = "my-game"
path = "src/main.rs"

[dependencies]
stickshift-assets.workspace = true
stickshift-core.workspace = true
stickshift-ecs.workspace = true
stickshift-math.workspace = true
stickshift-render-api.workspace = true
stickshift-scene.workspace = true

# The Metal backend lives below the render seam. Only the game binary depends on
# `stickshift-render` (and thus `metal`); it builds the backend and injects it, so
# `stickshift-core` stays metal-free (ARCHITECTURE §2).
[target.'cfg(target_os = "macos")'.dependencies]
stickshift-render.workspace = true
```

Every dependency is `*.workspace = true`, so versions come from
[the workspace dependency table](../Cargo.toml#L33) — never pin them per-crate. Take only the
crates you use; `stickshift-camera` (for a follow camera) and `clap` (for CLI flags) are the usual
additions. Worked example: [games/playable-demo/Cargo.toml](../games/playable-demo/Cargo.toml).

---

## 2. Implement `Game`

One trait, three hooks: [`stickshift_core::Game`](../crates/stickshift-core/src/game.rs#L96). `init` runs
once; `update` runs 0..N times per frame with `dt` always equal to
[`FIXED_DT`](../crates/stickshift-core/src/timestep.rs#L50) (1/60 s); `render` runs exactly once per
frame after that frame's updates. Advance state in `update`, record draws in `render`, and don't
cross the two — that split is what makes the simulation framerate-independent.

Everything the engine offers arrives through the [`EngineCtx`](../crates/stickshift-core/src/context.rs#L86)
argument: the `Scene` (ECS world + physics + streaming), the ECS world, the render seam, the
`Camera`, an input snapshot, frame timing. There is nothing else to reach for, and no way to reach
past it.

A complete game that imports a model, spawns an entity, moves it, and draws it:

```rust
use stickshift_assets::AssetCache;
use stickshift_core::{EngineCtx, Game};
use stickshift_ecs::TransformComponent;
use stickshift_math::glam::Vec3;
use stickshift_math::Transform;
use stickshift_render_api::{
    MaterialParams, MeshHandle, PipelineDescriptor, PipelineHandle, PipelineKind,
};

/// Resolved against this crate's directory, so the path holds whatever the
/// process working directory is.
const MODEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/model.glb");

struct MyGame {
    /// Created once in `init`; only handles are kept.
    pipeline: Option<PipelineHandle>,
    mesh: Option<MeshHandle>,
    entity: Option<stickshift_ecs::hecs::Entity>,
    /// Game state. Advanced on the fixed timestep, never from wall-clock time.
    elapsed: f32,
}

impl MyGame {
    fn new() -> Self {
        Self { pipeline: None, mesh: None, entity: None, elapsed: 0.0 }
    }
}

impl Game for MyGame {
    fn init(&mut self, ctx: &mut EngineCtx) {
        // Spawn from the engine-generic component vocabulary. Game-named
        // components belong in this crate, not in `stickshift-ecs`.
        self.entity = Some(
            ctx.world_mut()
                .spawn((TransformComponent::from_position(Vec3::ZERO),)),
        );

        // Create GPU resources ONCE, here. Keep the opaque handles.
        let renderer = ctx.renderer();
        self.pipeline =
            Some(renderer.create_pipeline(&PipelineDescriptor::new(PipelineKind::VertexColor)));

        // Import a glTF file once; the cache parses and uploads it, then hands
        // back a mesh handle per primitive.
        let mut cache = AssetCache::new();
        let asset = cache.load(renderer, MODEL).expect("import model");
        self.mesh = asset.mesh_handles.first().copied();
    }

    fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
        // `dt` is always FIXED_DT. Read input with `ctx.input()`.
        self.elapsed += dt;
        if let Some(e) = self.entity {
            let z = -self.elapsed * 10.0;
            if let Ok(mut t) = ctx.world_mut().get::<&mut TransformComponent>(e) {
                t.transform.position.z = z;
            }
        }
    }

    fn render(&mut self, ctx: &mut EngineCtx) {
        let (Some(pipeline), Some(mesh)) = (self.pipeline, self.mesh) else {
            return;
        };

        // Gather transforms first: the query borrows the world, the draws borrow
        // the renderer, and `EngineCtx` hands out one short-lived borrow at a time.
        let draws: Vec<Transform> = ctx
            .world()
            .query::<&TransformComponent>()
            .iter()
            .map(|(_e, t)| t.transform)
            .collect();

        let renderer = ctx.renderer();
        renderer.begin_frame();
        renderer.set_pipeline(pipeline);
        for transform in &draws {
            renderer.draw_mesh(mesh, transform, &MaterialParams::default());
        }
        renderer.submit();
    }
}
```

Worked example: [game.rs](../games/playable-demo/src/game.rs) — the same three hooks, with lane
input, a state machine, streaming and collision hung off them.

---

## 3. Load an asset

Put committed `.gltf` / `.glb` files under your crate's `assets/` and import them in `init`
through [`AssetCache`](../crates/stickshift-assets/src/cache.rs#L43), as above. The cache dedups by
path, so one file parses and uploads exactly once no matter how many entities draw it. What comes
back is a `CachedAsset`: `mesh_handles` (one per mesh, already uploaded through the render seam)
and `scene`, the parsed node tree with per-mesh positions, UVs and any decoded base-color image.

Two rules the demo follows and you should too:

- **Never upload on the per-frame path.** Create meshes, textures and pipelines in `init`; `render`
  only references handles.
- **Keep asset tools out of the runtime.** If an asset is *derived* (baked from a photo, a font, a
  generator), write the baker as a one-shot `examples/gen_*.rs`, commit its output, and keep its
  crates in `[dev-dependencies]`. The demo bakes its road, skyline and font atlas this way, which
  is why `image` and `fontdue` never reach the shipped binary
  ([Cargo.toml](../games/playable-demo/Cargo.toml#L15)).

Worked example: [assets.rs](../games/playable-demo/src/assets.rs) (the two load paths plus the
fit transforms that place a model on the ground) and
[examples/](../games/playable-demo/examples).

---

## 4. Record a draw

The render seam is two traits, handed to you as one `&mut dyn Renderer` from `ctx.renderer()`:
[`RenderDevice`](../crates/stickshift-render-api/src/device.rs#L98) for resources (and
`surface_size` / `safe_area_insets`) and
[`FrameRecorder`](../crates/stickshift-render-api/src/recorder.rs#L63) for the frame. The frame
protocol is `begin_frame` → `set_pipeline` / `bind_texture` / `draw_mesh` /
`draw_overlay_quad` → `submit`, exactly once per `render` call.

You never set the view: the driver pushes the engine camera's view-projection across the seam
before `render` runs, so whatever pose the camera is in at the end of `update` is the pose the
frame is drawn from. Move the camera with `ctx.camera_mut()`; `stickshift_camera::ChaseController` is a
ready-made follow camera.

Group draws by pipeline and texture so each switch happens once per group rather than once per
entity. For a HUD, record [`OverlayQuad`](../crates/stickshift-render-api/src/overlay.rs#L71)s — the
backend flushes them in its own orthographic pass after every 3D draw, so they always composite
on top.

Worked example: [render.rs](../games/playable-demo/src/render.rs) (grouped passes) and
[hud.rs](../games/playable-demo/src/hud.rs) (SDF text, safe-area insets, no per-frame
allocation).

---

## 5. Run it — windowed and headless

`main.rs` needs both entry points. The windowed one injects the Metal backend through a factory
(which is how `stickshift-core` stays free of `metal`); the headless one needs no GPU at all.

```rust
fn main() {
    let mut game = MyGame::new();

    if std::env::args().any(|a| a == "--smoke") {
        // Headless: no window, no GPU, synthetic clock. Safe on CI.
        let harness = stickshift_core::headless::run(&mut game, 120);
        assert_eq!(harness.frames_run(), 120);
        println!("smoke: 120 frames OK");
        return;
    }

    #[cfg(target_os = "macos")]
    stickshift_core::run_with_backend(
        &mut game,
        Box::new(|window, width, height| {
            Box::new(stickshift_render::MetalRenderer::new(window, width, height))
                as Box<dyn stickshift_core::Renderer>
        }),
    );
    #[cfg(not(target_os = "macos"))]
    stickshift_core::run(&mut game);
}
```

```sh
cargo run -p my-game             # windowed, Metal backend
cargo run -p my-game -- --smoke  # headless: 120 frames, exits 0
```

Worked example: [main.rs](../games/playable-demo/src/main.rs), which does this with `clap` for
argument parsing and asserts the stdout contract line in a unit test.

### Graphics settings: the built-in menu, or your own UI

Every windowed run gets graphics settings for free. There are two ways to expose them to
the player, and both drive the **same** model, `stickshift_core::GraphicsSettings`:

| Setting | Options (default **bold**) | What it does |
|---|---|---|
| Shadows | Off · Low · **High** | Off skips the shadow pass; Low/High pick the shadow map (2048² / 4096² on Metal) |
| Shadow Distance | **Match Draw Distance** · Medium · Near | Shadows cover all, half, or a quarter of the draw distance |
| Draw Distance | Near · Medium · **Far** | 0.5× / 0.75× / 1× the streaming reach you set in `init`, with the fog scaled to match |
| Resolution | **50%** · 75% · 100% | Render scale of the window's native pixels (50% is one pixel per point on Retina) |

**Path 1: the built-in menu (default).** `run` / `run_with_backend` install a native **Graphics**
menu in the macOS menu bar, next to the standard application menu. It has one submenu per setting
plus *Enter Full Screen* (⌃⌘F) and *Reset to Defaults*. The runner also remembers the player's
choices across launches (`NSUserDefaults`). You write no code for this.

**Path 2: your own UI.** Turn the menu off with `RunConfig` and drive the settings from your
game, for example from an options screen drawn with the HUD overlay:

```rust
#[cfg(target_os = "macos")]
stickshift_core::run_with_config(
    &mut game,
    Some(Box::new(|window, width, height| {
        Box::new(stickshift_render::MetalRenderer::new(window, width, height))
            as Box<dyn stickshift_core::Renderer>
    })),
    stickshift_core::RunConfig {
        native_settings_menu: false,       // your UI replaces the menu
        ..stickshift_core::RunConfig::default() // keep persistence
    },
);
```

```rust
use stickshift_core::{DrawDistance, GraphicsSettings, ShadowQuality};

// Anywhere you have an `EngineCtx` (init / update / render):
let mut s: GraphicsSettings = ctx.graphics_settings();  // what is in effect now
s.shadows = ShadowQuality::Low;
s.draw_distance = DrawDistance::Medium;
ctx.set_graphics_settings(s);  // queued; applied at the start of the next frame
```

Each enum lists its options in menu order (`DrawDistance::ALL`, …) with a `label()`, so a custom
UI can enumerate them rather than hardcode strings.

Whichever path requests a change, the engine applies it identically:

1. At the start of the next frame, before any `update`, it pushes the render settings (shadow tier,
   shadow range, fog) across the render seam and rescales the scene's streaming reach.
2. It calls your `Game::graphics_settings_changed` hook. This is optional, and the default does
   nothing.
3. It resizes the drawable for the render scale.
4. It saves the settings, unless you also set `persist_graphics_settings: false`.

Two things to know:

- **Draw distance scales *your* reach.** The engine records the scene's `spawn_ahead` as it stands
  at the end of `init` as 100%. Set your reach with `Scene::set_config` in `init`, as the demo does,
  and don't change it later yourself.
- **Author HUD sizes in points.** Multiply them by `renderer.surface_scale()` (drawable pixels per
  window point) so text keeps its size at every render scale. See
  [hud.rs](../games/playable-demo/src/hud.rs).

Settings are testable headlessly too. `Headless::set_graphics_settings` queues a change exactly as
the menu would, and the `NullRenderer` records the `RenderSettings` it received. Nothing is ever
read from or written to the player's stored preferences in a headless run.

---

## 6. Test it without a GPU

This is the part worth adopting early. [`Headless`](../crates/stickshift-core/src/headless.rs#L93)
owns the engine state a `Game` runs against — scene, input, perf, accumulator, and a
`NullRenderer` — and steps frames on a synthetic clock. So gameplay is an ordinary unit test:

```rust
#[test]
fn the_entity_moves_forward() {
    use stickshift_core::headless::Headless;
    use stickshift_core::input::Key;

    let mut game = MyGame::new();
    let mut h = Headless::new();

    h.input_mut().press_key(Key::Space);   // seed input before a run
    h.run(&mut game, 1);
    h.input_mut().release_key(Key::Space);
    h.run(&mut game, 60);                  // one simulated second

    assert!(game.elapsed > 0.0);
    assert!(h.world().len() > 0);           // inspect the ECS world
    assert!(h.camera().position().z != 0.0); // …and the engine-owned camera
}
```

`init` runs on the first `run` only, so a test can step in chunks. Afterwards you can inspect the
world, the `Scene` (including `streamed_count()`), the `NullRenderer`'s recorded draws, and the
camera — which means even *view* behaviour is testable with no display.

For any of this to be reproducible, keep two rules: never read wall-clock time in `update` (the
engine's fixed `dt` is the only clock you need), and drive every random choice from a seeded PRNG.
Worked example: [rng.rs](../games/playable-demo/src/rng.rs), and the test module at
[game.rs:670](../games/playable-demo/src/game.rs#L670).

---

## 7. Stay on the right side of the boundary

Game concepts must not appear in engine crates — and that is enforced, not merely requested: four
`stickshift-*` crates carry self-scanning guard tests that fail if a game-named token appears anywhere
in their source (listed in [PLAYABLE_DEMO.md §7](PLAYABLE_DEMO.md#7-the-boundary-and-what-enforces-it)).

In practice this is not a constraint so much as a filing rule:

- Game-named ECS components go in *your* crate. They ride on the same entities as
  `TransformComponent` / `RenderComponent` / `PhysicsBodyComponent` and compose freely — see
  [components.rs](../games/playable-demo/src/components.rs).
- Tuning constants go in your crate, ideally all in one module — see
  [config.rs](../games/playable-demo/src/config.rs).
- If you find yourself wanting to add a game concept to `stickshift-scene`, `stickshift-ecs`,
  `stickshift-assets` or `stickshift-core`, the generic primitive you actually want is probably already
  there (a callback, a config field, a tag component). The demo needed none.
- Only the game binary may depend on `stickshift-render`, and only to build the backend it injects.

---

## See also

- [PLAYABLE_DEMO.md](PLAYABLE_DEMO.md) — the demo walked through in full.
- [ARCHITECTURE.md](ARCHITECTURE.md) — the constraints behind these seams. ·
  [DESIGN.md](DESIGN.md) — diagrams of the loop, the seam, streaming and asset load.
- Inline API docs: `cargo doc --workspace --no-deps --open`.
