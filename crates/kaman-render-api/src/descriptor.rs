// Copyright (c) 2025 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Plain-data descriptors passed to the render seam.
//!
//! Everything in this module is **plain old data**: `#[derive]`-able structs and
//! enums that describe *what* a resource or a draw should look like, expressed in
//! backend-neutral terms. There are no GPU objects, no `metal` types, and no
//! lifetimes tied to a device here — a caller above the seam can freely construct,
//! copy, serialize, or compare these values.
//!
//! The render backend below the seam is responsible for translating these
//! descriptors into its own concrete objects (e.g. a [`VertexLayout`] into a
//! `metal::VertexDescriptor`, a [`VertexFormat`] into an `MTLVertexFormat`).

/// Scalar/vector format of a single vertex attribute.
///
/// Enumerates the small, backend-neutral set of formats the seam supports for
/// vertex data. A backend maps each variant to its native vertex format
/// (e.g. `Float32x3` → `MTLVertexFormat::Float3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    /// A single 32-bit float (1 × `f32`, 4 bytes).
    Float32,
    /// Two 32-bit floats (2 × `f32`, 8 bytes) — e.g. UV coordinates.
    Float32x2,
    /// Three 32-bit floats (3 × `f32`, 12 bytes) — e.g. position or normal.
    Float32x3,
    /// Four 32-bit floats (4 × `f32`, 16 bytes) — e.g. an RGBA colour or tangent.
    Float32x4,
}

impl VertexFormat {
    /// Size in bytes of a single value of this format.
    ///
    /// Useful for computing attribute offsets and validating a
    /// [`VertexLayout`]'s declared `stride`.
    #[must_use]
    pub const fn size_bytes(self) -> u32 {
        match self {
            VertexFormat::Float32 => 4,
            VertexFormat::Float32x2 => 8,
            VertexFormat::Float32x3 => 12,
            VertexFormat::Float32x4 => 16,
        }
    }
}

/// One named attribute within a vertex — its format and byte offset from the
/// start of the vertex.
///
/// `offset` is measured in bytes from the beginning of a vertex record; `format`
/// gives the attribute's type and (implicitly) its width via
/// [`VertexFormat::size_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    /// The shader binding location this attribute feeds.
    pub location: u32,
    /// Byte offset of this attribute from the start of the vertex record.
    pub offset: u32,
    /// The attribute's scalar/vector format.
    pub format: VertexFormat,
}

/// Backend-neutral description of a vertex buffer's memory layout.
///
/// Describes how one vertex is laid out in a tightly interleaved vertex buffer:
/// the total per-vertex `stride` in bytes and the ordered set of `attributes`
/// packed into it. A [`RenderDevice`](crate::RenderDevice) consumes this when
/// creating a pipeline so it can bind vertex data correctly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct VertexLayout {
    /// Total size in bytes of a single vertex record (the buffer stride).
    pub stride: u32,
    /// The attributes packed into each vertex, in declaration order.
    pub attributes: Vec<VertexAttribute>,
}

impl VertexLayout {
    /// Create a layout from a stride and a list of attributes.
    #[must_use]
    pub fn new(stride: u32, attributes: Vec<VertexAttribute>) -> Self {
        Self { stride, attributes }
    }
}

/// Per-draw material parameters supplied to
/// [`FrameRecorder::draw_mesh`](crate::FrameRecorder::draw_mesh).
///
/// A small, plain-data uniform block describing surface appearance in
/// backend-neutral terms. It intentionally carries no textures — those are bound
/// separately via [`FrameRecorder::bind_texture`](crate::FrameRecorder::bind_texture)
/// — only scalar/vector shading parameters that a backend copies into a uniform
/// buffer or argument buffer.
///
/// The default is an opaque, fully-white, dielectric surface (`base_color =
/// [1, 1, 1, 1]`, `metallic = 0`, `roughness = 1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialParams {
    /// Linear RGBA base colour / albedo, each channel in `0.0..=1.0`.
    pub base_color: [f32; 4],
    /// Metalness in `0.0..=1.0` (0 = dielectric, 1 = metal).
    pub metallic: f32,
    /// Perceptual roughness in `0.0..=1.0` (0 = mirror, 1 = fully rough).
    pub roughness: f32,
}

impl Default for MaterialParams {
    fn default() -> Self {
        Self {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 1.0,
        }
    }
}
