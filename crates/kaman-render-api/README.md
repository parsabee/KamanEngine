# kaman-render-api

The **render seam** for KamanEngine — a backend-agnostic contract that a concrete
renderer implements, so no crate above it ever touches a GPU type.

## The seam (blast-radius firewall)

`docs/ARCHITECTURE.md` §2 makes the render backend a swappable implementation
detail. KamanEngine renders with **raw Metal**, but ECS, scene, physics, and
scripting must not depend on that choice. This crate is the single insertion
point between them: it defines *only* a contract, and the Metal renderer
(`kaman-render`, KE-0102) implements the contract rather than exporting Metal
types.

Because everything above the seam binds to these traits and handles — never to
`metal` — a full renderer rewrite (or an additional backend, e.g. wgpu for
portability) is **provably contained** to the implementing crate.

## The "no Metal above this line" rule

- `kaman-render-api` depends on `std` and [`kaman-math`] (for `Transform`) and
  **nothing else**. It must never list `metal` in its `Cargo.toml`.
- CI enforces this: after the build, a step asserts `metal` is absent from this
  crate's dependency tree
  (`! cargo tree -p kaman-render-api -e normal | grep -qi metal`).
- Handles are **opaque** newtypes over a `u32`. The wrapped value is the
  backend's private identity (typically a slot in a resource table); code above
  the seam stores and passes handles without ever learning what they point at on
  the GPU.

## Contract surface

### Opaque handles

`MeshHandle`, `TextureHandle`, `PipelineHandle` — `Copy + Clone + Eq + Hash +
Debug` newtypes. Returned by `RenderDevice::create_*`, consumed by the recorder
and destroy calls. Unique among live resources of the same kind; not unique
across kinds.

### `RenderDevice` — load-time resource ownership

Creates and destroys meshes, textures, and pipelines from **plain-data**
descriptors (`MeshData`, `TextureData`, `PipelineDescriptor`), handing back
opaque handles. The device owns the GPU resources; handles are non-owning
references valid until the matching `destroy_*`.

### `FrameRecorder` — per-frame command recording

`begin_frame` → `set_pipeline` (≥1, before the first draw) → any mix of
`bind_texture` / `draw_mesh(handle, transform, material)` → `submit`. Per-frame
state (bound pipeline/texture) does not carry across a `begin_frame`/`submit`
boundary. `draw_mesh` takes a [`kaman-math`] `Transform` and a plain-data
`MaterialParams`.

### Plain-data descriptors

`VertexFormat`, `VertexAttribute`, `VertexLayout` describe vertex memory layout;
`MaterialParams` is a small backend-neutral shading block. No GPU objects, no
`metal` types, no device-tied lifetimes — freely constructed, copied, and
compared above the seam. The backend translates them into native objects.

## `NullRenderer` — headless test double

`NullRenderer` implements **both** traits without a GPU. It hands out sequential
handles and **records** every call so scene/game code can be unit-tested
headlessly:

- Resources: `created_meshes()`, `created_textures()`, `created_pipelines()`,
  `destroyed_*()`, and `live_*_count()`.
- Frames: `draws()` / `draw_count()` (each `RecordedDraw` captures the mesh plus
  the pipeline/texture bound at record time, the transform, and the material),
  `frames_begun()`, `frames_submitted()`, `frame_open()`.

It never panics on protocol violations — it records the observed behaviour so a
test can assert it. This is the mechanism by which the whole engine above the
seam is testable with no Metal device.

[`kaman-math`]: ../kaman-math

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
