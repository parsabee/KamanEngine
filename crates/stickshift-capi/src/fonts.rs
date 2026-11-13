// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! SDF fonts: an atlas texture plus a glyph table, laid out with
//! [`FontAtlas`](kaman_render_api::FontAtlas).

use kaman_render_api::{ColorSpace, FontAtlas, Glyph};

use crate::context::KamanContext;
use crate::context::{with_context, OwnedEvent};
use crate::core::{call, input, output, utf8, Error};
use crate::types::*;

pub(crate) struct FontEntry {
    /// The C handle of the atlas texture, re-checked at every use: destroying
    /// the texture stops the font drawing.
    pub(crate) atlas_id: u64,
    /// The layout tables, holding the atlas's backend texture handle.
    pub(crate) atlas: FontAtlas,
}

impl From<KamanGlyph> for Glyph {
    fn from(g: KamanGlyph) -> Self {
        Self {
            uv: [g.u0, g.v0, g.u1, g.v1],
            size: [g.width, g.height],
            bearing: [g.bearing_x, g.bearing_y],
            advance: g.advance,
        }
    }
}

/// Create a font from an SDF atlas texture and a glyph table (copied).
///
/// # Safety
/// `ctx` must be NULL or a live context; `desc`, `completion` and `out` NULL or
/// valid; `desc->glyphs` valid for `desc->glyph_count` glyphs.
#[no_mangle]
pub unsafe extern "C" fn kaman_font_create(
    ctx: *mut KamanContext,
    desc: *const KamanFontDesc,
    completion: *const KamanCompletion,
    out: *mut KamanFont,
) -> KamanResult {
    call(|| {
        let desc = input(desc, "desc")?;
        let out = output(out, "out")?;
        let completion = completion.as_ref().copied();
        let first_char = char::from_u32(desc.first_codepoint).ok_or_else(|| {
            Error::invalid_argument(format!(
                "{:#x} is not a Unicode scalar value",
                desc.first_codepoint
            ))
        })?;
        if !(desc.line_height.is_finite() && desc.line_height >= 0.0) {
            return Err(Error::invalid_argument(
                "line_height must be finite and non-negative",
            ));
        }
        let glyphs: Vec<Glyph> = if desc.glyph_count == 0 {
            Vec::new()
        } else if desc.glyphs.is_null() {
            return Err(Error::null("glyphs"));
        } else {
            std::slice::from_raw_parts(desc.glyphs, desc.glyph_count as usize)
                .iter()
                .map(|&g| g.into())
                .collect()
        };
        let id = with_context(ctx, |inner| {
            let atlas = inner
                .textures
                .get(desc.atlas.id)
                .ok_or_else(|| Error::invalid_handle("atlas"))?;
            if atlas.color_space != ColorSpace::Linear {
                return Err(Error::invalid_argument(
                    "an SDF font atlas holds distances, not colour: create it with \
                     KAMAN_COLOR_SPACE_LINEAR",
                ));
            }
            let texture = atlas.handle;
            let id = inner
                .fonts
                .insert(FontEntry {
                    atlas_id: desc.atlas.id,
                    atlas: FontAtlas {
                        texture,
                        first_char,
                        glyphs,
                        line_height: desc.line_height,
                        fallback: desc.fallback.into(),
                    },
                })
                .map_err(|_| Error::new(KAMAN_ERROR_LIMIT_EXCEEDED, "too many live fonts"))?;
            inner.emit(
                OwnedEvent::new(KAMAN_EVENT_RESOURCE_READY, id, 0),
                completion,
            );
            Ok(id)
        })?;
        *out = KamanFont { id };
        Ok(KAMAN_OK)
    })
}

/// Destroy a font. Its atlas texture is not destroyed.
///
/// # Safety
/// `ctx` must be NULL or a live context.
#[no_mangle]
pub unsafe extern "C" fn kaman_font_destroy(
    ctx: *mut KamanContext,
    font: KamanFont,
) -> KamanResult {
    call(|| {
        with_context(ctx, |inner| {
            inner
                .fonts
                .remove(font.id)
                .map(|_| KAMAN_OK)
                .ok_or_else(|| Error::invalid_handle("font"))
        })
    })
}

/// The width of `text` at `px` pixels per em, into `*out_width`: the widest
/// line's, since `\n` starts a new line.
///
/// # Safety
/// `ctx` must be NULL or a live context; `text` NULL or NUL-terminated;
/// `out_width` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn kaman_font_measure(
    ctx: *mut KamanContext,
    font: KamanFont,
    text: *const std::ffi::c_char,
    px: f32,
    out_width: *mut f32,
) -> KamanResult {
    call(|| {
        let text = utf8(text, "text")?;
        let out_width = output(out_width, "out_width")?;
        with_context(ctx, |inner| {
            let font = inner
                .fonts
                .get(font.id)
                .ok_or_else(|| Error::invalid_handle("font"))?;
            *out_width = font.atlas.measure(text, px);
            Ok(KAMAN_OK)
        })
    })
}
