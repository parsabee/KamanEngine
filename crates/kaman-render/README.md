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

## Sun, sky and camera come through the seam (KE-0406)

The backend chooses neither the camera nor the sun. Three sticky per-frame values
arrive through `FrameRecorder` and are folded into one `LightUniforms` block that is
uploaded **once per frame** and bound at fragment `[[buffer(0)]]` for every pass
that shades:

- `set_view_projection(Mat4)` — the world→clip matrix (KE-0205).
- `set_camera_position(Vec3)` — the eye in world space, so **specular is
  view-dependent**. It used to be a hardcoded `float3(0, 0, 1)`, which pinned every
  highlight to a fixed screen direction. The position is *pushed*, never recovered
  by inverting the view-projection.
- `set_sun_sky(&SunSky)` — the sun's **elevation and azimuth in degrees**, its
  colour and intensity, the ambient sky-fill level, and the sky gradient's
  zenith/horizon colours. Physical parameters only: the seam has no notion of time
  of day, so "a summer 4pm sun" is a game's policy (see `games/playable-demo`'s
  `config.rs`), expressed as angles.

The light direction is derived **once**, from `SunSky::direction()`, and that single
vector both shades the geometry and places the **sun disc + glow** the sky pass
draws — so turning the sun moves the lighting and the disc together. The sky is
still one fullscreen pass: it unprojects its own NDC through the camera's inverse
view-projection to get a per-pixel view ray, and the sun is one dot product plus two
`smoothstep`s. Azimuth is a compass bearing with `-Z` as north and `+X` as east; the
convention is documented and unit-tested in `kaman-render-api`'s `sun` module.

Before KE-0406 the light was written into a Metal buffer once at construction and
never updated, so nothing above the seam could set the sun at all. It is now written
into a small ring — one 256-byte slot per in-flight frame, selected by
`frame_index % MAX_FRAMES_IN_FLIGHT` — so the per-frame path still allocates nothing
and the frames-in-flight semaphore proves the CPU never rewrites a slot the GPU is
still reading.

KE-0406 also rebalanced the ambient term as **sky fill** (0.6 → 0.2 against an
unchanged diffuse 0.8): at the old balance an unlit face sat at 43% of a lit one and
every scene read overcast. Both 3D pixel-hash baselines were re-blessed for it.

## Shadows: a fitted shadow-map pass (KE-0407)

The sun casts real shadows. (Until KE-0407 the only "shadow" was a fake blob that
darkened a fixed circle around the world origin; it is deleted, uniforms and all.)

**Pass order.** Every frame is two render passes on one command buffer:

1. **Shadow pass** — depth-only, from the sun, into a 2048² `Depth32Float` shadow
   map. Every draw the frame recorded is a caster, rendered through a dedicated
   position-only vertex function (`shadow_vertex_main`) with no fragment stage.
   Depth is clamped rather than clipped, so casters nearer the sun than the near
   plane still cast.
2. **Scene pass** — the gradient sky, every draw through the lit untextured and
   textured pipelines (both sample the map at fragment `[[texture(1)]]`), then the 2D
   overlay.

To get every caster before the scene pass starts, the recorder **defers encoding**:
`draw_mesh` writes its uniforms into the ring and records the draw in a retained
list; `submit` replays that list into both passes. The list keeps its capacity, so
the steady state still allocates nothing.

**TBDR store actions.** The scene pass's MSAA attachments resolve in-tile and are
discarded (memoryless on iOS). The shadow map is the opposite: written by one pass
and sampled by the next, so it is `Private` (never `Memoryless`) with a `Store` depth
store action. Getting this wrong is silent — the scene would sample garbage — so the
texture's storage/usage is asserted at creation and the store action every time the
pass descriptor is built.

**Frustum fit and stability.** One map, not cascades: the fit covers only the camera
frustum cut at `fog_start + 2 / fog_density` view-depth units (where ground-level fog is
~98% opaque), capped at `MAX_SHADOW_DISTANCE` = 55. With the default fog (clear to 105,
density 1/30) the fog distance is 165, so the cap binds: shadows stay as sharp as they
were tuned (~4.7 cm texels) and fade out over the last 11 units of the slab instead of
spreading the map over ground three times as deep. The slab is enclosed in a
bounding sphere (whose size does not change as the camera turns), its radius rounded
up to 0.5 units, and its centre **snapped to whole shadow texels** in a light space
anchored at the world origin — so as the fit slides with the camera every world
point stays on the same sub-texel position and static shadow edges cannot shimmer.
Unit-tested GPU-free in `src/shadow.rs`. The light direction is the KE-0406
`SunSky::direction()` already in the light block — there is no second source of
truth — so turning the sun turns the shadows.

**Bias and filtering.** Each map texel the filter reads is compared against the
depth the **receiver's own triangle** has at that texel's centre (the plane is
reconstructed from screen-space derivatives), so a lit surface never shadows itself,
plus a constant **0.02 world units (2 cm)** for float error: under half a texel at the
demo's fit, and it moves a contact shadow only ~3 cm under a 32° sun, below one
texel. The filter is tent-weighted PCF over 4×4 texels (a ~4-texel penumbra). Full
reasoning in `src/shadow.rs`.

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

The raytracer is a **desktop-only, non-shipping** path. The module is gated
`#[cfg(all(feature = "raytracer", not(target_os = "ios")))]`, so even with the
feature enabled it compiles to nothing on iOS — an accidental feature enable can
never pull it into a mobile build. A default-build test (`raytracer_is_off_by_default`)
guards against the feature silently becoming a default.

Build/test it explicitly (macOS):

```text
cargo build -p kaman-render --features raytracer
cargo test  -p kaman-render --features raytracer
```

KE-0106 formalizes the gating and the iOS exclusion.

## Runtime shader compilation

Shaders compile at runtime via `include_str!` + `new_library_with_source`
(`shaders/rasterization.metal`). The `.metallib` precompile is KE-0107.

## Render pixel-hash guard (KR1.3)

`tests/pixel_hash.rs` renders fixed reference content into an offscreen Metal
texture, reads the pixels back, hashes it (FNV-1a 64-bit, no external crate), and
asserts the hash equals a committed baseline. This is the golden net that pins the
renderer's output so KE-0103/0104/0105 can prove they preserved behavior.

Four independent references are pinned in it, each with its own baseline (plus
`TEXTURED_REFERENCE_HASH`'s equivalent, `REFERENCE_HASH` in
`tests/textured_pixel_hash.rs`, for the textured pipeline):

| Constant | Guards |
| --- | --- |
| `REFERENCE_HASH` | The **3D reference scene**: a lit, rotated box through the Phong pipeline and the look stack (gradient sky, ACES tonemap, fog, shadow-map lookup, 4x MSAA). |
| `OVERLAY_REFERENCE_HASH` | The **2D overlay pass** (KE-0404): the pixel→NDC mapping and its `Y` flip, source-over blending in record order, and all three `OverlayFill` modes — solid, textured, and SDF. |
| `SUN_REFERENCE_HASH` | The **sun disc** and glow (KE-0406), in an otherwise empty frame. |
| `SHADOW_REFERENCE_HASH` | The **shadow map** (KE-0407): a floating cube casting onto a ground half drawn by the textured pipeline and half by the untextured one. Companion tests render it with the shadow pass's casters disabled (the frame must change, and darken exactly where the shadow is predicted) and with the sun turned (the shadow must move). |

**KE-0407** re-blessed `REFERENCE_HASH` and the textured baseline: deleting the
fake blob shadow (which was centred on the origin, where both reference meshes sit)
brightened them. Neither frame gains a real shadow — both re-render byte-identically
with the shadow pass's casters disabled, which is also the evidence that nothing
self-shadows (no acne). `OVERLAY_REFERENCE_HASH` and `SUN_REFERENCE_HASH` came back
unchanged.

The committed values live next to each constant in `tests/pixel_hash.rs`, with the
re-bless history and justification for every change — deliberately not duplicated
here, where a copy would go stale on each re-blessing.

The overlay reference draws no geometry and pushes no camera, so the two are truly
independent: a change to the 3D scene cannot move the overlay's pixels, or vice
versa. **KE-0406** leans on exactly that: it re-blessed the two 3D baselines (a
deliberate lighting change) and `OVERLAY_REFERENCE_HASH` came back **unchanged**,
which is the evidence that the new lighting did not leak into screen space.

Both go through `bless_or_assert`, which first refuses any frame with 8 or fewer
distinct pixel values. A render that silently drew *nothing* would otherwise hash
perfectly stably and, once blessed, pass forever while guarding nothing.

- **Re-bless (intended visual change only):**

  ```text
  BLESS=1 cargo test -p kaman-render --test pixel_hash -- --nocapture
  ```

  This prints each new hash **named by its constant** and passes without
  asserting; copy the one you intended to change into that constant **with a
  justification note in the commit message**, and leave the other alone. If a
  baseline you did not mean to touch comes back different, something unintended
  moved — investigate rather than pasting it. Blessing is deliberately manual and
  loud.
- **CI safety:** GitHub macOS runners are headless with no GPU, so
  `MTLCreateSystemDefaultDevice` can return nil. Both tests detect device absence
  and **skip** (print a skip line, return) instead of failing, so they never
  break a GPU-less build. On a real Mac they run and assert.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
