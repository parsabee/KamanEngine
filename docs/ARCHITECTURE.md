# KamanEngine Architecture

Status: authoritative. Decisions here are intentional constraints, not defaults.

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
- `FrameRecorder` — per-frame `set_pipeline` / `bind` / `draw` / `submit`.

No crate above the seam may import `metal` types. This is enforced by crate boundaries so a
full renderer rewrite is provably contained.

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

## 5. Physics decision (v1)

rapier3d is retained for v1 because it is already integrated and gets us to the "is it fun?"
gate fastest. v1 adds the missing **body/collider removal API** (required to despawn
obstacles in an infinite runner). The car's lateral/lane motion is **kinematic and
script-owned**, not solver-driven. A custom arcade-physics + spatial-query layer replaces
rapier on the shipping path in a later release; the `kaman-physics` wrapper hides rapier so
that swap does not ripple.

## 6. Provenance

Modules originate from the `ProjectRigor` prototype and are migrated here under the process
in [INTEGRATION.md](INTEGRATION.md). Provenance is recorded in docs only; no `ProjectRigor`
identifiers remain in migrated code.
