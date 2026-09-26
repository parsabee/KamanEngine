# Build your own game on KamanEngine

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
kaman-assets.workspace = true
kaman-core.workspace = true
kaman-ecs.workspace = true
kaman-math.workspace = true
kaman-render-api.workspace = true
kaman-scene.workspace = true

# The Metal backend lives below the render seam. Only the game binary depends on
# `kaman-render` (and thus `metal`); it builds the backend and injects it, so
# `kaman-core` stays metal-free (ARCHITECTURE §2).
[target.'cfg(target_os = "macos")'.dependencies]
kaman-render.workspace = true
```

Every dependency is `*.workspace = true`, so versions come from
[the workspace dependency table](../Cargo.toml#L33) — never pin them per-crate. Take only the
crates you use; `kaman-camera` (for a follow camera) and `clap` (for CLI flags) are the usual
additions. Worked example: [games/playable-demo/Cargo.toml](../games/playable-demo/Cargo.toml).

---

## 2. Implement `Game`

One trait, three hooks: [`kaman_core::Game`](../crates/kaman-core/src/game.rs#L95). `init` runs
once; `update` runs 0..N times per frame with `dt` always equal to
[`FIXED_DT`](../crates/kaman-core/src/timestep.rs#L50) (1/60 s); `render` runs exactly once per
frame after that frame's updates. Advance state in `update`, record draws in `render`, and don't
cross the two — that split is what makes the simulation framerate-independent.

Everything the engine offers arrives through the [`EngineCtx`](../crates/kaman-core/src/context.rs#L77)
argument: the `Scene` (ECS world + physics + streaming), the ECS world, the render seam, the
`Camera`, an input snapshot, frame timing. There is nothing else to reach for, and no way to reach
past it.

A complete game that imports a model, spawns an entity, moves it, and draws it:

```rust
use kaman_assets::AssetCache;
use kaman_core::{EngineCtx, Game};
use kaman_ecs::TransformComponent;
use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render_api::{MaterialParams, MeshHandle, PipelineDescriptor, PipelineHandle};

/// Resolved against this crate's directory, so the path holds whatever the
/// process working directory is.
const MODEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/model.glb");

struct MyGame {
    /// Created once in `init`; only handles are kept.
    pipeline: Option<PipelineHandle>,
    mesh: Option<MeshHandle>,
    entity: Option<kaman_ecs::hecs::Entity>,
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
        // components belong in this crate, not in `kaman-ecs`.
        self.entity = Some(
            ctx.world_mut()
                .spawn((TransformComponent::from_position(Vec3::ZERO),)),
        );

        // Create GPU resources ONCE, here. Keep the opaque handles.
        let renderer = ctx.renderer();
        self.pipeline = Some(renderer.create_pipeline(&PipelineDescriptor {
            vertex_shader: "vertex_main".into(),
            fragment_shader: "fragment_main".into(),
            vertex_layout: kaman_assets::render_vertex_layout(),
        }));

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
through [`AssetCache`](../crates/kaman-assets/src/cache.rs#L43), as above. The cache dedups by
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
[`RenderDevice`](../crates/kaman-render-api/src/device.rs#L87) for resources (and
`surface_size` / `safe_area_insets`) and
[`FrameRecorder`](../crates/kaman-render-api/src/recorder.rs#L59) for the frame. The frame
protocol is `begin_frame` → `set_pipeline` / `bind_texture` / `draw_mesh` /
`draw_overlay_quad` → `submit`, exactly once per `render` call.

You never set the view: the driver pushes the engine camera's view-projection across the seam
before `render` runs, so whatever pose the camera is in at the end of `update` is the pose the
frame is drawn from. Move the camera with `ctx.camera_mut()`; `kaman_camera::ChaseController` is a
ready-made follow camera.

Group draws by pipeline and texture so each switch happens once per group rather than once per
entity. For a HUD, record [`OverlayQuad`](../crates/kaman-render-api/src/overlay.rs#L71)s — the
backend flushes them in its own orthographic pass after every 3D draw, so they always composite
on top.

Worked example: [render.rs](../games/playable-demo/src/render.rs) (grouped passes) and
[hud.rs](../games/playable-demo/src/hud.rs) (SDF text, safe-area insets, no per-frame
allocation).

---

## 5. Run it — windowed and headless

`main.rs` needs both entry points. The windowed one injects the Metal backend through a factory
(which is how `kaman-core` stays free of `metal`); the headless one needs no GPU at all.

```rust
fn main() {
    let mut game = MyGame::new();

    if std::env::args().any(|a| a == "--smoke") {
        // Headless: no window, no GPU, synthetic clock. Safe on CI.
        let harness = kaman_core::headless::run(&mut game, 120);
        assert_eq!(harness.frames_run(), 120);
        println!("smoke: 120 frames OK");
        return;
    }

    #[cfg(target_os = "macos")]
    kaman_core::run_with_backend(
        &mut game,
        Box::new(|window, width, height| {
            Box::new(kaman_render::MetalRenderer::new(window, width, height))
                as Box<dyn kaman_core::Renderer>
        }),
    );
    #[cfg(not(target_os = "macos"))]
    kaman_core::run(&mut game);
}
```

```sh
cargo run -p my-game             # windowed, Metal backend
cargo run -p my-game -- --smoke  # headless: 120 frames, exits 0
```

Worked example: [main.rs](../games/playable-demo/src/main.rs), which does this with `clap` for
argument parsing and asserts the stdout contract line in a unit test.

---

## 6. Test it without a GPU

This is the part worth adopting early. [`Headless`](../crates/kaman-core/src/headless.rs#L86)
owns the engine state a `Game` runs against — scene, input, perf, accumulator, and a
`NullRenderer` — and steps frames on a synthetic clock. So gameplay is an ordinary unit test:

```rust
#[test]
fn the_entity_moves_forward() {
    use kaman_core::headless::Headless;
    use kaman_core::input::Key;

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
[game.rs:668](../games/playable-demo/src/game.rs#L668).

---

## 7. Stay on the right side of the boundary

Game concepts must not appear in engine crates — and that is enforced, not merely requested: four
`kaman-*` crates carry self-scanning guard tests that fail if a game-named token appears anywhere
in their source (listed in [PLAYABLE_DEMO.md §7](PLAYABLE_DEMO.md#7-the-boundary-and-what-enforces-it)).

In practice this is not a constraint so much as a filing rule:

- Game-named ECS components go in *your* crate. They ride on the same entities as
  `TransformComponent` / `RenderComponent` / `PhysicsBodyComponent` and compose freely — see
  [components.rs](../games/playable-demo/src/components.rs).
- Tuning constants go in your crate, ideally all in one module — see
  [config.rs](../games/playable-demo/src/config.rs).
- If you find yourself wanting to add a game concept to `kaman-scene`, `kaman-ecs`,
  `kaman-assets` or `kaman-core`, the generic primitive you actually want is probably already
  there (a callback, a config field, a tag component). The demo needed none.
- Only the game binary may depend on `kaman-render`, and only to build the backend it injects.

---

## See also

- [PLAYABLE_DEMO.md](PLAYABLE_DEMO.md) — the demo walked through in full.
- [ARCHITECTURE.md](ARCHITECTURE.md) — the constraints behind these seams. ·
  [DESIGN.md](DESIGN.md) — diagrams of the loop, the seam, streaming and asset load.
- Inline API docs: `cargo doc --workspace --no-deps --open`.
