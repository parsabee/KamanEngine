# KamanEngine Architecture

Status: authoritative. Decisions here are intentional constraints, not defaults.

> For a **visual** walkthrough — component, UML class, and sequence diagrams — see
> [DESIGN.md](DESIGN.md). This document is the *why*; DESIGN.md is the *how it fits and moves*.

## 1. Targets and the rendering choice

KamanEngine targets **macOS and iPhone only**. On both, Metal is the single native GPU
API, so the engine uses **raw Metal** (`metal` crate) directly — this is *one* rendering
code path across both targets, not two, and it gives direct access to TBDR features
(memoryless attachments, tile memory) that portable abstractions expose late or not at all.

We deliberately give up portability to Windows/Linux/Android/Web. For an App Store product
this is the correct trade. If portability is ever required, the `kaman-render-api` trait
seam (below) is the single insertion point for an additional backend (e.g. wgpu) without
touching ECS, physics, scene, or scripting code.

## 2. The render seam (blast-radius firewall)

`kaman-render-api` defines only:
- Opaque resource handles (`MeshHandle`, `TextureHandle`, `PipelineHandle`).
- `RenderDevice` — resource create/destroy, called at load time.
- `FrameRecorder` — per-frame `set_view_projection` / `set_pipeline` / `bind` / `draw` / `submit`.

No crate above the seam may import `metal` types. This is enforced by crate boundaries so a
full renderer rewrite is provably contained.

**Camera crosses the seam as a matrix (KE-0205).** `FrameRecorder::set_view_projection(Mat4)`
carries the view — a plain `kaman_math::glam::Mat4`, **no GPU type**. The engine owns a
`kaman_camera::Camera`; the driver pushes its `view_projection_matrix()` through the seam once per
frame before `Game::render`, and the backend forms `mvp = view_proj * model` per draw. The value is
sticky (retained until replaced) so the engine's push survives the game's `begin_frame`. This
**replaced and deleted** the minimal camera KE-0102 had temporarily inlined into `kaman-render`
(`kaman-render/src/camera.rs`); the real camera now lives in `kaman-camera`, which also provides the
`ChaseController` follow camera the `car-runner` uses. `kaman-core → kaman-camera → kaman-math` keeps
the dependency direction acyclic and `kaman-core` metal-free.

As of KE-0102 the concrete backend exists **below** this seam: `kaman-render` provides
`MetalRenderer`, a raw-Metal type implementing both `RenderDevice` and `FrameRecorder` (and
therefore `kaman-core`'s `Renderer` marker). `kaman-render` is the *only* crate that depends on
`metal`. Crucially, `kaman-core` does **not** depend on `kaman-render`: the windowed entry
`kaman-core::run_with_backend` takes a backend **factory**
(`FnOnce(&Window, u32, u32) -> Box<dyn Renderer>`), and the game binary (`car-runner`, which does
depend on `kaman-render`) constructs the Metal backend and injects it. So `metal` reaches the
process only through `kaman-render` and the game binary — never through `kaman-core` — and the CI
firewall (`cargo tree -p kaman-core | grep metal` → nothing) still holds. The migrated renderer is
guarded by an offscreen render pixel-hash (KR1.3, `kaman-render/tests/pixel_hash.rs`).

**Flexible vertex layout through the seam (KE-0402).** The backend now builds its Metal vertex
descriptor **from the `VertexLayout` in `MeshData`** — mapping each `VertexAttribute`'s
`location`/`offset`/`format` (including `Float32x2` for UVs) and taking the buffer stride from
`layout.stride` — instead of a hardcoded `[pos,normal,color]` 0/12/24 `Float3` triple. `upload_mesh`
likewise deindexes at the layout's byte stride. For the `[pos,normal,color]` layout the resulting
descriptor and bytes are identical to the prior hardcoded path, so the KR1.3 pixel hash is unchanged
(`0x292c5df343b5eba8`). This lets arbitrary imported layouts flow through the one seam.

**Static glTF import (`kaman-assets`, KE-0402).** The new **engine-generic, metal-free**
`kaman-assets` crate imports static glTF (`.gltf`/`.glb`) into a `SceneAsset` (meshes + a baked node
tree) packed onto the render seam's `MeshData`/`VertexLayout`, with a load-once/dedup `AssetCache`
("upload once, reference by handle", one level up from KE-0103) and a `spawn_scene` ECS helper. It
depends only on `gltf`, `kaman-math`, `kaman-render-api`, and `kaman-ecs` — the `kaman-assets →
kaman-ecs` edge stays acyclic (`kaman-ecs` never depends on `kaman-assets`) and no `metal` enters the
tree. Imported geometry packs onto `[pos,normal,color]` with a default vertex color so it renders
through the existing pipeline immediately; parsed UVs are retained on `MeshAsset::uvs` for the KE-0403
textured pipeline. `car-runner` now loads its player mesh from a committed `assets/cube.gltf`.

**Modern-look rendering stack (KE-0401).** Below the seam, `kaman-render` applies a small,
mobile/TBDR-safe look stack — nothing above the seam learns about it. In pass order per frame:
(1) **sRGB + tonemap** — lighting is computed in linear space and presented through an explicit
ACES filmic tonemap + sRGB encode in the fragment shaders (`present_color`), so colors are correct
and not washed out even though both render targets are `*Unorm`; (2) a **gradient sky** fullscreen
triangle replaces the flat clear, drawn depth-test/write-disabled; (3) **distance fog** blends far
geometry into the sky horizon color, hiding the streaming spawn edge (pairs with KE-0203);
(4) a cheap **directional blob shadow** projected onto the ground plane grounds the car (no shadow
map). **MSAA** (4x) wraps all of it: the scene renders into a multisampled color + depth attachment
and resolves **in-tile** into the single-sample target via the `MultisampleResolve` store action, so
the multisampled buffers never spill to system memory. The MSAA color/depth attachments are
**memoryless-ready**: their storage mode is a single cfg hook (`MSAA_MEMORYLESS`, iOS ⇒
`Memoryless`), which **KE-0305** flips for iOS; macOS uses `Private`. The offscreen pixel-hash path
still produces a readable, resolved single-sample texture. **Bloom is deferred** (see the KE-0401
report). Because the look changed intentionally, both pixel-hash references were re-blessed.

## 3. Workspace layout

```
kaman-engine/
├── crates/
│   ├── kaman-math          # glam re-export + Ray/AABB/Frustum       (reuse)
│   ├── kaman-perf          # profiling / frame timing                (reuse)
│   ├── kaman-ecs           # hecs wrapper + components               (reuse)
│   ├── kaman-physics       # rapier3d wrapper (+ removal API)        (refactor)
│   ├── kaman-camera        # camera + controller math                (refactor)
│   ├── kaman-scene         # hecs World + PhysicsWorld + streaming   (refactor)
│   ├── kaman-render-api    # RenderDevice/FrameRecorder traits       (new seam)
│   ├── kaman-render        # raw-Metal impl of kaman-render-api      (refactor)
│   ├── kaman-core          # app/lifecycle + platform (#[cfg])       (refactor)
│   ├── kaman-assets        # glTF + textures                         (new)
│   └── kaman-script        # KamanScript lexer/parser/interpreter    (new)
├── games/
│   └── car-runner          # first title; only uses engine public API
├── shaders/                # MSL source → precompiled .metallib
└── platform/{macos,ios}    # thin app wrappers (iOS added in Phase 3)
```

Dependency direction is acyclic: `kaman-math` is a leaf; `kaman-core` and `games/*` sit at
the top. Game concepts (car, road, score) never enter engine crates — the boundary is a
`Game` trait + `EngineCtx` seam (Phase 1).

## 4. Engine stack

| Concern | Choice | Notes |
|---|---|---|
| Rendering | raw Metal (`metal`) | Apple-only; behind `kaman-render-api` |
| Windowing | `winit` (macOS) / UIKit+CADisplayLink (iOS) | `#[cfg]`-split in `kaman-core` |
| Math | `glam` | leaf crate |
| ECS | `hecs` | already integrated in the prototype |
| Physics | `rapier3d` **now**, custom arcade/spatial-query **later** | v1 keeps rapier + adds a removal API; arcade feel migrates to a custom query layer post-v1 |
| Audio | `kira` | Phase 4 |
| Assets | `gltf` + `image` | static meshes only for v1 |
| Scripting | KamanScript (custom) | frozen 20-construct spec → tree-walk interpreter → bytecode VM later; `mlua` is the fallback behind an `Interpreter` trait |

### 4a. Fixed-timestep loop (KE-0201)

`kaman-core` drives simulation on a **fixed timestep decoupled from the display
rate**. `FIXED_DT = 1/60 s` is the single source of truth (`kaman-core`'s
`timestep.rs`); physics (`kaman-physics`) steps at the same rate so its behavior
is deterministic. Each display frame:

1. banks the real elapsed time in an `Accumulator`,
2. calls `Game::update(ctx, FIXED_DT)` a whole number of times (0..N), draining
   the accumulator, then
3. calls `Game::render(ctx)` **once**, carrying an interpolation `alpha`
   (`remainder / FIXED_DT`, `0..1`) so rendering can lerp between fixed states.

This makes the `update` count per second of simulated time framerate-independent
(identical at 60 and 120 Hz). A **spiral-of-death guard** caps catch-up at
`MAX_STEPS_PER_FRAME` steps per frame and discards the surplus, so a stall slows
the sim rather than wedging the loop. The headless driver (synthetic clock, one
step/frame — deterministic tests + `--smoke`) and the winit windowed driver (real
monotonic clock, variable steps) share **one** loop implementation
(`driver::drive_frame`); only the clock source differs.

## 5. Physics decision (v1)

rapier3d is retained for v1 because it is already integrated and gets us to the "is it fun?"
gate fastest. v1 adds the missing **body/collider removal API** (required to despawn
obstacles in an infinite runner). The car's lateral/lane motion is **kinematic and
script-owned**, not solver-driven. A custom arcade-physics + spatial-query layer replaces
rapier on the shipping path in a later release; the `kaman-physics` wrapper hides rapier so
that swap does not ripple.

**Status (KE-0202):** the wrapper has migrated into `crates/kaman-physics` and the removal
API has landed — `PhysicsWorld::remove_body` (removes the body and its attached colliders)
and `remove_collider`. Both uphold a **stale-handle-safety** invariant: after removal every
query on the freed handle returns `None` / is a no-op and never derefs freed storage
(rapier's generational handles + tests enforce this). The companion **handle-ownership**
invariant — a handle in a `PhysicsBodyComponent` is removed from physics atomically with
clearing the component — is documented here and in the crate; the despawn orchestration that
upholds it is KE-0203. `PhysicsWorld::step` advances at the crate-local `FIXED_DT`, which
mirrors `kaman-core`'s `FIXED_DT` (`kaman-physics` does **not** depend on `kaman-core`, to
avoid a dependency cycle; a test pins the value).

**Status (KE-0203):** `crates/kaman-scene` now owns the hecs `World` + `PhysicsWorld` and adds
engine-generic **streaming** (spawn-ahead / despawn-behind by focus distance) and **floating-origin
rebase**. Despawn is **atomic** — the physics body is removed before the ECS entity, upholding the
handle-ownership invariant above. Rebase shifts ECS transforms and physics bodies together, only
**between** physics steps (`stream → step_physics → maybe_rebase`), so relative positions are
preserved and the solver never sees a mid-step discontinuity. `kaman-core` owns a `Scene` in its
loop and exposes it via `EngineCtx`; the dependency direction stays acyclic
(`kaman-core → kaman-scene → {kaman-ecs, kaman-physics, kaman-math}`).

## 6. Provenance

Modules originate from the `ProjectRigor` prototype and are migrated here under the process
in [INTEGRATION.md](INTEGRATION.md). Provenance is recorded in docs only; no `ProjectRigor`
identifiers remain in migrated code.
