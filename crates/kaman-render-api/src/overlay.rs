// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The engine-generic **2D overlay** (HUD) seam types (KE-0404).
//!
//! The overlay is a screen-space pass the backend draws **after** the 3D scene:
//! orthographic, no depth test or write, alpha-blended, so it composites on top of
//! whatever the scene rendered. A game records overlay draws during its frame with
//! [`FrameRecorder::draw_overlay_quad`](crate::FrameRecorder::draw_overlay_quad);
//! the backend batches and flushes them at
//! [`submit`](crate::FrameRecorder::submit).
//!
//! Everything here is **engine-generic plain data** — screen rectangles, UVs,
//! colors and glyph metrics. No game concepts, and no GPU or Metal type crosses
//! the seam.
//!
//! # Coordinates
//!
//! Overlay coordinates are **pixels with the origin at the top-left** of the
//! drawable, `+X` right and `+Y` down — the usual 2D UI convention, independent of
//! the 3D scene's world space. The drawable's size comes from
//! [`RenderDevice::surface_size`](crate::RenderDevice::surface_size), and the
//! region safe from notches/rounded corners from
//! [`RenderDevice::safe_area_insets`](crate::RenderDevice::safe_area_insets).

use crate::handles::TextureHandle;

/// How an [`OverlayQuad`] is filled.
///
/// The variants map to how the backend's overlay shader treats the sampled
/// texel: not at all, as straight color, or as a signed-distance field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFill {
    /// A flat, untextured quad — the quad's color is used directly. Useful for
    /// panels, bars and dimming the scene behind an overlay.
    Solid,
    /// Sample `texture` as ordinary RGBA and multiply by the quad's color.
    Textured(TextureHandle),
    /// Sample `texture` as a **signed-distance field** (distance in the red
    /// channel) and derive crisp, resolution-independent coverage from it,
    /// tinted by the quad's color. This is how text is drawn: one quad per glyph
    /// against an SDF font atlas.
    Sdf(TextureHandle),
}

impl OverlayFill {
    /// The texture this fill samples, if any.
    #[must_use]
    pub fn texture(&self) -> Option<TextureHandle> {
        match *self {
            OverlayFill::Solid => None,
            OverlayFill::Textured(t) | OverlayFill::Sdf(t) => Some(t),
        }
    }

    /// Whether this fill interprets its texture as a signed-distance field.
    #[must_use]
    pub fn is_sdf(&self) -> bool {
        matches!(self, OverlayFill::Sdf(_))
    }
}

/// One screen-space quad in the 2D overlay pass.
///
/// `rect` is `[x, y, width, height]` in **pixels, origin top-left**; `uv` is
/// `[u0, v0, u1, v1]` into the fill's texture (ignored for
/// [`OverlayFill::Solid`]); `color` is a linear RGBA tint multiplied into the
/// result, and carries the alpha used for blending.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayQuad {
    /// Destination rectangle in pixels: `[x, y, width, height]`, origin top-left.
    pub rect: [f32; 4],
    /// Source rectangle in the fill's texture: `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Linear RGBA tint (alpha participates in blending).
    pub color: [f32; 4],
    /// How the quad is filled.
    pub fill: OverlayFill,
}

impl OverlayQuad {
    /// A flat colored rectangle at `rect`.
    #[must_use]
    pub fn solid(rect: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            color,
            fill: OverlayFill::Solid,
        }
    }
}

/// One glyph's placement data in an [`FontAtlas`].
///
/// All values are in the atlas's own **em units** (i.e. relative to
/// [`FontAtlas::line_height`]-scaled text), so a caller scales them by a pixel
/// size at layout time and the same metrics work at any size — the point of an
/// SDF font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Source rectangle of this glyph in the atlas texture: `[u0, v0, u1, v1]`.
    pub uv: [f32; 4],
    /// Size of the glyph's quad in em units: `[width, height]`.
    pub size: [f32; 2],
    /// Offset from the cursor's pen position (baseline, left edge) to the quad's
    /// top-left corner, in em units: `[left, top]`. `top` is negative for the
    /// usual case of a glyph rising above the baseline.
    pub bearing: [f32; 2],
    /// How far to advance the pen after drawing this glyph, in em units.
    pub advance: f32,
}

/// A prebaked **SDF font atlas**: glyph metrics plus the texture they index into.
///
/// The atlas is engine-generic data — the backend never parses a font. A game (or
/// a build step) bakes an atlas, uploads the texture with
/// [`RenderDevice::create_texture`](crate::RenderDevice::create_texture), and
/// builds one of these to lay text out.
///
/// Only a contiguous ASCII range is supported (v1): `first_char ..=` the last
/// entry of `glyphs`. Characters outside the range fall back to
/// [`fallback`](Self::fallback).
#[derive(Debug, Clone)]
pub struct FontAtlas {
    /// The uploaded SDF atlas texture.
    pub texture: TextureHandle,
    /// The first character `glyphs[0]` describes.
    pub first_char: char,
    /// Per-character metrics, densely indexed from [`first_char`](Self::first_char).
    pub glyphs: Vec<Glyph>,
    /// Baseline-to-baseline distance in em units, for multi-line layout.
    pub line_height: f32,
    /// Metrics used for characters outside the atlas's range (typically a blank
    /// advance, so unknown characters render as a space rather than panicking).
    pub fallback: Glyph,
}

impl FontAtlas {
    /// The metrics for `c`, or [`fallback`](Self::fallback) if it is outside the
    /// atlas's range.
    #[must_use]
    pub fn glyph(&self, c: char) -> &Glyph {
        let first = self.first_char as u32;
        let code = c as u32;
        code.checked_sub(first)
            .and_then(|i| self.glyphs.get(i as usize))
            .unwrap_or(&self.fallback)
    }

    /// Width of `text` when laid out at `px` pixels per em, in pixels.
    ///
    /// Useful for right-aligning or centering a HUD string without allocating.
    #[must_use]
    pub fn measure(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.glyph(c).advance).sum::<f32>() * px
    }

    /// Lay `text` out and hand each glyph's quad to `emit`.
    ///
    /// `origin` is the pen start in pixels — the **left edge of the baseline** of
    /// the first line — `px` is the em size in pixels, and `color` tints every
    /// glyph. Newlines advance the pen by [`line_height`](Self::line_height).
    ///
    /// This **allocates nothing**: it walks the string and emits one
    /// [`OverlayQuad`] per visible glyph, so a HUD can draw straight into the
    /// recorder each frame. Whitespace and zero-area glyphs are skipped.
    pub fn layout(
        &self,
        text: &str,
        origin: [f32; 2],
        px: f32,
        color: [f32; 4],
        mut emit: impl FnMut(OverlayQuad),
    ) {
        let (mut pen_x, mut pen_y) = (origin[0], origin[1]);
        for c in text.chars() {
            if c == '\n' {
                pen_x = origin[0];
                pen_y += self.line_height * px;
                continue;
            }
            let g = self.glyph(c);
            if g.size[0] > 0.0 && g.size[1] > 0.0 {
                emit(OverlayQuad {
                    rect: [
                        pen_x + g.bearing[0] * px,
                        pen_y + g.bearing[1] * px,
                        g.size[0] * px,
                        g.size[1] * px,
                    ],
                    uv: g.uv,
                    color,
                    fill: OverlayFill::Sdf(self.texture),
                });
            }
            pen_x += g.advance * px;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> FontAtlas {
        // Two 1-em-wide glyphs starting at 'A', plus a blank fallback.
        let g = Glyph {
            uv: [0.0, 0.0, 0.5, 1.0],
            size: [0.5, 1.0],
            bearing: [0.0, -1.0],
            advance: 0.6,
        };
        FontAtlas {
            texture: TextureHandle(1),
            first_char: 'A',
            glyphs: vec![g, g],
            line_height: 1.25,
            fallback: Glyph {
                uv: [0.0; 4],
                size: [0.0, 0.0],
                bearing: [0.0, 0.0],
                advance: 0.5,
            },
        }
    }

    #[test]
    fn glyph_lookup_is_dense_from_first_char() {
        let a = atlas();
        assert_eq!(a.glyph('A').advance, 0.6);
        assert_eq!(a.glyph('B').advance, 0.6);
        // Outside the range falls back rather than panicking.
        assert_eq!(a.glyph('Z').advance, 0.5);
        assert_eq!(a.glyph(' ').advance, 0.5);
    }

    #[test]
    fn measure_sums_advances_scaled_by_size() {
        let a = atlas();
        // Two in-range glyphs at 0.6 em each, times 20px.
        assert!((a.measure("AB", 20.0) - 24.0).abs() < 1e-5);
    }

    #[test]
    fn layout_emits_one_quad_per_visible_glyph_and_advances_the_pen() {
        let a = atlas();
        let mut quads = Vec::new();
        a.layout("AB", [10.0, 100.0], 20.0, [1.0; 4], |q| quads.push(q));
        assert_eq!(quads.len(), 2);
        // First glyph sits at the pen; the second is one advance (0.6 * 20) later.
        assert!((quads[0].rect[0] - 10.0).abs() < 1e-5);
        assert!((quads[1].rect[0] - 22.0).abs() < 1e-5);
        // Bearing lifts the quad above the baseline.
        assert!((quads[0].rect[1] - 80.0).abs() < 1e-5);
        // Size scales with the em size.
        assert!((quads[0].rect[2] - 10.0).abs() < 1e-5);
    }

    #[test]
    fn layout_skips_blank_glyphs_but_still_advances() {
        let a = atlas();
        let mut quads = Vec::new();
        // ' ' is out of range → blank fallback: no quad, but the pen moves.
        a.layout("A A", [0.0, 0.0], 10.0, [1.0; 4], |q| quads.push(q));
        assert_eq!(quads.len(), 2, "the space emits no quad");
        // 0.6 (A) + 0.5 (space) = 1.1 em → 11px.
        assert!((quads[1].rect[0] - 11.0).abs() < 1e-5);
    }

    #[test]
    fn newline_resets_x_and_advances_the_line() {
        let a = atlas();
        let mut quads = Vec::new();
        a.layout("A\nB", [5.0, 50.0], 10.0, [1.0; 4], |q| quads.push(q));
        assert_eq!(quads.len(), 2);
        assert!((quads[1].rect[0] - 5.0).abs() < 1e-5, "x resets to the origin");
        // Baseline advances by line_height * px (50 + 12.5 = 62.5); the quad then
        // sits one em above it via the glyph's bearing (-1 em * 10px).
        assert!((quads[1].rect[1] - 52.5).abs() < 1e-5, "y advances by line_height * px");
    }

    #[test]
    fn fill_reports_texture_and_sdf_mode() {
        assert_eq!(OverlayFill::Solid.texture(), None);
        assert!(!OverlayFill::Solid.is_sdf());
        assert_eq!(
            OverlayFill::Sdf(TextureHandle(7)).texture(),
            Some(TextureHandle(7))
        );
        assert!(OverlayFill::Sdf(TextureHandle(7)).is_sdf());
        assert!(!OverlayFill::Textured(TextureHandle(7)).is_sdf());
    }
}
