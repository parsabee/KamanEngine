# kaman-render

The raw-**Metal** backend for KamanEngine: the concrete implementation that
lives *below* the render seam (`docs/ARCHITECTURE.md` §2). This is the **only**
engine crate that depends on `metal`; everything above the seam talks to the
`kaman-render-api` traits and never imports a Metal type.

Migrated from the prototype's rasterization renderer in KE-0102.

## How it implements the seam

`MetalRenderer` implements **both** halves of the `kaman-render-api` contract, so
it automatically satisfies `kaman-core`'s `Renderer` marker via that crate's
blanket `impl<T: RenderDevice + FrameRecorder> Renderer for T`:

- **`RenderDevice`** (load-time): `create_mesh` uploads (deindexes) geometry into
  a **persistent** Metal vertex buffer **once** and stores it in a generational
  registry keyed by the returned `MeshHandle`; `create_texture` uploads an RGBA8
  texture; `create_pipeline` names the built-in Phong pipeline (this Phase-1 port
  has a single pipeline). `destroy_mesh` frees the slot and bumps its generation.
- **`FrameRecorder`** (per-frame): `begin_frame` acquires the color attachment (a
  drawable for a window, or an offscreen texture for tests) and opens a depth-tested
  render encoder; `draw_mesh(handle, transform, material)` **looks the persistent
  vertex buffer up by handle — no mesh allocation on the hot path** — computes an
  MVP from the backend camera and the instance transform, writes a per-draw uniform
  buffer (the one remaining per-frame allocation, removed by KE-0104), and records a
  triangle draw; `submit` ends encoding and presents (window) or synchronizes for
  CPU readback (offscreen).

### Upload once, reference by handle (KE-0103)

Mesh geometry is uploaded **once**, at load time (game/scene `init`), into a
persistent `MTLBuffer` and thereafter referenced by an opaque `MeshHandle`. The
handle packs a **slot index + generation**; `destroy_mesh` bumps the slot's
generation so any stale copy of the handle becomes detectably invalid.

**Stale-handle invariant:** a `draw_mesh` (or lookup) with a freed/old-generation
handle is a **defined no-op error** (`RegistryError::StaleHandle` /
`UnknownHandle`), never a silent draw of whatever buffer now occupies the reused
slot. Enforced by `tests/persistent_meshes.rs`.

**Allocation instrument (KR1.2):** `MetalRenderer::allocation_count()` counts every
`new_buffer*` the backend issues. `tests/persistent_meshes.rs` snapshots it around
the per-frame path and asserts **zero mesh uploads** occur after load (the only
per-frame allocation left is one uniform buffer per draw). The counter is public so
KE-0104 (uniform ring) and KE-0105 (frames-in-flight) can reuse it.

The `draw_mesh` path is a faithful port of the prototype's
`render_with_transforms_and_colors`. Color comes from the per-vertex mesh data
(as in the prototype), so `MaterialParams` is accepted but does not yet drive the
raster color.

### Behavior preservation (KE-0102 / KE-0103)

KE-0103 removed per-frame **mesh** allocation: geometry is uploaded once and kept
in the registry, so the hot draw path allocates no vertex buffers. The one per-frame
allocation still kept on purpose is the per-draw uniform buffer (KE-0104 replaces it
with a ring); the depth texture is the one cached resource (as in the prototype).
Making mesh upload persistent does not change any rendered pixel, so the KE-0102
pixel-hash guard (`0x292c5df343b5eba8`) stays valid and unchanged.

## Wiring: metal stays out of `kaman-core`

`kaman-core` does **not** depend on this crate. Instead, `kaman-core::run_with_backend`
takes a **backend factory** (`FnOnce(&Window, u32, u32) -> Box<dyn Renderer>`). The
game binary (which *does* depend on `kaman-render`) constructs a `MetalRenderer` in
the factory and hands it to the engine; the factory runs once in `resumed`, after
the window is created. The headless driver keeps the GPU-free `NullRenderer`. This
keeps `metal` out of `kaman-core`'s dependency tree (CI-enforced).

## Camera (temporary inline)

`kaman-camera` is still a stub and Phase 1 has no camera ticket, so a **minimal**
view/projection `Camera` is inlined here to place the reference scene. A later
camera-migration ticket should replace it with the real crate and delete
`src/camera.rs`.

## Ray tracer (feature `raytracer`, off by default)

The ray-tracing code that was entangled in the prototype renderer is behind the
off-by-default `raytracer` cargo feature (`src/raytracer.rs` + `shaders/raytracing.metal`).
The **default** build is rasterization-only and never compiles `raytracing.metal`.
Build/test it explicitly:

```text
cargo build -p kaman-render --features raytracer
cargo test  -p kaman-render --features raytracer
```

KE-0106 formalizes the gating and the iOS exclusion.

## Runtime shader compilation

Shaders compile at runtime via `include_str!` + `new_library_with_source`
(`shaders/rasterization.metal`). The `.metallib` precompile is KE-0107.

## Render pixel-hash guard (KR1.3)

`tests/pixel_hash.rs` renders a fixed reference scene into an offscreen Metal
texture, reads the pixels back, hashes them (FNV-1a 64-bit, no external crate),
and asserts the hash equals the committed `REFERENCE_HASH`. This is the golden net
that pins the migrated renderer's output so KE-0103/0104/0105 can prove they
preserved behavior.

- **Committed baseline:** `0x292c5df343b5eba8`, blessed from this migrated
  renderer's first correct frame.
- **Re-bless (intended visual change only):**

  ```text
  BLESS=1 cargo test -p kaman-render --test pixel_hash -- --nocapture
  ```

  This prints the new hash and passes without asserting; copy it into
  `REFERENCE_HASH` **with a justification note in the commit message**. Blessing is
  deliberately manual and loud.
- **CI safety:** GitHub macOS runners are headless with no GPU, so
  `MTLCreateSystemDefaultDevice` can return nil. The test detects device absence
  and **skips** (prints a skip line, returns) instead of failing, so it never
  breaks a GPU-less build. On a real Mac it runs and asserts.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
