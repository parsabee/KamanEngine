// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Render **quality settings** that cross the seam (KE-0408): shadow quality, the
//! shadow range, and the draw-distance scale.
//!
//! These are the renderer-facing half of the engine's graphics settings. The
//! user-facing presets ("Near / Medium / Far", "Match Draw Distance", …) live in
//! `kaman-core`, which turns them into a [`RenderSettings`] and pushes it with
//! [`RenderDevice::set_render_settings`](crate::RenderDevice::set_render_settings).
//! Like [`SunSky`](crate::SunSky), the value is plain data with **no GPU type**, and
//! the backend decides what each tier costs. For example, the texel count of a
//! [`ShadowQuality`] tier is the backend's choice, so an iOS backend can map the
//! same tier to a smaller map.
//!
//! Every quantity here is **relative** to the backend's own look defaults (its
//! fog, its shadow map). That keeps the seam free of absolute look parameters that
//! nothing above it owns yet, and it means "draw distance" and "shadow range" stay
//! consistent however the backend tunes its fog.
//!
//! Render **resolution** is not part of this value. It is a property of the
//! drawable, set with [`RenderDevice::resize_surface`](crate::RenderDevice::resize_surface).

/// How the sun's shadows are rendered.
///
/// # Example
///
/// ```rust
/// use kaman_render_api::ShadowQuality;
///
/// assert_eq!(ShadowQuality::default(), ShadowQuality::High);
/// assert!(!ShadowQuality::Off.casts_shadows());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShadowQuality {
    /// No shadows: the shadow pass is skipped entirely and nothing is shadowed.
    Off,
    /// A lower-resolution shadow map (the Metal backend uses 2048²): softer,
    /// blockier edges at a quarter of the memory and fill cost of `High`.
    Low,
    /// The full-resolution shadow map (the Metal backend uses 4096²). The default.
    #[default]
    High,
}

impl ShadowQuality {
    /// Whether this tier renders shadows at all (everything but [`Off`](Self::Off)).
    #[must_use]
    pub fn casts_shadows(self) -> bool {
        self != Self::Off
    }
}

/// The renderer-facing quality settings, pushed with
/// [`RenderDevice::set_render_settings`](crate::RenderDevice::set_render_settings).
///
/// Sticky: a backend keeps the last value until the next push, and uses
/// `RenderSettings::default()` until the first one. The default reproduces the
/// backend's look exactly as it was before settings existed.
///
/// # Example
///
/// ```rust
/// use kaman_render_api::{RenderSettings, ShadowQuality};
///
/// // Half the draw distance, with shadows only over the nearest quarter of it.
/// let settings = RenderSettings {
///     shadows: ShadowQuality::Low,
///     shadow_distance: Some(0.25),
///     draw_distance_scale: 0.5,
/// };
/// assert_ne!(settings, RenderSettings::default());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSettings {
    /// Shadow quality tier. Default [`ShadowQuality::High`].
    pub shadows: ShadowQuality,
    /// How far shadows reach, as a **fraction of the draw distance** (the depth
    /// at which the distance fog becomes opaque).
    ///
    /// - `None` (the default) shadows everything the fog leaves visible.
    /// - `Some(f)` shadows only the nearest `f` of it (e.g. `Some(0.25)` is a
    ///   quarter). The same shadow map then covers less ground, so edges are
    ///   sharper. A fraction outside `0..=1` is clamped by the backend.
    pub shadow_distance: Option<f32>,
    /// Multiplier on the backend's default **draw distance**: the fog's start and
    /// its opaque depth both scale by it, so the fog keeps hiding the edge of a
    /// streamed world whose reach is scaled by the same factor. Must be positive;
    /// default `1.0`.
    pub draw_distance_scale: f32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            shadows: ShadowQuality::High,
            shadow_distance: None,
            draw_distance_scale: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_pre_settings_look() {
        let s = RenderSettings::default();
        assert_eq!(s.shadows, ShadowQuality::High);
        assert_eq!(s.shadow_distance, None);
        assert_eq!(s.draw_distance_scale, 1.0);
    }

    #[test]
    fn null_renderer_records_settings_and_surface() {
        use crate::{NullRenderer, RenderDevice};
        let mut r = NullRenderer::new();
        assert_eq!(r.render_settings(), None);
        assert_eq!(
            r.surface_size(),
            (
                crate::null::NULL_SURFACE_WIDTH,
                crate::null::NULL_SURFACE_HEIGHT
            )
        );
        assert_eq!(r.surface_scale(), 1.0);

        let s = RenderSettings {
            shadows: ShadowQuality::Off,
            shadow_distance: Some(0.5),
            draw_distance_scale: 0.75,
        };
        r.set_render_settings(&s);
        assert_eq!(r.render_settings(), Some(s));

        r.resize_surface(640, 0, 1.5);
        assert_eq!(r.surface_size(), (640, 1), "a zero dimension becomes 1");
        assert_eq!(r.surface_scale(), 1.5);
    }

    #[test]
    fn only_off_disables_shadows() {
        assert!(!ShadowQuality::Off.casts_shadows());
        assert!(ShadowQuality::Low.casts_shadows());
        assert!(ShadowQuality::High.casts_shadows());
    }
}
