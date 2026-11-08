# kaman-render

The raw-**Metal** backend for KamanEngine: the concrete implementation that
lives *below* the render seam (`docs/ARCHITECTURE.md` §2). This is the **only**
engine crate that depends on `metal`; everything above the seam talks to the
`kaman-render-api` traits and never imports a Metal type.

Migrated from the prototype's rasterization renderer. It is rasterization-only.

## How it implements the seam

`MetalRenderer` implements **both** halves of the `kaman-render-api` contract, so
it automatically satisfies `kaman-core`'s `Renderer` marker via that crate's
blanket `impl<T: RenderDevice + FrameRecorder> Renderer for T`:

- **`RenderDevice`** (load-time): `create_mesh` uploads (deindexes) geometry into
  a **persistent** Metal vertex buffer **once** and stores it in a generational
  registry keyed by the returned `MeshHandle`; `create_texture` uploads an RGBA8
  texture with a full mip chain; `create_pipeline` selects one of the two built-in
  pipelines from the descriptor's vertex layout (`[pos,normal,color]` → the untextured
  Phong pipeline, `[pos,normal,uv]` → the textured one). `destroy_mesh` frees the slot
  and bumps its generation.
- **`FrameRecorder`** (per-frame): `begin_frame` acquires the color attachment (a
  drawable for a window, or an offscreen texture for tests) and opens a depth-tested
  render encoder; `draw_mesh(handle, transform, material)` **looks the persistent
  vertex buffer up by handle — no mesh allocation on the hot path** — computes an
  MVP from the seam-provided view-projection and the instance transform, writes it into
  the persistent **uniform ring** (no per-frame allocation), and records the draw;
  `submit` encodes the shadow pass (unless shadows are off) and the scene pass, and presents (window) or
  synchronizes for CPU readback (offscreen).

### Upload once, reference by handle

Mesh geometry is uploaded **once**, at load time (game/scene `init`), into a
persistent `MTLBuffer` and thereafter referenced by an opaque `MeshHandle`. The
handle packs a **slot index + generation**; `destroy_mesh` bumps the slot's
generation so any stale copy of the handle becomes detectably invalid.

**Stale-handle invariant:** a `draw_mesh` (or lookup) with a freed/old-generation
handle is a **defined no-op error** (`RegistryError::StaleHandle` /
`UnknownHandle`), never a silent draw of whatever buffer now occupies the reused
slot. Enforced by `tests/persistent_meshes.rs`.

**Allocation instrument:** `MetalRenderer::allocation_count()` counts every
`new_buffer*` the backend issues. `tests/persistent_meshes.rs` snapshots it around
the per-frame path and asserts the delta is **zero** after load: meshes are persistent
and per-draw uniforms live in the ring, so the steady state allocates no buffers. The
counter is public so the frames-in-flight tests can reuse it.

**Material.** The untextured pipeline takes its color from the per-vertex mesh data
(as the prototype did), so `MaterialParams` does not change its output. The textured
pipeline multiplies the sampled base color by `MaterialParams::base_color`. The
`metallic` and `roughness` fields are accepted but neither pipeline uses them.

### Behavior preservation

Making mesh upload persistent and moving per-draw uniforms into the ring changed no
rendered pixel: the pixel-hash guard held through both refactors. (Its baselines have
since been re-blessed for deliberate look changes; see below.)

## Sun, sky and camera come through the seam

The backend chooses neither the camera nor the sun. Three sticky per-frame values
arrive through `FrameRecorder` and are folded into one `LightUniforms` block that is
uploaded **once per frame** and bound at fragment `[[buffer(0)]]` for every pass
that shades:

- `set_view_projection(Mat4)` — the world→clip matrix.
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

Originally the light was written into a Metal buffer once at construction and
never updated, so nothing above the seam could set the sun at all. It is now written
into a small ring — one 256-byte slot per in-flight frame, selected by
`frame_index % MAX_FRAMES_IN_FLIGHT` — so the per-frame path still allocates nothing
and the frames-in-flight semaphore proves the CPU never rewrites a slot the GPU is
still reading.

Moving the sun into the seam also rebalanced the ambient term as **sky fill** (0.6 → 0.2 against an
unchanged diffuse 0.8): at the old balance an unlit face sat at 43% of a lit one and
every scene read overcast. Both 3D pixel-hash baselines were re-blessed for it.

## Shadows: a fitted shadow-map pass

The sun casts real shadows. (Before this pass the only "shadow" was a fake blob that
darkened a fixed circle around the world origin; it is deleted, uniforms and all.)

**Pass order.** Every frame is two render passes on one command buffer:

1. **Shadow pass** — depth-only, from the sun, into a 4096² `Depth32Float` shadow
   map (64 MiB, allocated once). Every draw the frame recorded is a caster, rendered through a dedicated
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
discarded (and will be memoryless on iOS, via the `MSAA_MEMORYLESS` hook, once iOS is
supported). The shadow map is the opposite: written by one pass
and sampled by the next, so it is `Private` (never `Memoryless`) with a `Store` depth
store action. Getting this wrong is silent — the scene would sample garbage — so the
texture's storage/usage is asserted at creation and the store action every time the
pass descriptor is built.

**Frustum fit and stability.** One map, not cascades: by default the fit covers the
camera frustum cut at the draw distance, `fog_start + 2 / fog_density` view-depth units
(where ground-level fog is ~98% opaque): 165 with the default fog (clear to 105, density
1/30). Shadows therefore reach as far as the visible world and fade out over the last 20%
of the slab (132 to 165), inside the fog. The map is 4096² so that slab still gets ~6.9 cm
texels (a 142-unit sphere); that is ~1.5× softer than the 2048², 55-unit fit it replaced.
**Shadow draw distance** is its own backend setting:
`MetalRenderer::set_shadow_distance(None)` (the default) matches the draw distance, and
`Some(d)` fixes a shorter slab (smaller texels, so sharper shadows). The slab is clamped
to at most the fog's distance and at least 1 unit. Like `set_shadows_enabled`, it sits
below the render seam. Games reach it through the seam's `RenderSettings` (see below).
The slab is enclosed in a
bounding sphere (whose size does not change as the camera turns), its radius rounded
up to 0.5 units, and its centre **snapped to whole shadow texels** in a light space
anchored at the world origin — so as the fit slides with the camera every world
point stays on the same sub-texel position and static shadow edges cannot shimmer.
Unit-tested GPU-free in `src/shadow.rs`. The light direction is the
`SunSky::direction()` already in the light block — there is no second source of
truth — so turning the sun turns the shadows.

**Bias and filtering.** Each map texel the filter reads is compared against the
depth the **receiver's own triangle** has at that texel's centre (the plane is
reconstructed from screen-space derivatives), so a lit surface never shadows itself,
plus a constant **0.02 world units (2 cm)** for float error: ~0.29 of a ~6.9 cm texel
at the demo's fit, so under half a texel. It moves a contact shadow only ~3 cm under a
32° sun, also under half a texel. The filter is tent-weighted PCF over 4×4 texels (a
~4-texel, ~28 cm penumbra). Full
reasoning in `src/shadow.rs`.

## Graphics settings through the seam

`RenderDevice::set_render_settings(&RenderSettings)` is how the engine's graphics settings (the
built-in Graphics menu, or a game's own UI) reach the backend. Every field is relative to the
backend's own look defaults:

- **`shadows: ShadowQuality`**. `High` is the 4096² map. `Low` re-creates it at 2048²
  (`SHADOW_MAP_SIZE_LOW`); only a size change allocates. `Off` skips the shadow pass in `submit` and
  zeroes `shadow_strength`, so the lit shaders ignore the map. Command buffers retain what they
  reference, so replacing the map cannot disturb a frame in flight.
- **`shadow_distance: Option<f32>`** is a fraction of the draw distance, resolved to world units
  into `set_shadow_distance`. `None` matches the draw distance.
- **`draw_distance_scale`** multiplies the fog's start and its opaque depth (`start × s`,
  `density ÷ s`), always from the defaults captured at build, so repeated changes never compound.

`RenderDevice::resize_surface(width, height, pixels_per_point)` sizes the `CAMetalLayer`'s drawable
explicitly for the render-scale setting; CoreAnimation scales it to the window. The MSAA color and
depth targets follow the drawable's size lazily. `surface_scale()` reports `pixels_per_point`,
which is the HUD's UI scale. Tested in `tests/render_settings.rs`.

## Wiring: metal stays out of `kaman-core`

`kaman-core` does **not** depend on this crate. Instead, `kaman-core::run_with_backend`
takes a **backend factory** (`FnOnce(&Window, u32, u32) -> Box<dyn Renderer>`). The
game binary (which *does* depend on `kaman-render`) constructs a `MetalRenderer` in
the factory and hands it to the engine; the factory runs once in `resumed`, after
the window is created. The headless driver keeps the GPU-free `NullRenderer`. This
keeps `metal` out of `kaman-core`'s dependency tree (CI-enforced).

## The camera is not here

The backend has no camera of its own: it receives the world→clip matrix and the eye
position through the seam each frame. The camera type and the chase controller live in
`kaman-camera`.

## Runtime shader compilation

Shaders compile at runtime via `include_str!` + `new_library_with_source`
(`shaders/rasterization.metal`). An off-by-default `precompiled-shaders` feature compiles
them to a `.metallib` at build time instead (it needs full Xcode); making that the default
is planned for v0.1.0.

## Render pixel-hash guard

`tests/pixel_hash.rs` renders fixed reference content into an offscreen Metal
texture, reads the pixels back, hashes it (FNV-1a 64-bit, no external crate), and
asserts the hash equals a committed baseline. This is the golden net that pins the
renderer's output so refactors (persistent buffers, the uniform ring, frames in
flight) can prove they preserved behavior.

Four independent references are pinned in it, each with its own baseline (plus
`TEXTURED_REFERENCE_HASH`'s equivalent, `REFERENCE_HASH` in
`tests/textured_pixel_hash.rs`, for the textured pipeline):

| Constant | Guards |
| --- | --- |
| `REFERENCE_HASH` | The **3D reference scene**: a lit, rotated box through the Phong pipeline and the look stack (gradient sky, ACES tonemap, fog, shadow-map lookup, 4x MSAA). |
| `OVERLAY_REFERENCE_HASH` | The **2D overlay pass**: the pixel→NDC mapping and its `Y` flip, source-over blending in record order, and all three `OverlayFill` modes — solid, textured, and SDF. |
| `SUN_REFERENCE_HASH` | The **sun disc** and glow, in an otherwise empty frame. |
| `SHADOW_REFERENCE_HASH` | The **shadow map**: a floating cube casting onto a ground half drawn by the textured pipeline and half by the untextured one. Companion tests render it with the shadow pass's casters disabled (the frame must change, and darken exactly where the shadow is predicted) and with the sun turned (the shadow must move). |

The **shadow-map pass** re-blessed `REFERENCE_HASH` and the textured baseline: deleting the
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
versa. Moving the **sun into the seam** leaned on exactly that: it re-blessed the two 3D baselines (a
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
- **No GPU, no failure:** when `MTLCreateSystemDefaultDevice` returns nil the tests
  detect it and **skip** (print a skip line, return) instead of failing, so they never
  break a GPU-less build. GitHub's macOS runners do expose a (paravirtual) Metal
  device, so in CI the tests run and assert, as they do on a real Mac.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
