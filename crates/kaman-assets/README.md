# kaman-assets

Engine-generic **static-mesh asset import** for KamanEngine. Imports static
**glTF** (`.gltf` / `.glb`) files into engine-neutral geometry ready for the
render seam, with a load-once/dedup cache and an ECS spawn helper.

This crate is **metal-free** and **engine-generic**: it depends only on `gltf`,
`kaman-math`, `kaman-render-api`, and `kaman-ecs` — never on `metal`, and never on
any game crate. A unit test scans the source for game-specific concept words to
keep the boundary honest (mirroring `kaman-ecs`).

## Import → seam mapping

`import_gltf(path)` (or `import_slice(bytes)`) walks the glTF default scene and
produces a `SceneAsset`:

- **Meshes** (`SceneAsset::meshes`): each glTF primitive becomes a `MeshAsset`
  with its position / normal / UV read out of the buffers.
- **Node tree** (`SceneAsset::nodes` / `roots`): every node's transform is
  **baked into world space** (static v1 — no skinning, no animation), so a
  consumer needs no runtime hierarchy math.

Geometry is packed onto the canonical **`[position_xyz, normal_xyz, color_rgb]`**
render layout (`render_vertex_layout()`, a 36-byte stride matching the built-in
Phong pipeline), so an imported mesh renders immediately through the existing
untextured pipeline. The packed **color** comes from the primitive's material
base-color factor (KE-0703) — so a multi-material mesh (e.g. a car body + dark
wheels) renders each part's authored color — falling back to a neutral default
color when the primitive has no explicit material. Each `MeshAsset` carries:

| field       | meaning                                                        |
|-------------|----------------------------------------------------------------|
| `vertices`  | packed `[pos,normal,color]` bytes, ready for `MeshData`         |
| `indices`   | 32-bit triangle indices                                        |
| `layout`    | the `[pos,normal,color]` `VertexLayout`                        |
| `positions` / `normals` | parsed source attributes                          |
| `uvs`       | parsed texture coordinates — **retained for KE-0403** (not packed) |

The UVs are deliberately kept out of the packed render bytes; KE-0403's textured
pipeline builds a UV-carrying vertex buffer from them without re-parsing the file.

```rust
use kaman_assets::import_gltf;
use kaman_render_api::MeshData;

let scene = import_gltf("assets/cube.gltf")?;
let mesh = &scene.meshes[0];
let data = MeshData { vertices: &mesh.vertices, indices: &mesh.indices, layout: mesh.layout.clone() };
# Ok::<(), kaman_assets::ImportError>(())
```

## Load once, reference by handle

`AssetCache` parses (and optionally uploads) a file **once**, keyed by path, and
shares it across N instances — the "upload once, reference by handle" discipline
(KE-0103) applied to whole asset files.

- `load_parse_only(path)` — parse and cache; no GPU upload.
- `load(device, path)` — parse and upload the meshes into a `RenderDevice` once,
  caching the resulting `MeshHandle`s. Later loads of the same path return the
  cached handles with no re-parse / re-upload.

## Spawning ECS entities

`spawn_scene(world, &scene)` spawns one entity per mesh-bearing node
(`TransformComponent` + `RenderComponent` + `StaticTag`) from the baked node
graph. It lives in this crate (not a game) so any title reuses it; the
`kaman-assets → kaman-ecs` edge is acyclic (`kaman-ecs` never depends on
`kaman-assets`).

## The vertex-layout-through-the-seam fix

KE-0402 also made the `kaman-render` backend build its Metal vertex descriptor
**from** the `VertexLayout` in `MeshData` (mapping each attribute's
`location`/`offset`/`format`, including `Float32x2` for UVs, and taking the stride
from the layout) instead of a hardcoded 0/12/24 `Float3` triple. For the
`[pos,normal,color]` layout the mapping is byte-identical to the old descriptor,
so the box scene's pixel hash is unchanged.

## Regenerating the fixture

The committed cube fixture (`games/playable-demo/assets/cube.gltf`, and the test copy
`tests/fixtures/cube.gltf`) is produced by:

```sh
cargo run -p kaman-assets --example gen_cube -- games/playable-demo/assets/cube.gltf
```
