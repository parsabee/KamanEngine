// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`RenderError`]: why a fallible backend call failed.
//!
//! The `try_*` constructors and resource calls on
//! [`MetalRenderer`](crate::MetalRenderer) return these instead of panicking, so a
//! caller that cannot unwind (the C API) can report the failure. The infallible
//! seam methods ([`RenderDevice`](stickshift_render_api::RenderDevice)) keep their
//! signatures and panic with the same message where the input is a caller error.

use std::fmt;

/// The largest texture (and offscreen target) edge, in pixels, the backend
/// accepts: the Apple-GPU limit for 2D textures on every Apple Silicon family.
pub const MAX_TEXTURE_DIMENSION: u32 = 16384;

/// Why a fallible backend call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    /// No Metal device is available (no GPU, or Metal is unavailable).
    NoDevice,
    /// The surface to render into is unusable: a null or non-`CAMetalLayer`
    /// pointer, or a window that is not an AppKit window.
    InvalidSurface(String),
    /// A width or height is zero or above [`MAX_TEXTURE_DIMENSION`].
    InvalidSize {
        /// The requested width, in pixels.
        width: u32,
        /// The requested height, in pixels.
        height: u32,
    },
    /// Pixel data does not match the texture's size.
    InvalidTexture(String),
    /// Mesh data is malformed: its layout is not one of the built-in pipelines',
    /// its byte length is not a whole number of vertices, or an index is out of range.
    InvalidMesh(String),
    /// A resource would exceed a device limit (e.g. `maxBufferLength`).
    LimitExceeded(String),
    /// The built-in shaders failed to compile or load.
    ShaderCompile(String),
    /// A built-in shader function or pipeline state could not be created.
    PipelineCreation(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no Metal device is available"),
            Self::InvalidSurface(why) => write!(f, "invalid surface: {why}"),
            Self::InvalidSize { width, height } => write!(
                f,
                "invalid size {width}x{height}: each side must be 1..={MAX_TEXTURE_DIMENSION}"
            ),
            Self::InvalidTexture(why) => write!(f, "invalid texture data: {why}"),
            Self::InvalidMesh(why) => write!(f, "invalid mesh data: {why}"),
            Self::LimitExceeded(why) => write!(f, "limit exceeded: {why}"),
            Self::ShaderCompile(why) => write!(f, "shader compile failed: {why}"),
            Self::PipelineCreation(why) => write!(f, "pipeline creation failed: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Check a width/height pair against `1..=MAX_TEXTURE_DIMENSION`.
pub(crate) fn check_size(width: u32, height: u32) -> Result<(), RenderError> {
    if (1..=MAX_TEXTURE_DIMENSION).contains(&width) && (1..=MAX_TEXTURE_DIMENSION).contains(&height)
    {
        Ok(())
    } else {
        Err(RenderError::InvalidSize { width, height })
    }
}

/// The number of RGBA8 bytes a `width` × `height` image needs, or `None` if
/// that overflows `usize`.
pub(crate) fn rgba8_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_inside_the_limit_pass_and_zero_or_oversized_fail() {
        assert!(check_size(1, 1).is_ok());
        assert!(check_size(MAX_TEXTURE_DIMENSION, MAX_TEXTURE_DIMENSION).is_ok());
        assert_eq!(
            check_size(0, 4),
            Err(RenderError::InvalidSize {
                width: 0,
                height: 4
            })
        );
        assert!(check_size(4, MAX_TEXTURE_DIMENSION + 1).is_err());
    }

    #[test]
    fn rgba8_len_is_four_bytes_per_pixel_and_never_overflows() {
        assert_eq!(rgba8_len(2, 3), Some(24));
        assert_eq!(rgba8_len(0, 9), Some(0));
        assert_eq!(rgba8_len(u32::MAX, u32::MAX).map(|_| ()), {
            // On 64-bit targets this fits; the point is that it is computed with
            // checked arithmetic rather than wrapping.
            (u32::MAX as usize)
                .checked_mul(u32::MAX as usize)
                .and_then(|p| p.checked_mul(4))
                .map(|_| ())
        });
    }

    #[test]
    fn display_names_the_problem() {
        assert!(RenderError::NoDevice
            .to_string()
            .contains("no Metal device"));
        assert!(RenderError::InvalidSize {
            width: 0,
            height: 1
        }
        .to_string()
        .contains("0x1"));
    }
}
