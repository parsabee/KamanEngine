// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! glTF (`.gltf` / `.glb`) import into a [`SceneAsset`].
//!
//! The importer walks the glTF default scene's node tree, **bakes** each node's
//! transform into world space (static v1: no runtime hierarchy math, no
//! skinning/animation), reads each mesh's positions/normals/UVs, and packs the
//! geometry onto the canonical `[pos,normal,color]` render layout with a default
//! vertex color. Parsed UVs are retained on [`MeshAsset::uvs`] for the textured pipeline.

use std::path::Path;

use kaman_math::glam::{Mat4, Vec3};

use crate::error::ImportError;
use crate::scene::{
    pack_render_vertices, pack_textured_vertices, render_vertex_layout, textured_vertex_layout,
    BaseColorTexture, MeshAsset, Node, SceneAsset, DEFAULT_IMPORT_COLOR,
};

/// Import a glTF or GLB file at `path` into a [`SceneAsset`].
///
/// Both text `.gltf` (with external or embedded buffers) and binary `.glb` are
/// supported — the `gltf` crate dispatches on the file. External buffer/`.bin`
/// resources are resolved relative to `path`.
///
/// # Errors
/// Returns [`ImportError`] if the file cannot be read or parsed, or if a mesh
/// primitive is missing, malformed or not made of triangles. Malformed input
/// never panics.
pub fn import_gltf(path: impl AsRef<Path>) -> Result<SceneAsset, ImportError> {
    let path = path.as_ref();
    let base = path.parent().unwrap_or_else(|| Path::new("./"));
    let file = std::fs::File::open(path).map_err(gltf::Error::Io)?;
    let gltf = gltf::Gltf::from_reader(std::io::BufReader::new(file))?;
    import_document(gltf, Some(base))
}

/// Import a glTF/GLB document from an in-memory byte slice.
///
/// The slice must be self-contained: a GLB, or a `.gltf` whose buffers and
/// images are embedded as `data:` URIs or buffer views. There is no path to
/// resolve external files against, so a reference to one is an error.
///
/// # Errors
/// Returns [`ImportError`] on a parse failure, an external file reference, or a
/// missing, malformed or non-triangle primitive. Malformed input never panics.
pub fn import_slice(bytes: &[u8]) -> Result<SceneAsset, ImportError> {
    import_document(gltf::Gltf::from_slice(bytes)?, None)
}

/// Load a parsed document's buffers and images, then build the scene.
///
/// `base` is the directory external files resolve against; `None` for a slice.
fn import_document(gltf: gltf::Gltf, base: Option<&Path>) -> Result<SceneAsset, ImportError> {
    let gltf::Gltf { document, blob } = gltf;
    let buffers = gltf::import_buffers(&document, base, blob)?;
    let images = import_images(&document, &buffers, base)?;
    build_scene(&document, &buffers, &images)
}

/// Decode every image the document declares.
///
/// `gltf::import_slice` refuses every image given by URI when it has no base
/// path, `data:` URIs included, so a self-contained `.gltf` with an embedded
/// texture could not be imported from memory. Here a `data:` URI decodes
/// without a base (nothing is read from disk for it), while any other URI still
/// needs one. An image stored in a buffer view is range-checked first, because
/// the `gltf` crate slices the buffer without checking.
fn import_images(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    base: Option<&Path>,
) -> Result<Vec<gltf::image::Data>, ImportError> {
    document
        .images()
        .map(|image| {
            let source = image.source();
            let base = match &source {
                gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
                    Some(base.unwrap_or_else(|| Path::new(".")))
                }
                gltf::image::Source::Uri { .. } if base.is_none() => {
                    return Err(gltf::Error::ExternalReferenceInSliceImport.into());
                }
                gltf::image::Source::Uri { .. } => base,
                gltf::image::Source::View { view, .. } => {
                    let buffer_len = buffers.get(view.buffer().index()).map_or(0, |b| b.0.len());
                    let in_range = view
                        .offset()
                        .checked_add(view.length())
                        .is_some_and(|end| end <= buffer_len);
                    if !in_range {
                        return Err(ImportError::InvalidPrimitive(format!(
                            "image {} runs past the end of its buffer",
                            image.index()
                        )));
                    }
                    base
                }
            };
            Ok(gltf::image::Data::from_source(source, base, buffers)?)
        })
        .collect()
}

/// Shared scene builder over a parsed document + its buffer and image data.
fn build_scene(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<SceneAsset, ImportError> {
    // 1. Import every unique mesh once (deduplicated by glTF mesh index). A glTF
    //    "mesh" may hold several primitives; each becomes its own MeshAsset, and
    //    we remember the primitive->MeshAsset index range per glTF mesh so nodes
    //    can reference them.
    let mut scene = SceneAsset::default();
    // For each glTF mesh index, the range of MeshAsset indices it produced.
    let mut mesh_ranges: Vec<std::ops::Range<usize>> = Vec::new();

    for mesh in document.meshes() {
        let start = scene.meshes.len();
        for primitive in mesh.primitives() {
            let asset = import_primitive(&primitive, buffers, images)?;
            scene.meshes.push(asset);
        }
        mesh_ranges.push(start..scene.meshes.len());
    }

    // 2. Walk the default scene (or the first scene) and bake the node tree.
    let gltf_scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    if let Some(gltf_scene) = gltf_scene {
        for node in gltf_scene.nodes() {
            let idx = bake_node(&node, Mat4::IDENTITY, None, &mesh_ranges, &mut scene);
            scene.roots.push(idx);
        }
    }

    Ok(scene)
}

/// Recursively bake one glTF node (and its subtree) into flat [`Node`]s, pushing
/// them onto `scene.nodes` and returning the pushed node's index.
///
/// A node with a mesh that expanded into multiple primitives is represented as a
/// single node pointing at the *first* primitive's MeshAsset, with the remaining
/// primitives attached as child nodes sharing the same baked transform — so every
/// primitive is drawn without losing the hierarchy.
fn bake_node(
    node: &gltf::Node,
    parent_world: Mat4,
    parent: Option<usize>,
    mesh_ranges: &[std::ops::Range<usize>],
    scene: &mut SceneAsset,
) -> usize {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_world * local;

    // Which MeshAsset indices this node's glTF mesh maps to (may be several).
    let primitive_indices: Vec<usize> = node
        .mesh()
        .and_then(|m| mesh_ranges.get(m.index()))
        .map(|r| r.clone().collect())
        .unwrap_or_default();

    let first_mesh = primitive_indices.first().copied();

    // Reserve this node's slot.
    let self_index = scene.nodes.len();
    scene.nodes.push(Node {
        world,
        mesh: first_mesh,
        children: Vec::new(),
        parent,
    });

    let mut children = Vec::new();

    // Extra primitives beyond the first become mesh-only child nodes sharing the
    // baked world transform (identity local, since `world` is already baked).
    for &extra in primitive_indices.iter().skip(1) {
        let child_index = scene.nodes.len();
        scene.nodes.push(Node {
            world,
            mesh: Some(extra),
            children: Vec::new(),
            parent: Some(self_index),
        });
        children.push(child_index);
    }

    // Recurse into real glTF children.
    for child in node.children() {
        let child_index = bake_node(&child, world, Some(self_index), mesh_ranges, scene);
        children.push(child_index);
    }

    scene.nodes[self_index].children = children;
    self_index
}

/// Read one glTF primitive into a [`MeshAsset`].
///
/// The primitive's material is inspected for a base-color texture (and, best
/// effort, a normal and metallic-roughness texture). If a base-color texture is
/// present the mesh is packed on the `[pos,normal,uv]`
/// [`textured_vertex_layout`] and the decoded RGBA8 image is attached; otherwise
/// it stays on the `[pos,normal,color]` layout, taking its per-vertex color from
/// the material's base-color factor (or the neutral default when the primitive
/// has no explicit material). UVs are retained on the asset either way.
fn import_primitive(
    primitive: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
) -> Result<MeshAsset, ImportError> {
    use gltf::accessor::{DataType, Dimensions};
    use gltf::Semantic;

    // The `gltf` reader trusts each accessor's type, shape and range, and panics
    // (or hits `unreachable!`) when they are wrong, so check them first.
    let position_accessor = primitive
        .get(&Semantic::Positions)
        .ok_or(ImportError::MissingPositions)?;
    check_accessor(
        &position_accessor,
        "POSITION",
        Dimensions::Vec3,
        &[DataType::F32],
    )?;
    if let Some(accessor) = primitive.get(&Semantic::Normals) {
        check_accessor(&accessor, "NORMAL", Dimensions::Vec3, &[DataType::F32])?;
    }
    if let Some(accessor) = primitive.get(&Semantic::TexCoords(0)) {
        check_accessor(
            &accessor,
            "TEXCOORD_0",
            Dimensions::Vec2,
            &[DataType::F32, DataType::U8, DataType::U16],
        )?;
    }
    if let Some(accessor) = primitive.indices() {
        check_accessor(
            &accessor,
            "indices",
            Dimensions::Scalar,
            &[DataType::U8, DataType::U16, DataType::U32],
        )?;
    }

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or(ImportError::MissingPositions)?
        .collect();

    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|iter| iter.collect());

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|tc| tc.into_f32().collect())
        .unwrap_or_default();

    // Indices: use the primitive's index buffer, or synthesize a trivial one
    // (0..n) for a non-indexed primitive.
    let indices: Vec<u32> = match reader.read_indices() {
        Some(idx) => idx.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    if let Some(&bad) = indices.iter().find(|&&i| i as usize >= positions.len()) {
        return Err(ImportError::InvalidPrimitive(format!(
            "index {bad} is past the last of {} vertices",
            positions.len()
        )));
    }
    let indices = triangle_list(primitive.mode(), indices)?;

    // Normalize per-vertex attribute lengths to the position count.
    let mut uvs = uvs;
    uvs.resize(positions.len(), [0.0, 0.0]);
    let (positions, normals, uvs, indices) = match normals {
        Some(mut normals) => {
            normals.resize(positions.len(), [0.0, 1.0, 0.0]);
            (positions, normals, uvs, indices)
        }
        // glTF 2.0: "When normals are not specified, client implementations MUST
        // calculate flat normals."
        None => flat_shaded(&positions, &uvs, &indices),
    };

    // Inspect the primitive's material for texture slots.
    let material = primitive.material();
    let pbr = material.pbr_metallic_roughness();
    let base_color_factor = pbr.base_color_factor();

    let base_color = pbr
        .base_color_texture()
        .and_then(|info| decode_texture(&info.texture(), images));
    let normal_map = material
        .normal_texture()
        .and_then(|info| decode_texture(&info.texture(), images));
    let metallic_roughness = pbr
        .metallic_roughness_texture()
        .and_then(|info| decode_texture(&info.texture(), images));

    // A base-color texture selects the `[pos,normal,uv]` textured layout so the
    // backend's textured pipeline can sample it; otherwise stay on the default
    // `[pos,normal,color]` path (untextured pixel-hash unaffected).
    let (vertices, layout) = if base_color.is_some() {
        (
            pack_textured_vertices(&positions, &normals, &uvs),
            textured_vertex_layout(),
        )
    } else {
        // Untextured path: an authored material contributes its base-color factor
        // as the packed per-vertex color, so a multi-material mesh (e.g. a body +
        // dark wheels) renders each part's color through the untextured pipeline.
        // A primitive with no explicit material (`index() == None`, glTF's default
        // white factor) keeps the neutral default color, so the untextured
        // fixtures' pixel-hash is unchanged.
        let color = if material.index().is_some() {
            [
                base_color_factor[0],
                base_color_factor[1],
                base_color_factor[2],
            ]
        } else {
            DEFAULT_IMPORT_COLOR
        };
        (
            pack_render_vertices(&positions, &normals, color),
            render_vertex_layout(),
        )
    };

    Ok(MeshAsset {
        vertices,
        indices,
        layout,
        positions,
        normals,
        uvs,
        base_color,
        base_color_factor,
        normal_map,
        metallic_roughness,
    })
}

/// A primitive's per-vertex attributes and its triangle-list indices:
/// `(positions, normals, uvs, indices)`.
type Attributes = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// Flat normals for a primitive that has none: every triangle gets its own three
/// vertices (positions and UVs copied from its corners) carrying the face normal
/// `normalize((b - a) × (c - a))`, so each face is lit as the plane it lies in.
/// Returns `(positions, normals, uvs, indices)` with `indices` simply `0..n`.
///
/// A degenerate (zero-area) triangle has no plane; it gets `+Y`, which is stable
/// and harmless since it covers no pixels.
fn flat_shaded(positions: &[[f32; 3]], uvs: &[[f32; 2]], indices: &[u32]) -> Attributes {
    let mut out_positions = Vec::with_capacity(indices.len());
    let mut out_normals = Vec::with_capacity(indices.len());
    let mut out_uvs = Vec::with_capacity(indices.len());
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[triangle[k] as usize]));
        let normal = (b - a)
            .cross(c - a)
            .try_normalize()
            .unwrap_or(Vec3::Y)
            .to_array();
        for &i in triangle {
            out_positions.push(positions[i as usize]);
            out_normals.push(normal);
            out_uvs.push(uvs[i as usize]);
        }
    }
    let out_indices = (0..out_positions.len() as u32).collect();
    (out_positions, out_normals, out_uvs, out_indices)
}

/// Check what the `gltf` reader assumes about an accessor before it reads it:
/// the element shape and component type, a non-zero count, a stride no smaller
/// than one element, and a byte range inside its buffer view.
fn check_accessor(
    accessor: &gltf::Accessor,
    what: &str,
    dimensions: gltf::accessor::Dimensions,
    data_types: &[gltf::accessor::DataType],
) -> Result<(), ImportError> {
    let invalid = |why: String| Err(ImportError::InvalidPrimitive(format!("{what} {why}")));
    if accessor.dimensions() != dimensions || !data_types.contains(&accessor.data_type()) {
        return invalid(format!(
            "accessor is {:?} {:?}, expected {dimensions:?} of {data_types:?}",
            accessor.data_type(),
            accessor.dimensions(),
        ));
    }
    let count = accessor.count();
    if count == 0 {
        return invalid("accessor has count 0".into());
    }
    if let Some(view) = accessor.view() {
        let size = accessor.size();
        let stride = view.stride().unwrap_or(size);
        if stride < size {
            return invalid(format!(
                "buffer view stride {stride} is smaller than one {size}-byte element"
            ));
        }
        let end = (count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(size))
            .and_then(|n| n.checked_add(accessor.offset()));
        if end.is_none_or(|end| end > view.length()) {
            return invalid("accessor runs past the end of its buffer view".into());
        }
    }
    Ok(())
}

/// Expand a primitive's indices into a plain triangle list.
///
/// Triangle lists pass through (their length must be a whole number of
/// triangles); strips and fans expand per the glTF 2.0 spec, keeping each
/// triangle's winding and dropping the zero-area triangles strips use for
/// stitching. Points and lines are rejected.
fn triangle_list(mode: gltf::mesh::Mode, indices: Vec<u32>) -> Result<Vec<u32>, ImportError> {
    use gltf::mesh::Mode;
    let list = match mode {
        Mode::Triangles => {
            if !indices.len().is_multiple_of(3) {
                return Err(ImportError::InvalidPrimitive(format!(
                    "{} triangle-list indices is not a whole number of triangles",
                    indices.len()
                )));
            }
            indices
        }
        // Triangle i is (v[i], v[i + 1 + i % 2], v[i + 2 - i % 2]).
        Mode::TriangleStrip => indices
            .windows(3)
            .enumerate()
            .map(|(i, w)| {
                if i % 2 == 0 {
                    [w[0], w[1], w[2]]
                } else {
                    [w[0], w[2], w[1]]
                }
            })
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .flatten()
            .collect(),
        // Triangle i is (v[i + 1], v[i + 2], v[0]).
        Mode::TriangleFan => match indices.split_first() {
            Some((&hub, rest)) => rest.windows(2).flat_map(|w| [w[0], w[1], hub]).collect(),
            None => Vec::new(),
        },
        Mode::Points => return Err(ImportError::UnsupportedPrimitiveMode("POINTS")),
        Mode::Lines => return Err(ImportError::UnsupportedPrimitiveMode("LINES")),
        Mode::LineLoop => return Err(ImportError::UnsupportedPrimitiveMode("LINE_LOOP")),
        Mode::LineStrip => return Err(ImportError::UnsupportedPrimitiveMode("LINE_STRIP")),
    };
    if list.is_empty() {
        return Err(ImportError::InvalidPrimitive(
            "primitive has no whole triangle".into(),
        ));
    }
    Ok(list)
}

/// Decode one glTF texture's source image into an RGBA8 [`BaseColorTexture`].
///
/// The `gltf` crate already decodes embedded/external images into
/// [`gltf::image::Data`] (raw pixels + a [`gltf::image::Format`]); this converts
/// whatever channel layout that is into tightly-packed RGBA8, the shape the
/// render seam's [`TextureData`](kaman_render_api::TextureData) accepts. Returns
/// `None` if the image index is out of range or the pixel data is malformed.
fn decode_texture(
    texture: &gltf::Texture,
    images: &[gltf::image::Data],
) -> Option<BaseColorTexture> {
    let data = images.get(texture.source().index())?;
    let rgba8 = to_rgba8(data)?;
    Some(BaseColorTexture {
        width: data.width,
        height: data.height,
        rgba8,
    })
}

/// Convert a decoded glTF image into tightly-packed RGBA8 bytes.
///
/// Handles the channel layouts the `gltf`/`image` decoders produce for the
/// common cases (RGB8/RGBA8, and single/dual-channel greyscale), expanding each
/// to 4 bytes per pixel with an opaque alpha default. Returns `None` if the pixel
/// buffer is too small for the declared dimensions.
fn to_rgba8(data: &gltf::image::Data) -> Option<Vec<u8>> {
    use gltf::image::Format;
    let px = (data.width as usize).checked_mul(data.height as usize)?;
    let src = &data.pixels;
    let mut out = Vec::with_capacity(px * 4);
    match data.format {
        Format::R8G8B8A8 => {
            if src.len() < px * 4 {
                return None;
            }
            out.extend_from_slice(&src[..px * 4]);
        }
        Format::R8G8B8 => {
            if src.len() < px * 3 {
                return None;
            }
            for c in src[..px * 3].chunks_exact(3) {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Format::R8G8 => {
            if src.len() < px * 2 {
                return None;
            }
            for c in src[..px * 2].chunks_exact(2) {
                out.extend_from_slice(&[c[0], c[0], c[0], c[1]]);
            }
        }
        Format::R8 => {
            if src.len() < px {
                return None;
            }
            for &g in &src[..px] {
                out.extend_from_slice(&[g, g, g, 255]);
            }
        }
        // 16-bit formats are uncommon for base-color glTF textures; downsample
        // each 16-bit channel to its high byte so we still upload something
        // sensible rather than dropping the texture.
        Format::R16 | Format::R16G16 | Format::R16G16B16 | Format::R16G16B16A16 => {
            let channels = match data.format {
                Format::R16 => 1,
                Format::R16G16 => 2,
                Format::R16G16B16 => 3,
                _ => 4,
            };
            if src.len() < px * channels * 2 {
                return None;
            }
            for pixel in src[..px * channels * 2].chunks_exact(channels * 2) {
                let hi = |i: usize| pixel[i * 2 + 1];
                let (r, g, b, a) = match channels {
                    1 => (hi(0), hi(0), hi(0), 255),
                    2 => (hi(0), hi(0), hi(0), hi(1)),
                    3 => (hi(0), hi(1), hi(2), 255),
                    _ => (hi(0), hi(1), hi(2), hi(3)),
                };
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        // 32-bit float formats: rare; clamp+scale each channel's first byte.
        Format::R32G32B32FLOAT | Format::R32G32B32A32FLOAT => {
            let channels = if matches!(data.format, Format::R32G32B32FLOAT) {
                3
            } else {
                4
            };
            if src.len() < px * channels * 4 {
                return None;
            }
            for pixel in src[..px * channels * 4].chunks_exact(channels * 4) {
                let ch = |i: usize| {
                    let f = f32::from_le_bytes([
                        pixel[i * 4],
                        pixel[i * 4 + 1],
                        pixel[i * 4 + 2],
                        pixel[i * 4 + 3],
                    ]);
                    (f.clamp(0.0, 1.0) * 255.0) as u8
                };
                let a = if channels == 4 { ch(3) } else { 255 };
                out.extend_from_slice(&[ch(0), ch(1), ch(2), a]);
            }
        }
    }
    Some(out)
}
