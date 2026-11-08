// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The [`ImportError`] type returned by the glTF importer and the asset cache.

use std::fmt;

/// An error importing or caching a glTF asset.
#[derive(Debug)]
pub enum ImportError {
    /// The underlying `gltf` crate failed to read or parse the document.
    Gltf(gltf::Error),
    /// A mesh primitive had no `POSITION` attribute, so it cannot be imported.
    MissingPositions,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Gltf(e) => write!(f, "glTF import failed: {e}"),
            ImportError::MissingPositions => {
                write!(f, "glTF primitive is missing required POSITION data")
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
            ImportError::MissingPositions => None,
        }
    }
}

impl From<gltf::Error> for ImportError {
    fn from(e: gltf::Error) -> Self {
        ImportError::Gltf(e)
    }
}
