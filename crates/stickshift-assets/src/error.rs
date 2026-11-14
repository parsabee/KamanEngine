// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`ImportError`] type returned by the glTF importer and the asset cache.

use std::fmt;

/// An error importing or caching a glTF asset.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImportError {
    /// The underlying `gltf` crate failed to read or parse the document.
    Gltf(gltf::Error),
    /// A mesh primitive had no `POSITION` attribute, so it cannot be imported.
    MissingPositions,
    /// A mesh primitive draws points or lines (named here); only triangles,
    /// triangle strips and triangle fans can be imported.
    UnsupportedPrimitiveMode(&'static str),
    /// A mesh primitive's data is malformed: an accessor with the wrong type or
    /// shape, a count of zero or a range past its buffer view, an index past the
    /// last vertex, or no whole triangle. The string says which.
    InvalidPrimitive(String),
    /// The scene's node hierarchy is not a set of disjoint trees, as glTF
    /// requires: a node is its own ancestor (a cycle), has two parents, or is
    /// both a scene root and another node's child. The string says which node.
    InvalidHierarchy(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Gltf(e) => write!(f, "glTF import failed: {e}"),
            ImportError::MissingPositions => {
                write!(f, "glTF primitive is missing required POSITION data")
            }
            ImportError::UnsupportedPrimitiveMode(mode) => write!(
                f,
                "glTF primitive mode {mode} is not supported (only triangles, strips and fans)"
            ),
            ImportError::InvalidPrimitive(why) => write!(f, "malformed glTF primitive: {why}"),
            ImportError::InvalidHierarchy(why) => {
                write!(f, "malformed glTF node hierarchy: {why}")
            }
        }
    }
}

impl ImportError {
    /// Whether the import failed because a file could not be read (as opposed
    /// to a document that could not be parsed).
    #[must_use]
    pub fn is_io(&self) -> bool {
        matches!(self, ImportError::Gltf(gltf::Error::Io(_)))
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImportError::Gltf(e) => Some(e),
            _ => None,
        }
    }
}

impl From<gltf::Error> for ImportError {
    fn from(e: gltf::Error) -> Self {
        ImportError::Gltf(e)
    }
}
