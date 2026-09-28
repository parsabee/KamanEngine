// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Engine **graphics settings** (KE-0408): shadows, shadow distance, draw
//! distance, and render resolution, as user-facing presets.
//!
//! [`GraphicsSettings`] is the one model every settings UI drives:
//!
//! - **The built-in menu.** On macOS the windowed runner installs a native
//!   "Graphics" menu in the menu bar by default. Each item just requests a new
//!   `GraphicsSettings`.
//! - **A game's own UI.** Opt out of the menu with
//!   [`RunConfig::native_settings_menu`](crate::RunConfig::native_settings_menu),
//!   then read the current value with
//!   [`EngineCtx::graphics_settings`](crate::EngineCtx::graphics_settings) and
//!   request a change with
//!   [`EngineCtx::set_graphics_settings`](crate::EngineCtx::set_graphics_settings)
//!   from any hook.
//!
//! Either way the change takes the same path:
//!
//! 1. It is queued and applied at the start of the next frame, before any
//!    `update`.
//! 2. The engine pushes the renderer half across the seam as a
//!    [`RenderSettings`] and scales the scene's streaming reach.
//! 3. It then calls [`Game::graphics_settings_changed`](crate::Game::graphics_settings_changed).
//! 4. In a windowed run it also resizes the drawable for the render scale and,
//!    unless opted out, saves the settings for the next launch.
//!
//! # What each setting does
//!
//! - [`ShadowQuality`]: `Off` skips the shadow pass. `Low` and `High` pick the
//!   shadow map's resolution; the backend owns the texel counts (2048² and 4096²
//!   on Metal).
//! - [`ShadowDistance`]: how far shadows reach, as a fraction of the draw distance.
//!   The default, `MatchDrawDistance`, shadows everything visible. Shorter ranges
//!   concentrate the same map on less ground, which gives sharper edges and draws
//!   fewer shadows.
//! - [`DrawDistance`]: a multiplier on the reach the game configured.
//!   - The streaming `spawn_ahead` the scene had at the end of
//!     [`Game::init`](crate::Game::init) and the renderer's distance fog scale
//!     together, so the fog keeps hiding the spawn edge.
//!   - Raising it fills the extra distance on the next streaming pass.
//!   - Lowering it keeps what is already spawned (it sits behind the thicker fog)
//!     and simply spawns nothing new until the player catches up. Nothing is
//!     despawned early, so seeded spawn sequences stay deterministic.
//! - [`RenderScale`]: the drawable's resolution as a fraction of the window's
//!   native pixels. The platform upscales it to the window. The default, 50%, is
//!   one drawable pixel per point on a Retina display. The
//!   [`surface_scale`](kaman_render_api::RenderDevice::surface_scale) the
//!   renderer reports lets a HUD keep a constant size at every scale.

use kaman_render_api::RenderSettings;
pub use kaman_render_api::ShadowQuality;

/// How far shadows reach, relative to the [`DrawDistance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShadowDistance {
    /// Shadows reach as far as the draw distance (the default).
    #[default]
    MatchDrawDistance,
    /// Shadows reach half the draw distance.
    Medium,
    /// Shadows reach a quarter of the draw distance.
    Near,
}

impl ShadowDistance {
    /// Every option, in menu order.
    pub const ALL: [Self; 3] = [Self::MatchDrawDistance, Self::Medium, Self::Near];

    /// The fraction of the draw distance shadows cover: `None` for all of it.
    #[must_use]
    pub fn fraction(self) -> Option<f32> {
        match self {
            Self::MatchDrawDistance => None,
            Self::Medium => Some(0.5),
            Self::Near => Some(0.25),
        }
    }

    /// Human-readable label, as the built-in menu shows it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::MatchDrawDistance => "Match Draw Distance",
            Self::Medium => "Medium",
            Self::Near => "Near",
        }
    }
}

/// How far the world is drawn, relative to the reach the game configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DrawDistance {
    /// Half the game's reach.
    Near,
    /// Three quarters of the game's reach.
    Medium,
    /// The game's full reach (the default).
    #[default]
    Far,
}

impl DrawDistance {
    /// Every option, in menu order.
    pub const ALL: [Self; 3] = [Self::Near, Self::Medium, Self::Far];

    /// Multiplier on the game's streaming reach and the renderer's fog.
    #[must_use]
    pub fn scale(self) -> f32 {
        match self {
            Self::Near => 0.5,
            Self::Medium => 0.75,
            Self::Far => 1.0,
        }
    }

    /// Human-readable label, as the built-in menu shows it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Near => "Near",
            Self::Medium => "Medium",
            Self::Far => "Far",
        }
    }
}

/// The render resolution, as a fraction of the window's native pixel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RenderScale {
    /// 50%: one drawable pixel per point on a Retina display (the default, and
    /// the resolution the engine rendered at before settings existed).
    #[default]
    Half,
    /// 75%.
    ThreeQuarters,
    /// 100%: every native pixel (4× the pixels of `Half` on Retina).
    Full,
}

impl RenderScale {
    /// Every option, in menu order.
    pub const ALL: [Self; 3] = [Self::Half, Self::ThreeQuarters, Self::Full];

    /// The fraction of the native pixel size rendered.
    #[must_use]
    pub fn factor(self) -> f32 {
        match self {
            Self::Half => 0.5,
            Self::ThreeQuarters => 0.75,
            Self::Full => 1.0,
        }
    }

    /// Human-readable label, as the built-in menu shows it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Half => "50%",
            Self::ThreeQuarters => "75%",
            Self::Full => "100%",
        }
    }

    /// The drawable size and UI scale for a window of `physical_width` ×
    /// `physical_height` native pixels at `scale_factor` pixels per point.
    ///
    /// Returns `(width, height, pixels_per_point)`, what
    /// [`RenderDevice::resize_surface`](kaman_render_api::RenderDevice::resize_surface)
    /// takes. Dimensions are rounded and at least 1.
    #[must_use]
    pub fn surface_for(
        self,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f64,
    ) -> (u32, u32, f32) {
        let f = f64::from(self.factor());
        let scale = |v: u32| ((f64::from(v) * f).round() as u32).max(1);
        (
            scale(physical_width),
            scale(physical_height),
            (scale_factor * f) as f32,
        )
    }
}

/// Label for a [`ShadowQuality`], as the built-in menu shows it.
#[must_use]
pub fn shadow_quality_label(quality: ShadowQuality) -> &'static str {
    match quality {
        ShadowQuality::Off => "Off",
        ShadowQuality::Low => "Low",
        ShadowQuality::High => "High",
    }
}

/// Every [`ShadowQuality`], in menu order.
pub const SHADOW_QUALITIES: [ShadowQuality; 3] =
    [ShadowQuality::Off, ShadowQuality::Low, ShadowQuality::High];

/// The engine's graphics settings (KE-0408). See the [module docs](self).
///
/// # Example
///
/// A game with its own options screen, cycling the draw distance on a key press:
///
/// ```rust
/// use kaman_core::graphics::{DrawDistance, GraphicsSettings};
/// use kaman_core::{EngineCtx, Game, Key};
///
/// struct MyGame;
/// impl Game for MyGame {
///     fn init(&mut self, _: &mut EngineCtx) {}
///     fn update(&mut self, ctx: &mut EngineCtx, _dt: f32) {
///         if ctx.input().is_key_just_pressed(Key::Q) {
///             let mut s = ctx.graphics_settings();
///             s.draw_distance = match s.draw_distance {
///                 DrawDistance::Far => DrawDistance::Near,
///                 DrawDistance::Near => DrawDistance::Medium,
///                 DrawDistance::Medium => DrawDistance::Far,
///             };
///             ctx.set_graphics_settings(s); // applied at the start of the next frame
///         }
///     }
///     fn render(&mut self, _: &mut EngineCtx) {}
/// }
/// # let _ = GraphicsSettings::default();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GraphicsSettings {
    /// Shadow quality. Default [`ShadowQuality::High`].
    pub shadows: ShadowQuality,
    /// Shadow range. Default [`ShadowDistance::MatchDrawDistance`].
    pub shadow_distance: ShadowDistance,
    /// Draw distance. Default [`DrawDistance::Far`].
    pub draw_distance: DrawDistance,
    /// Render resolution. Default [`RenderScale::Half`].
    pub render_scale: RenderScale,
}

impl GraphicsSettings {
    /// The renderer-facing half of these settings, as it crosses the render seam.
    #[must_use]
    pub fn render_settings(&self) -> RenderSettings {
        RenderSettings {
            shadows: self.shadows,
            shadow_distance: self.shadow_distance.fraction(),
            draw_distance_scale: self.draw_distance.scale(),
        }
    }

    /// Encode as four small integers for persistence: shadows, shadow distance,
    /// draw distance, render scale. Codes start at 1, so a missing stored value
    /// (read back as 0) is distinguishable and decodes to the default.
    #[must_use]
    pub fn to_codes(&self) -> [i64; 4] {
        let index = |i: usize| i as i64 + 1;
        [
            index(position(&SHADOW_QUALITIES, self.shadows)),
            index(position(&ShadowDistance::ALL, self.shadow_distance)),
            index(position(&DrawDistance::ALL, self.draw_distance)),
            index(position(&RenderScale::ALL, self.render_scale)),
        ]
    }

    /// Decode [`to_codes`](Self::to_codes) output. Any unknown or missing code
    /// (e.g. from an older or newer build) falls back to that setting's default,
    /// so stale stored preferences can never fail a launch.
    #[must_use]
    pub fn from_codes(codes: [i64; 4]) -> Self {
        fn pick<T: Copy + Default>(all: &[T; 3], code: i64) -> T {
            usize::try_from(code - 1)
                .ok()
                .and_then(|i| all.get(i).copied())
                .unwrap_or_default()
        }
        Self {
            shadows: pick(&SHADOW_QUALITIES, codes[0]),
            shadow_distance: pick(&ShadowDistance::ALL, codes[1]),
            draw_distance: pick(&DrawDistance::ALL, codes[2]),
            render_scale: pick(&RenderScale::ALL, codes[3]),
        }
    }
}

/// Index of `value` in `all` (every enum's `ALL` lists every variant).
fn position<T: PartialEq>(all: &[T], value: T) -> usize {
    all.iter().position(|v| *v == value).unwrap_or(0)
}

/// The engine's live graphics settings: the applied value, a queued change, and
/// what the draw distance scales from. Owned by the loop, reached by games
/// through [`EngineCtx`](crate::EngineCtx).
#[derive(Debug, Clone, Default)]
pub struct GraphicsState {
    current: GraphicsSettings,
    pending: Option<GraphicsSettings>,
    /// The scene's `spawn_ahead` at the end of `Game::init`: 100% draw distance.
    base_spawn_ahead: Option<f32>,
    /// Bumped each time a changed value is applied, so a platform layer can
    /// notice (resize the drawable, persist, refresh a menu) without a callback.
    revision: u64,
}

impl GraphicsState {
    /// The settings currently in effect.
    #[must_use]
    pub fn current(&self) -> GraphicsSettings {
        self.current
    }

    /// Queue `settings` to be applied at the start of the next frame. A later
    /// request before then replaces an earlier one.
    pub fn request(&mut self, settings: GraphicsSettings) {
        self.pending = Some(settings);
    }

    /// The queued, not-yet-applied settings, if any.
    #[must_use]
    pub fn pending(&self) -> Option<GraphicsSettings> {
        self.pending
    }

    /// How many changes have been applied since start-up.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The streaming reach the draw distance scales (the scene's `spawn_ahead`
    /// as the game left it at the end of `init`), once captured.
    #[must_use]
    pub fn base_spawn_ahead(&self) -> Option<f32> {
        self.base_spawn_ahead
    }

    pub(crate) fn take_pending(&mut self) -> Option<GraphicsSettings> {
        self.pending.take()
    }

    pub(crate) fn set_current(&mut self, settings: GraphicsSettings) {
        self.current = settings;
    }

    pub(crate) fn bump_revision(&mut self) {
        self.revision += 1;
    }

    pub(crate) fn set_base_spawn_ahead(&mut self, reach: f32) {
        self.base_spawn_ahead = Some(reach);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_pre_settings_engine() {
        let s = GraphicsSettings::default();
        assert_eq!(s.shadows, ShadowQuality::High);
        assert_eq!(s.shadow_distance, ShadowDistance::MatchDrawDistance);
        assert_eq!(s.draw_distance, DrawDistance::Far);
        assert_eq!(s.render_scale, RenderScale::Half);
        // ...which is exactly the renderer's own default.
        assert_eq!(s.render_settings(), RenderSettings::default());
    }

    #[test]
    fn presets_map_to_their_factors() {
        assert_eq!(DrawDistance::ALL.map(DrawDistance::scale), [0.5, 0.75, 1.0]);
        assert_eq!(
            ShadowDistance::ALL.map(ShadowDistance::fraction),
            [None, Some(0.5), Some(0.25)]
        );
        assert_eq!(RenderScale::ALL.map(RenderScale::factor), [0.5, 0.75, 1.0]);
    }

    #[test]
    fn render_settings_carry_every_renderer_field() {
        let s = GraphicsSettings {
            shadows: ShadowQuality::Low,
            shadow_distance: ShadowDistance::Near,
            draw_distance: DrawDistance::Medium,
            render_scale: RenderScale::Full,
        };
        assert_eq!(
            s.render_settings(),
            RenderSettings {
                shadows: ShadowQuality::Low,
                shadow_distance: Some(0.25),
                draw_distance_scale: 0.75,
            }
        );
    }

    #[test]
    fn every_combination_round_trips_through_codes() {
        for shadows in SHADOW_QUALITIES {
            for shadow_distance in ShadowDistance::ALL {
                for draw_distance in DrawDistance::ALL {
                    for render_scale in RenderScale::ALL {
                        let s = GraphicsSettings {
                            shadows,
                            shadow_distance,
                            draw_distance,
                            render_scale,
                        };
                        let codes = s.to_codes();
                        assert!(codes.iter().all(|&c| c >= 1), "codes start at 1");
                        assert_eq!(GraphicsSettings::from_codes(codes), s);
                    }
                }
            }
        }
    }

    #[test]
    fn missing_or_unknown_codes_decode_to_defaults() {
        assert_eq!(
            GraphicsSettings::from_codes([0; 4]),
            GraphicsSettings::default()
        );
        assert_eq!(
            GraphicsSettings::from_codes([-3, 99, 4, i64::MAX]),
            GraphicsSettings::default()
        );
        // A partially valid set keeps the valid parts.
        let s = GraphicsSettings::from_codes([1, 0, 1, 0]);
        assert_eq!(s.shadows, ShadowQuality::Off);
        assert_eq!(s.draw_distance, DrawDistance::Near);
        assert_eq!(s.shadow_distance, ShadowDistance::default());
        assert_eq!(s.render_scale, RenderScale::default());
    }

    #[test]
    fn half_scale_on_retina_is_one_pixel_per_point() {
        // A 800x600-point window on a 2x display: 1600x1200 native pixels.
        assert_eq!(
            RenderScale::Half.surface_for(1600, 1200, 2.0),
            (800, 600, 1.0)
        );
        assert_eq!(
            RenderScale::Full.surface_for(1600, 1200, 2.0),
            (1600, 1200, 2.0)
        );
        assert_eq!(
            RenderScale::ThreeQuarters.surface_for(1600, 1200, 2.0),
            (1200, 900, 1.5)
        );
        // Never a zero-sized drawable (a minimised window).
        assert_eq!(RenderScale::Half.surface_for(1, 0, 1.0), (1, 1, 0.5));
    }

    #[test]
    fn requests_queue_until_taken() {
        let mut g = GraphicsState::default();
        let near = GraphicsSettings {
            draw_distance: DrawDistance::Near,
            ..GraphicsSettings::default()
        };
        g.request(near);
        assert_eq!(g.current(), GraphicsSettings::default(), "not applied yet");
        assert_eq!(g.pending(), Some(near));
        assert_eq!(g.take_pending(), Some(near));
        assert_eq!(g.pending(), None);
    }
}

/// The apply path through the real loop, driven headlessly: what a game (or the
/// built-in menu) requesting new settings does to the renderer, the scene and
/// the game hook.
#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::headless::Headless;
    use crate::{EngineCtx, Game, Key};
    use kaman_ecs::TransformComponent;

    /// Streams around a fixed focus every update, requests `on_q` when Q is
    /// pressed, and records every settings hook call.
    #[derive(Default)]
    struct Probe {
        seen_in_init: Option<GraphicsSettings>,
        changed: Vec<GraphicsSettings>,
        on_q: Option<GraphicsSettings>,
    }

    impl Game for Probe {
        fn init(&mut self, ctx: &mut EngineCtx) {
            self.seen_in_init = Some(ctx.graphics_settings());
        }
        fn update(&mut self, ctx: &mut EngineCtx, _dt: f32) {
            if ctx.input().is_key_just_pressed(Key::Q) {
                if let Some(s) = self.on_q {
                    ctx.set_graphics_settings(s);
                }
            }
            ctx.scene_mut().stream(kaman_math::glam::Vec3::ZERO, |cx| {
                let e = cx
                    .world
                    .spawn((TransformComponent::from_position(cx.position),));
                cx.spawned(e);
            });
        }
        fn render(&mut self, _: &mut EngineCtx) {}
        fn graphics_settings_changed(&mut self, ctx: &mut EngineCtx, s: &GraphicsSettings) {
            assert_eq!(
                ctx.graphics_settings(),
                *s,
                "already in effect for the hook"
            );
            self.changed.push(*s);
        }
    }

    fn near() -> GraphicsSettings {
        GraphicsSettings {
            draw_distance: DrawDistance::Near,
            ..GraphicsSettings::default()
        }
    }

    #[test]
    fn defaults_are_pushed_at_init_without_a_change_event() {
        let mut game = Probe::default();
        let mut h = Headless::new();
        h.run(&mut game, 1);
        assert_eq!(game.seen_in_init, Some(GraphicsSettings::default()));
        assert!(game.changed.is_empty());
        assert_eq!(h.graphics().revision(), 0);
        assert_eq!(
            h.renderer().render_settings(),
            Some(RenderSettings::default())
        );
        assert_eq!(h.graphics().base_spawn_ahead(), Some(60.0));
        assert_eq!(h.scene().config().spawn_ahead, 60.0);
    }

    #[test]
    fn settings_queued_before_init_are_what_init_sees() {
        // What a windowed run does with the settings saved by the last launch.
        let mut game = Probe::default();
        let mut h = Headless::new();
        h.set_graphics_settings(near());
        h.run(&mut game, 1);
        assert_eq!(game.seen_in_init, Some(near()));
        assert!(game.changed.is_empty(), "start-up is not a change");
        assert_eq!(h.graphics().revision(), 0);
        assert_eq!(
            h.renderer()
                .render_settings()
                .map(|r| r.draw_distance_scale),
            Some(0.5)
        );
        // The base is the game's own reach; the scene runs at half of it.
        assert_eq!(h.graphics().base_spawn_ahead(), Some(60.0));
        assert_eq!(h.scene().config().spawn_ahead, 30.0);
    }

    #[test]
    fn a_game_request_applies_at_the_start_of_the_next_frame() {
        let mut game = Probe {
            on_q: Some(GraphicsSettings {
                shadows: ShadowQuality::Off,
                shadow_distance: ShadowDistance::Near,
                draw_distance: DrawDistance::Medium,
                render_scale: RenderScale::Full,
            }),
            ..Probe::default()
        };
        let mut h = Headless::new();
        h.run(&mut game, 1);

        h.input_mut().press_key(Key::Q);
        h.run(&mut game, 1);
        h.input_mut().release_key(Key::Q);
        // Requested during that frame's update: queued, not yet in effect.
        assert!(game.changed.is_empty());
        assert_eq!(h.graphics().current(), GraphicsSettings::default());
        assert!(h.graphics().pending().is_some());

        h.run(&mut game, 1);
        let wanted = game.on_q.unwrap();
        assert_eq!(game.changed, vec![wanted], "one hook call");
        assert_eq!(h.graphics().current(), wanted);
        assert_eq!(h.graphics().revision(), 1);
        assert_eq!(
            h.renderer().render_settings(),
            Some(wanted.render_settings())
        );
        assert_eq!(h.scene().config().spawn_ahead, 45.0);

        // Re-requesting the same value is a no-op.
        h.set_graphics_settings(wanted);
        h.run(&mut game, 1);
        assert_eq!(game.changed.len(), 1);
        assert_eq!(h.graphics().revision(), 1);
    }

    #[test]
    fn draw_distance_rescales_streaming_without_despawning_ahead() {
        let mut game = Probe::default();
        let mut h = Headless::new();
        h.set_graphics_settings(near());
        h.run(&mut game, 1);
        // 30 units at 6-unit spacing: slots 0..=5.
        assert_eq!(h.scene().streamed_count(), 6);

        // Far: the next pass fills out to the full 60 (slots 0..=10).
        h.set_graphics_settings(GraphicsSettings::default());
        h.run(&mut game, 1);
        assert_eq!(h.scene().config().spawn_ahead, 60.0);
        assert_eq!(h.scene().streamed_count(), 11);

        // Back to Near: what is already out there stays (behind the thicker
        // fog), and nothing is spawned twice when it grows again.
        h.set_graphics_settings(near());
        h.run(&mut game, 1);
        assert_eq!(h.scene().streamed_count(), 11);
        h.set_graphics_settings(GraphicsSettings::default());
        h.run(&mut game, 1);
        assert_eq!(h.scene().streamed_count(), 11);
    }
}
