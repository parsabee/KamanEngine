// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The engine-generic **sun + sky** description (KE-0406).
//!
//! [`SunSky`] is how anything above the seam asks for a *specific sun*: where it
//! is in the sky, what colour and how bright it is, how much light the sky fills
//! the shadows with, and what the sky gradient looks like behind it. A game
//! pushes one through [`FrameRecorder::set_sun_sky`](crate::FrameRecorder::set_sun_sky)
//! and the backend honours it for that frame — the sun direction drives the
//! shading *and* the sun disc the sky pass draws, so they can never disagree.
//!
//! Everything here is **physical parameters only**. There is deliberately no
//! notion of *time of day*: "summer, 4pm" is a game's policy, expressed as an
//! elevation/azimuth pair in the game's own config. The engine knows angles.
//!
//! # Elevation and azimuth convention
//!
//! The convention is stated once, here, and pinned by unit tests — an unstated
//! one is a guaranteed future bug, since the only way to notice you got it wrong
//! is that the light comes from the wrong side.
//!
//! World space is right-handed with **`+Y` up**. The horizontal plane is `XZ`,
//! and the sun's horizontal bearing is read as a **compass bearing with `-Z` as
//! north and `+X` as east**:
//!
//! - **Elevation** is measured **up from the horizon**, in degrees.
//!   `0°` is a sun exactly on the horizon (the light travels horizontally);
//!   `90°` is a sun straight overhead (the light travels straight down, `-Y`).
//!   Negative elevations put the sun below the horizon.
//! - **Azimuth** is measured in degrees **from `-Z`, turning toward `+X`** — i.e.
//!   *clockwise* when looking down at the `XZ` plane from `+Y` with `-Z` drawn
//!   "up" on the page. So `0°` = north (`-Z`), `90°` = east (`+X`),
//!   `180°` = south (`+Z`), `270°` = west (`-X`). Azimuth wraps, so `370°` and
//!   `10°` describe the same sun.
//!
//! The vector a caller actually wants is always **derived** — [`SunSky::toward_sun`]
//! (from the scene toward the sun) and [`SunSky::direction`] (the direction the
//! light *travels*, which is what a shader shades with). Nothing above or below
//! the seam hand-builds a light vector.

use kaman_math::glam::Vec3;

/// A sun and the sky it hangs in: the complete lighting *description* the seam
/// accepts.
///
/// Push one with
/// [`FrameRecorder::set_sun_sky`](crate::FrameRecorder::set_sun_sky). Colours are
/// **linear** RGB (the backend owns the tonemap + sRGB encode), and the sun's
/// position is an [elevation/azimuth pair](crate::sun#elevation-and-azimuth-convention) in
/// degrees rather than a vector, so a caller can express "a low sun from the
/// west" without knowing which axis the world streams along.
///
/// # Why angles and not a direction vector
///
/// A vector invites two bugs this type makes impossible: a caller normalising
/// (or forgetting to normalise) its own light direction, and the sky's sun disc
/// being derived from a *different* vector than the shading. Both come out of
/// [`direction`](Self::direction) instead, which is the single source of truth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunSky {
    /// Sun height above the horizon in **degrees**: `0` on the horizon, `90`
    /// straight overhead. See the [module convention](crate::sun#elevation-and-azimuth-convention).
    pub sun_elevation_deg: f32,
    /// Sun bearing in **degrees** from `-Z` (north) turning toward `+X` (east):
    /// `0` north, `90` east, `180` south, `270` west. Wraps freely. See the
    /// [module convention](crate::sun#elevation-and-azimuth-convention).
    pub sun_azimuth_deg: f32,
    /// Linear RGB colour of the sunlight. A white sun is `[1.0, 1.0, 1.0]`; a
    /// warm afternoon sun pulls the blue channel down a little.
    pub sun_color: [f32; 3],
    /// Brightness multiplier on the sun's direct (diffuse) contribution. This is
    /// the term that should dominate — see [`sky_fill`](Self::sky_fill).
    pub sun_intensity: f32,
    /// Ambient **sky fill**: the flat term that lights surfaces the sun cannot
    /// reach, standing in for light scattered out of the sky dome.
    ///
    /// It is *fill*, not a second sun: keep it well below
    /// [`sun_intensity`](Self::sun_intensity) (roughly `0.15..=0.25` against a
    /// sun near `1.0`) or unlit faces creep up toward lit ones and the scene
    /// reads flatly overcast instead of sunlit.
    pub sky_fill: f32,
    /// Linear RGB colour of the sky at the **zenith** (straight up).
    pub sky_zenith_color: [f32; 3],
    /// Linear RGB colour of the sky at the **horizon**. Distance fog blends
    /// toward this, so far geometry meets the sky seamlessly — changing it
    /// changes the haze the horizon dissolves into.
    pub sky_horizon_color: [f32; 3],
}

impl SunSky {
    /// Unit vector pointing **from the scene toward the sun** — the direction you
    /// would look to see the sun, and therefore where the sky's sun disc is drawn.
    ///
    /// Derived from [`sun_elevation_deg`](Self::sun_elevation_deg) and
    /// [`sun_azimuth_deg`](Self::sun_azimuth_deg) per the
    /// [module convention](crate::sun#elevation-and-azimuth-convention).
    #[must_use]
    pub fn toward_sun(&self) -> Vec3 {
        let (sin_el, cos_el) = self.sun_elevation_deg.to_radians().sin_cos();
        let (sin_az, cos_az) = self.sun_azimuth_deg.to_radians().sin_cos();
        // Azimuth 0 is -Z (north) and turns toward +X (east), so the horizontal
        // part is (sin az, -cos az) scaled by the horizon-plane length cos(el).
        Vec3::new(sin_az * cos_el, sin_el, -cos_az * cos_el)
    }

    /// Unit vector in the direction the sunlight **travels** — the negation of
    /// [`toward_sun`](Self::toward_sun), and what a shader shades with.
    ///
    /// An overhead sun (elevation `90°`) gives `(0, -1, 0)`: light going straight
    /// down.
    #[must_use]
    pub fn direction(&self) -> Vec3 {
        -self.toward_sun()
    }
}

impl Default for SunSky {
    fn default() -> Self {
        // A high, white, mid-afternoon sun over a calm blue gradient sky. The
        // angles are the pre-KE-0406 hardcoded backend default expressed in this
        // convention (its direction was `(-0.5, -1.0, -0.3)`, i.e. bearing ~120°
        // at ~60° elevation), so a caller that pushes nothing keeps the engine's
        // historical default look — except for the fill, which is rebalanced as
        // sky fill rather than a second light (KE-0406).
        Self {
            sun_elevation_deg: 60.0,
            sun_azimuth_deg: 120.0,
            sun_color: [1.0, 1.0, 1.0],
            sun_intensity: 0.8,
            sky_fill: 0.2,
            sky_zenith_color: [0.09, 0.22, 0.44],
            sky_horizon_color: [0.55, 0.62, 0.72],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert `a` and `b` agree componentwise to within a float epsilon.
    fn close(a: Vec3, b: Vec3) {
        assert!(
            (a - b).length() < 1e-5,
            "expected {b:?}, got {a:?} (delta {})",
            (a - b).length()
        );
    }

    #[test]
    fn elevation_90_points_straight_down() {
        // A sun directly overhead lights straight down, whatever the bearing —
        // azimuth is degenerate at the zenith, so every azimuth must agree.
        for azimuth in [0.0, 37.0, 120.0, 270.0, 359.0] {
            let sun = SunSky {
                sun_elevation_deg: 90.0,
                sun_azimuth_deg: azimuth,
                ..SunSky::default()
            };
            close(sun.toward_sun(), Vec3::Y);
            close(sun.direction(), Vec3::NEG_Y);
        }
    }

    #[test]
    fn elevation_0_is_horizontal() {
        // On the horizon the light travels parallel to the ground: no Y at all.
        for azimuth in [0.0, 45.0, 123.0, 200.0, 310.0] {
            let sun = SunSky {
                sun_elevation_deg: 0.0,
                sun_azimuth_deg: azimuth,
                ..SunSky::default()
            };
            assert!(
                sun.toward_sun().y.abs() < 1e-6,
                "azimuth {azimuth}: expected a horizontal sun, got {:?}",
                sun.toward_sun()
            );
            assert!(sun.direction().y.abs() < 1e-6);
        }
    }

    #[test]
    fn azimuth_turns_north_to_east_to_south_to_west() {
        // The convention, pinned: 0 = -Z (north), and increasing azimuth turns
        // toward +X (east) — clockwise seen from above. If this test ever needs
        // "fixing", the sun has started coming from the wrong side.
        let at = |azimuth: f32| {
            SunSky {
                sun_elevation_deg: 0.0,
                sun_azimuth_deg: azimuth,
                ..SunSky::default()
            }
            .toward_sun()
        };
        close(at(0.0), Vec3::NEG_Z);
        close(at(90.0), Vec3::X);
        close(at(180.0), Vec3::Z);
        close(at(270.0), Vec3::NEG_X);
    }

    #[test]
    fn azimuth_wraps_and_direction_is_the_negated_bearing() {
        let a = SunSky {
            sun_elevation_deg: 32.0,
            sun_azimuth_deg: 284.0,
            ..SunSky::default()
        };
        let b = SunSky {
            sun_azimuth_deg: 284.0 + 360.0,
            ..a
        };
        close(a.toward_sun(), b.toward_sun());
        close(a.direction(), b.direction());
    }

    #[test]
    fn derived_vectors_are_unit_length() {
        for elevation in [-20.0, 0.0, 15.0, 32.0, 60.0, 90.0] {
            for azimuth in [0.0, 73.0, 180.0, 284.0] {
                let sun = SunSky {
                    sun_elevation_deg: elevation,
                    sun_azimuth_deg: azimuth,
                    ..SunSky::default()
                };
                assert!((sun.toward_sun().length() - 1.0).abs() < 1e-6);
                assert!((sun.direction().length() - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn a_western_afternoon_sun_comes_from_negative_x() {
        // The demo's case (KE-0406): a ~30° sun in the west-northwest. The light
        // must travel *toward* +X (eastward), i.e. arrive from the left of a
        // camera looking along -Z.
        let sun = SunSky {
            sun_elevation_deg: 32.0,
            sun_azimuth_deg: 284.0,
            ..SunSky::default()
        };
        assert!(sun.toward_sun().x < -0.7, "sun sits in the west");
        assert!(sun.toward_sun().y > 0.0, "sun is above the horizon");
        assert!(sun.direction().x > 0.0, "light travels eastward");
        assert!(sun.direction().y < 0.0, "light travels downward");
    }

    #[test]
    fn default_reproduces_the_historical_backend_sun() {
        // The pre-KE-0406 hardcoded default was `direction: (-0.5, -1.0, -0.3)`.
        // The default angles must reproduce it (normalised) so pushing nothing
        // keeps the engine's historical look.
        let historical = Vec3::new(-0.5, -1.0, -0.3).normalize();
        let d = SunSky::default().direction();
        assert!(
            (d - historical).length() < 0.02,
            "default sun {d:?} drifted from the historical {historical:?}"
        );
    }

    #[test]
    fn default_fill_is_dominated_by_the_sun() {
        // The KE-0406 rebalance: fill is sky fill, not a second sun.
        let sun = SunSky::default();
        assert!((0.15..=0.25).contains(&sun.sky_fill));
        assert!(sun.sky_fill < sun.sun_intensity * 0.5);
    }
}
