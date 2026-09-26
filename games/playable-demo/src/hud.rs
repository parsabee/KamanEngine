// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The demo's **HUD** (KE-0707): a live score readout while playing, and a
//! game-over overlay reporting the run with a replay prompt.
//!
//! This is the worked example of "the game draws its UI through the engine": the
//! HUD owns no game logic — it reads [`CarRunner`]'s state and turns it into
//! [`OverlayQuad`]s via the engine's 2D overlay seam (KE-0404). The engine draws
//! them after the 3D scene, orthographic and alpha-blended, so the HUD always
//! composites on top.
//!
//! # Allocation-free
//!
//! Text is formatted into a fixed [`StackStr`] buffer and laid out with
//! [`FontAtlas::layout`], which emits one quad per glyph through a callback. No
//! per-frame heap allocation happens anywhere on this path (KR1.2).
//!
//! # Safe area
//!
//! Positions are derived from [`RenderDevice::surface_size`][surface_size] shrunk
//! by [`RenderDevice::safe_area_insets`][insets], so the HUD stays clear of notches and
//! rounded corners. On macOS the insets are zero; the iOS path reports real ones.
//!
//! [surface_size]: kaman_render_api::RenderDevice::surface_size
//! [insets]: kaman_render_api::RenderDevice::safe_area_insets

use std::fmt::Write as _;

use kaman_core::Renderer;
use kaman_render_api::{FontAtlas, Glyph, OverlayQuad, TextureData};

use crate::config::{
    HUD_BANNER_PX, HUD_BLACK_COLOR, HUD_DIM_COLOR, HUD_MARGIN, HUD_SCORE_PX, HUD_TEXT_COLOR,
    HUD_TITLE_PX,
};
use crate::game::{CarRunner, GameState};

/// A tiny fixed-capacity string, so the HUD can format numbers each frame without
/// touching the heap. Writes past `N` bytes are dropped rather than panicking —
/// HUD text is short and cosmetic, and a truncated readout beats a crash.
pub(crate) struct StackStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackStr<N> {
    /// An empty buffer.
    pub(crate) fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// The bytes written so far, as a string slice.
    pub(crate) fn as_str(&self) -> &str {
        // SAFETY-equivalent: only whole `&str` chunks are appended in `write_str`,
        // so the buffer is always valid UTF-8 up to `len`.
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> std::fmt::Write for StackStr<N> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        // Append only if the whole chunk fits, keeping the buffer valid UTF-8.
        let end = self.len + s.len();
        if end <= N {
            self.buf[self.len..end].copy_from_slice(s.as_bytes());
            self.len = end;
        }
        Ok(())
    }
}

/// Magic bytes at the head of the committed font binary.
const FONT_MAGIC: &[u8; 4] = b"KFNT";
/// Number of `f32`s per glyph record in the font binary.
const GLYPH_FLOATS: usize = 9;

/// Load the committed SDF font atlas (baked by the `gen_font` example) and upload
/// its texture, returning the [`FontAtlas`] the HUD lays text out with.
///
/// The binary is self-contained — metrics plus a single-channel distance field —
/// so the runtime needs neither a font parser nor an image decoder. The distance
/// channel is expanded to RGBA for upload (the overlay's SDF mode reads `.r`).
///
/// # Panics
/// If the file is missing or malformed: it is a committed asset, so a bad one is a
/// build error, not a runtime condition to recover from.
pub(crate) fn load_font(renderer: &mut dyn Renderer, path: &str) -> FontAtlas {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("read font atlas {path}: {e}"));
    assert!(
        data.len() >= 28 && &data[..4] == FONT_MAGIC,
        "{path} is not a KFNT font atlas",
    );

    let u32_at = |off: usize| {
        u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
    };
    let f32_at = |off: usize| f32::from_bits(u32_at(off));

    let width = u32_at(8);
    let height = u32_at(12);
    let first_char = char::from_u32(u32_at(16)).expect("font atlas has a valid first char");
    let count = u32_at(20) as usize;
    let line_height = f32_at(24);

    let glyphs_off = 28;
    let pixels_off = glyphs_off + count * GLYPH_FLOATS * 4;
    assert!(
        data.len() >= pixels_off + (width * height) as usize,
        "{path} is truncated",
    );

    let mut glyphs = Vec::with_capacity(count);
    for i in 0..count {
        let base = glyphs_off + i * GLYPH_FLOATS * 4;
        let f = |k: usize| f32_at(base + k * 4);
        glyphs.push(Glyph {
            uv: [f(0), f(1), f(2), f(3)],
            size: [f(4), f(5)],
            bearing: [f(6), f(7)],
            advance: f(8),
        });
    }

    // Expand the single-channel distance field to RGBA for the seam's texture
    // upload; the overlay's SDF mode samples the red channel.
    let field = &data[pixels_off..pixels_off + (width * height) as usize];
    let mut rgba = Vec::with_capacity(field.len() * 4);
    for &d in field {
        rgba.extend_from_slice(&[d, d, d, 255]);
    }
    let texture = renderer.create_texture(&TextureData {
        width,
        height,
        rgba8: &rgba,
    });

    FontAtlas {
        texture,
        first_char,
        glyphs,
        line_height,
        // Unknown characters advance like a space but draw nothing.
        fallback: Glyph {
            uv: [0.0; 4],
            size: [0.0, 0.0],
            bearing: [0.0, 0.0],
            advance: 0.25,
        },
    }
}

impl CarRunner {
    /// Draw this frame's HUD through the engine's overlay seam (KE-0707).
    ///
    /// While [`GameState::Playing`] this is just the live score in the top-left
    /// safe corner. On [`GameState::GameOver`] it dims the scene and centers a
    /// "GAME OVER" banner with the final score, the best so far, and the replay
    /// prompt. Called from the render pass with the frame already open.
    pub(crate) fn draw_hud(&self, renderer: &mut dyn Renderer) {
        let Some(font) = self.font.as_ref() else {
            return;
        };

        let (surface_w, surface_h) = renderer.surface_size();
        let (screen_w, screen_h) = (surface_w as f32, surface_h as f32);
        let [inset_top, inset_right, inset_bottom, inset_left] = renderer.safe_area_insets();
        let _ = inset_right;

        // Background wash, drawn FIRST so everything else composites over it
        // (overlay quads blend in record order):
        //   Ready    — opaque black, so the title screen hides the scene entirely.
        //   Playing  — the opening fade retiring that black over HUD_FADE_SECONDS.
        //   GameOver — a partial dim that keeps the crashed scene readable behind.
        let wash = match self.state {
            GameState::Ready => Some(HUD_BLACK_COLOR),
            GameState::Playing => {
                let alpha = self.fade_alpha();
                (alpha > 0.0).then(|| {
                    let mut c = HUD_BLACK_COLOR;
                    c[3] = alpha;
                    c
                })
            }
            GameState::GameOver => Some(HUD_DIM_COLOR),
        };
        if let Some(color) = wash {
            renderer.draw_overlay_quad(&OverlayQuad::solid(
                [0.0, 0.0, screen_w, screen_h],
                color,
            ));
        }

        // Live score, top-left of the safe area — only once a run is under way.
        // `layout`'s origin is the text baseline, so drop by one em to sit the
        // cap-height under the inset.
        if self.state != GameState::Ready {
            let mut score_text = StackStr::<32>::new();
            let _ = write!(score_text, "SCORE {}", self.score());
            font.layout(
                score_text.as_str(),
                [
                    inset_left + HUD_MARGIN,
                    inset_top + HUD_MARGIN + HUD_SCORE_PX,
                ],
                HUD_SCORE_PX,
                HUD_TEXT_COLOR,
                |quad| renderer.draw_overlay_quad(&quad),
            );
        }

        // Both frozen states center a banner over the wash above; only the lines
        // differ. `Playing` draws no banner at all.
        let mut final_text = StackStr::<32>::new();
        let mut best_text = StackStr::<32>::new();
        let banner: &[(&str, f32)] = match self.state {
            GameState::Playing => &[],
            GameState::Ready => &[
                ("KAMAN RUNNER", HUD_TITLE_PX),
                ("PRESS SPACE TO START", HUD_BANNER_PX),
            ],
            GameState::GameOver => {
                let _ = write!(final_text, "SCORE {}", self.score());
                let _ = write!(best_text, "BEST {}", self.best_score());
                &[
                    ("GAME OVER", HUD_TITLE_PX),
                    (final_text.as_str(), HUD_BANNER_PX),
                    (best_text.as_str(), HUD_BANNER_PX),
                    ("PRESS SPACE TO REPLAY", HUD_BANNER_PX),
                ]
            }
        };

        if banner.is_empty() {
            return;
        }

        // Stack the lines around the vertical center of the safe area, so the
        // block is centered rather than top-aligned.
        let center_y = inset_top + (screen_h - inset_top - inset_bottom) * 0.5;
        let block_h: f32 = banner.iter().map(|(_, px)| px * font.line_height).sum();
        let mut baseline = center_y - block_h * 0.5 + banner[0].1;

        for &(text, px) in banner {
            let width = font.measure(text, px);
            font.layout(
                text,
                [(screen_w - width) * 0.5, baseline],
                px,
                HUD_TEXT_COLOR,
                |quad| renderer.draw_overlay_quad(&quad),
            );
            baseline += px * font.line_height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use kaman_core::headless::Headless;
    use kaman_core::input::Key;
    use kaman_render_api::{OverlayFill, RenderDevice as _};

    use crate::config::HUD_FADE_SECONDS;

    #[test]
    fn stack_str_formats_without_allocating() {
        let mut s = StackStr::<32>::new();
        let _ = write!(s, "SCORE {}", 1234);
        assert_eq!(s.as_str(), "SCORE 1234");
    }

    #[test]
    fn stack_str_drops_overflow_instead_of_panicking() {
        // Capacity 8: the first write fits, the second does not and is dropped.
        let mut s = StackStr::<8>::new();
        let _ = write!(s, "12345678");
        let _ = write!(s, "overflow");
        assert_eq!(s.as_str(), "12345678", "the overflowing chunk is dropped");
    }

    // -----------------------------------------------------------------------
    // Headless HUD assertions (KE-0404 / KE-0707)
    //
    // These drive the *real* demo through `kaman_core::headless::Headless` and read
    // back what `draw_hud` recorded on the `NullRenderer`'s overlay stream — no GPU,
    // no window, fixed timestep, seeded PRNG, so every run is identical.
    //
    // `NullRenderer` accumulates overlay quads across every frame and never resets
    // them, so a single frame's HUD is the *tail* it appended. Every observation
    // therefore snapshots the count, drives exactly one frame, and takes what is
    // new (`hud_frame`) — the assertions are about per-frame structure, not a
    // running total.
    // -----------------------------------------------------------------------

    /// Drive exactly one frame and return the overlay quads *that frame* recorded,
    /// in record order.
    fn hud_frame(game: &mut CarRunner, h: &mut Headless) -> Vec<OverlayQuad> {
        let before = h.renderer().overlay_quad_count();
        h.run(game, 1);
        h.renderer().overlay_quads()[before..].to_vec()
    }

    /// Boot the demo headlessly and leave it on its title screen.
    ///
    /// Zero frames still runs [`kaman_core::Game::init`] (which loads the HUD font),
    /// so nothing has been rendered yet and the first `hud_frame` after this is
    /// genuinely the demo's first HUD frame.
    fn booted() -> (CarRunner, Headless) {
        let mut game = CarRunner::new();
        let mut h = Headless::new();
        h.run(&mut game, 0);
        assert_eq!(
            game.state,
            GameState::Ready,
            "the demo opens on its title screen",
        );
        (game, h)
    }

    /// Boot and tap the start key, leaving the demo in [`GameState::Playing`] —
    /// mirroring `game.rs`'s `started()` helper, and what a player does.
    fn started() -> (CarRunner, Headless) {
        let (mut game, mut h) = booted();
        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);
        h.input_mut().release_key(Key::Space);
        assert_eq!(
            game.state,
            GameState::Playing,
            "the start key begins the run",
        );
        (game, h)
    }

    /// The loaded font atlas, or a loud failure.
    ///
    /// `draw_hud` returns early when the atlas is missing, so every assertion below
    /// would "pass" by observing nothing. Failing here instead keeps that
    /// impossible.
    fn font_of(game: &CarRunner) -> &FontAtlas {
        game.font
            .as_ref()
            .expect("init loads the committed SDF atlas; without it draw_hud is a no-op")
    }

    /// How many quads laying `text` out emits: one per glyph with a non-empty quad.
    ///
    /// Mirrors [`FontAtlas::layout`]'s own skip rule (whitespace and zero-area
    /// glyphs emit nothing), so the expected counts below are *derived from the
    /// atlas* rather than hardcoded — they stay correct if the font is re-baked.
    fn visible_glyphs(font: &FontAtlas, text: &str) -> usize {
        text.chars()
            .filter(|&c| {
                let g = font.glyph(c);
                g.size[0] > 0.0 && g.size[1] > 0.0
            })
            .count()
    }

    /// Assert the frame's **first** recorded quad is the full-screen wash, and
    /// return its color.
    ///
    /// Record order *is* composite order for the overlay (it blends source-over in
    /// the order recorded), so the wash must come first or it paints over the text
    /// instead of behind it. Nothing about a single quad reveals that, and a
    /// reordering of `draw_hud` would break it silently — hence an explicit check
    /// in every state that draws a wash.
    fn assert_wash_first(quads: &[OverlayQuad], surface: (u32, u32), state: &str) -> [f32; 4] {
        assert!(
            !quads.is_empty(),
            "{state} records a background wash quad",
        );
        let wash = quads[0];
        assert_eq!(
            wash.fill,
            OverlayFill::Solid,
            "{state}: the wash is an untextured quad",
        );
        assert_eq!(
            wash.rect,
            [0.0, 0.0, surface.0 as f32, surface.1 as f32],
            "{state}: the wash covers the whole drawable",
        );
        assert!(
            quads[1..].iter().all(|q| q.fill != OverlayFill::Solid),
            "{state}: only the leading quad is the wash; everything after it is text",
        );
        wash.color
    }

    /// Assert every quad lands inside the drawable.
    ///
    /// The HUD derives its layout from `surface_size` shrunk by the safe-area
    /// insets, so a sign error there — or a banner that outgrows the screen — pushes
    /// text off the edge where a unit test of `layout` alone would never see it.
    fn assert_within_surface(quads: &[OverlayQuad], surface: (u32, u32), state: &str) {
        let (w, h) = (surface.0 as f32, surface.1 as f32);
        for (i, q) in quads.iter().enumerate() {
            let [x, y, qw, qh] = q.rect;
            assert!(
                qw > 0.0 && qh > 0.0,
                "{state}: quad {i} has an empty rect {:?}",
                q.rect,
            );
            assert!(
                x >= 0.0 && y >= 0.0 && x + qw <= w && y + qh <= h,
                "{state}: quad {i} at {:?} falls outside the {w}x{h} drawable",
                q.rect,
            );
        }
    }

    /// Whether `quad` sits in the top-left corner the live score is laid out in
    /// (one score line tall, left half of the screen).
    ///
    /// Score glyphs and banner glyphs share the same atlas, tint and fill mode, so
    /// position is the only thing that distinguishes them — and the corner is also
    /// exactly where the score is *supposed* to be.
    /// Whether `quad` sits in the top-left score corner rather than in the centered
    /// banner block — the test for "this is a score glyph".
    ///
    /// The bound is the top-left *region* (left half, top quarter), not exact glyph
    /// math. A glyph quad is not the glyph's ink: the baked atlas pads each cell for
    /// the distance field, so `'S'` at 30px reports `size.y = 1.0` em with
    /// `bearing.y = -0.854`, putting the box ~4px *below* the baseline it was laid
    /// out on. Asserting against the baseline would fail on that padding while the
    /// HUD is perfectly correct. The region test still distinguishes what matters —
    /// a score drawn centered, or down in the banner, lands nowhere near here.
    fn in_score_corner(quad: &OverlayQuad, surface: (u32, u32)) -> bool {
        let [x, y, _, h] = quad.rect;
        x >= 0.0
            && x < surface.0 as f32 * 0.5
            && y >= 0.0
            && y + h <= surface.1 as f32 * 0.25
    }

    #[test]
    fn the_hud_font_loads_against_the_headless_renderer() {
        // `draw_hud` bails out when `font` is `None`, so if the committed atlas did
        // not load headlessly every HUD test here would pass by drawing nothing.
        // `init` loads it unconditionally through the seam's `create_texture`, which
        // `NullRenderer` implements, so it *does* load — pinned here so the rest of
        // this module can never degrade into vacuous assertions unnoticed.
        let (mut game, mut h) = booted();
        {
            let font = font_of(&game);
            assert!(!font.glyphs.is_empty(), "the atlas carries glyph metrics");
            assert!(font.line_height > 0.0, "the atlas carries line metrics");
            assert!(
                visible_glyphs(font, "SCORE 0") > 0,
                "the atlas can actually draw the score readout",
            );
            // And its texture really went across the seam, so the SDF quads the HUD
            // records name a texture the backend handed out.
            assert!(
                h.renderer().created_textures().contains(&font.texture),
                "the atlas texture was uploaded through the render seam",
            );
        }

        let quads = hud_frame(&mut game, &mut h);
        assert!(
            quads.len() > 1,
            "the title screen records a wash plus glyph quads, not nothing",
        );
    }

    #[test]
    fn ready_washes_the_scene_black_and_draws_the_title_without_a_score() {
        let (mut game, mut h) = booted();
        let surface = h.renderer().surface_size();
        let quads = hud_frame(&mut game, &mut h);

        // The title screen must hide the scene *entirely*, so its wash is opaque
        // black; anything less and the frozen world shows through the menu.
        let wash = assert_wash_first(&quads, surface, "Ready");
        assert_eq!(wash, HUD_BLACK_COLOR, "Ready washes to opaque black");
        assert_within_surface(&quads, surface, "Ready");

        // Everything after the wash is a banner glyph, and the count matches the two
        // title lines exactly — which is what proves no score readout was drawn. An
        // exact count is stable here because both strings are fixed and the expected
        // value is derived from the atlas rather than hardcoded.
        let font = font_of(&game);
        let expected =
            visible_glyphs(font, "KAMAN RUNNER") + visible_glyphs(font, "PRESS SPACE TO START");
        assert_eq!(
            quads.len() - 1,
            expected,
            "Ready draws exactly the title + prompt banner, and no score",
        );
        assert!(
            !quads
                .iter()
                .any(|q| q.fill.is_sdf() && in_score_corner(q, surface)),
            "Ready draws no text in the live-score corner",
        );
    }

    #[test]
    fn playing_past_the_fade_draws_the_score_and_no_wash() {
        let (mut game, mut h) = started();
        let surface = h.renderer().surface_size();

        // Drive until the opening fade has fully retired (HUD_FADE_SECONDS of
        // simulated time on the fixed timestep) and take the first genuinely
        // fade-free frame. Stepping one frame at a time, rather than guessing a
        // frame count, keeps the run as short as possible: the demo's traffic is
        // deterministic and a longer drive eventually crashes into it.
        let mut frames = 0u32;
        let quads = loop {
            let frame = hud_frame(&mut game, &mut h);
            frames += 1;
            assert_eq!(
                game.state,
                GameState::Playing,
                "the run ended after {frames} frames, before the {}s opening fade retired — \
                 so this is not the fade-free Playing HUD under test",
                HUD_FADE_SECONDS,
            );
            if game.fade_alpha() == 0.0 {
                break frame;
            }
            assert!(
                frames < 600,
                "the fade never retired in {frames} frames (HUD_FADE_SECONDS = {})",
                HUD_FADE_SECONDS,
            );
        };

        // Fade gone ⇒ no wash at all: while driving, the scene must be fully visible.
        assert!(
            quads.iter().all(|q| q.fill != OverlayFill::Solid),
            "a fade-free Playing frame records no wash quad, only score glyphs",
        );
        assert_within_surface(&quads, surface, "Playing");

        // And the live score IS drawn — as SDF text (KE-0404), in the top-left safe
        // corner. `draw_hud` reads `score()` after this frame's update, so the score
        // the frame drew is exactly the one the game holds now.
        let font = font_of(&game);
        let text = format!("SCORE {}", game.score());
        assert_eq!(
            quads.len(),
            visible_glyphs(font, &text),
            "Playing draws exactly the live score readout ({text:?})",
        );
        assert!(
            quads.iter().all(|q| q.fill.is_sdf()),
            "score text is SDF glyphs, not solid quads",
        );
        assert!(
            quads.iter().all(|q| in_score_corner(q, surface)),
            "the live score sits in the top-left safe corner",
        );
    }

    #[test]
    fn the_opening_fade_wash_is_partial_and_retiring() {
        // Starting a run hands the title screen's black over to a timed fade. While
        // it plays there must still be a wash, but a *partial* one that weakens every
        // frame: one stuck at 1.0 leaves the player staring at black, and one that
        // jumped straight to 0 would pop.
        let (mut game, mut h) = started();
        let surface = h.renderer().surface_size();
        assert_eq!(game.fade_alpha(), 1.0, "the fade starts fully black");

        let mut previous = 1.0f32;
        for frame in 0..3 {
            let quads = hud_frame(&mut game, &mut h);
            assert_eq!(
                game.state,
                GameState::Playing,
                "still driving at fade frame {frame}",
            );

            let wash = assert_wash_first(&quads, surface, "Playing (mid-fade)");
            assert_within_surface(&quads, surface, "Playing (mid-fade)");
            assert_eq!(
                [wash[0], wash[1], wash[2]],
                [HUD_BLACK_COLOR[0], HUD_BLACK_COLOR[1], HUD_BLACK_COLOR[2]],
                "the fade varies only black's alpha, never its color",
            );
            assert!(
                wash[3] > 0.0 && wash[3] < 1.0,
                "fade frame {frame}: alpha is partial, got {}",
                wash[3],
            );
            assert!(
                wash[3] < previous,
                "fade frame {frame}: the fade retires ({} is not below {previous})",
                wash[3],
            );
            previous = wash[3];

            // The readout is already live under the fade — it is the wash that is
            // temporary, not the score.
            assert!(
                quads[1..].iter().any(|q| in_score_corner(q, surface)),
                "fade frame {frame}: the live score draws under the fade",
            );
        }
    }

    /// Drive from the first `Playing` frame until the deterministic traffic ends the
    /// run; return the game, the harness, the **last `Playing` frame's** quads and
    /// the **`GameOver` frame's** quads.
    ///
    /// The crash frame's `update` flips the state before `render` runs, so the frame
    /// the run ends on already draws the game-over overlay.
    fn drive_to_game_over() -> (CarRunner, Headless, Vec<OverlayQuad>, Vec<OverlayQuad>) {
        let (mut game, mut h) = started();
        let mut playing: Option<Vec<OverlayQuad>> = None;
        for _ in 0..600 {
            let quads = hud_frame(&mut game, &mut h);
            if game.state == GameState::GameOver {
                let playing = playing.expect("a Playing frame always precedes the crash frame");
                return (game, h, playing, quads);
            }
            playing = Some(quads);
        }
        panic!("the center runner never crashed in 600 frames");
    }

    #[test]
    fn game_over_dims_the_scene_and_adds_a_banner() {
        let (game, h, playing, quads) = drive_to_game_over();
        let surface = h.renderer().surface_size();

        // A *dim*, not the title screen's blackout: the crashed car has to stay
        // visible behind the banner, which is what the partial alpha buys.
        let wash = assert_wash_first(&quads, surface, "GameOver");
        assert_eq!(wash, HUD_DIM_COLOR, "GameOver dims rather than blacks out");
        assert!(
            wash[3] > 0.0 && wash[3] < 1.0,
            "the game-over wash is partial so the scene reads through it, got {}",
            wash[3],
        );
        assert_within_surface(&quads, surface, "GameOver");

        // The banner's four lines sit on top of the still-live score, so this frame
        // records strictly more quads than the Playing frame just before it.
        assert!(
            quads.len() > playing.len(),
            "the game-over banner adds quads ({} vs the preceding Playing frame's {})",
            quads.len(),
            playing.len(),
        );

        // Exactly: the live readout plus the four banner lines. Stable because the
        // strings are fixed (bar the two numbers, taken from the game) and the
        // per-string counts come from the atlas.
        let font = font_of(&game);
        let score = format!("SCORE {}", game.score());
        let best = format!("BEST {}", game.best_score());
        let expected: usize = [
            score.as_str(), // the live readout, still drawn in GameOver
            "GAME OVER",
            score.as_str(), // the banner's final score
            best.as_str(),
            "PRESS SPACE TO REPLAY",
        ]
        .iter()
        .map(|line| visible_glyphs(font, line))
        .sum();
        assert_eq!(
            quads.len() - 1,
            expected,
            "GameOver draws the live score plus the four banner lines",
        );

        // The banner is centred vertically rather than top-aligned: every glyph
        // outside the score corner lands in the middle half of the drawable.
        let banner: Vec<&OverlayQuad> = quads[1..]
            .iter()
            .filter(|q| !in_score_corner(q, surface))
            .collect();
        assert!(!banner.is_empty(), "the game-over banner is drawn");
        let height = surface.1 as f32;
        assert!(
            banner
                .iter()
                .all(|q| q.rect[1] >= height * 0.25 && q.rect[1] + q.rect[3] <= height * 0.75),
            "the banner block is centred, not top-aligned",
        );
    }

}
