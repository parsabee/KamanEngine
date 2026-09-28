// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The sun's shadow map: frustum fitting, stability, and bias (KE-0407).
//!
//! This module is **pure math** — no Metal types — so everything that decides
//! *where* the shadow map looks and *how* its lookups are biased is unit-tested
//! without a GPU. The backend ([`crate::backend`]) owns the Metal side: the depth
//! texture, the depth-only caster pipeline, and the pass itself.
//!
//! # Pass order
//!
//! Every frame the backend records, in this order, on one command buffer:
//!
//! 1. **Shadow pass** — depth-only, from the sun, into a
//!    [`SHADOW_MAP_SIZE`]² `Depth32Float` texture. Every mesh the frame draws is
//!    rendered into it as a caster through a dedicated position-only vertex
//!    function (no fragment stage at all).
//! 2. **Scene pass** — the gradient sky, then every draw through the lit
//!    (untextured / textured) pipelines, which sample the map, then the 2D overlay.
//!
//! To make (1) possible the backend **defers encoding**: `draw_mesh` records a
//! draw (and writes its uniforms into the ring) but encodes nothing; `submit`
//! encodes the shadow pass over the recorded draws, then the scene pass over the
//! same list. A caster therefore never needs to be submitted twice by the game.
//!
//! # Frustum fit and why it does not shimmer
//!
//! A single map, not cascades, because the shadow-relevant part of the world is a
//! small **slab**: shadows are only drawn out to [`shadow_distance_for_fog`]
//! view-depth units — where ground-level distance fog becomes essentially opaque,
//! but never further than [`MAX_SHADOW_DISTANCE`]. The cap keeps the map's texels
//! small when the fog is pushed far out (a 3× deeper slab would make every texel,
//! and so every shadow edge, 3× blurrier); past it shadows fade out over the last
//! [`SHADOW_FADE_FRACTION`] of the slab. [`fit_shadow_frustum`] each frame:
//!
//! 1. Cuts the camera frustum at that view depth and takes the eight corners of the
//!    resulting slab ([`visible_slab_corners`]).
//! 2. Encloses them in a **bounding sphere** centred on the slab's axis. A sphere
//!    (rather than a tight light-space box) has the same size whichever way the
//!    camera turns, so the map's world-space texel size does not breathe as the
//!    view rotates. The radius is additionally **rounded up** to a
//!    [`RADIUS_QUANTUM`] so float noise in the corners can never change it frame
//!    to frame.
//! 3. Rotates the sphere's centre into a light space whose origin is the **world
//!    origin** (a rotation only, so it does not move with the camera) and **snaps
//!    the centre to whole texels** there. With the texel size fixed (2) and the
//!    projection's left/bottom edge always a whole number of texels from the fixed
//!    origin, every world point lands on the *same fractional texel position*
//!    every frame: the map slides in exact texel steps and a static edge
//!    rasterises identically, so it cannot crawl or shimmer as the camera moves.
//!    `snapping_keeps_every_world_point_on_the_same_subtexel_position` pins this.
//!
//! The ortho projection is extended [`SHADOW_CASTER_MARGIN`] toward the sun so a
//! caster outside the sphere (a tall building beside the road) still lands in
//! the map; the pass also clamps depth instead of clipping, so anything even
//! nearer the sun than that is flattened onto the near plane and still casts.
//!
//! # Bias
//!
//! Acne and peter-panning are controlled by two terms, both resolved per fragment
//! in `shadow_visibility` in `rasterization.metal`:
//!
//! - **Receiver-plane depth.** Every map texel the PCF kernel reads is compared
//!   against the depth the *receiver's own triangle* has **at that texel's
//!   centre**, extrapolated along the triangle's plane (reconstructed from the
//!   screen-space derivatives of the fragment's shadow-map position, which is exact
//!   for a planar triangle). A lit surface therefore never sees *itself* as an
//!   occluder however steeply it faces away from the sun — which is what removes
//!   acne without a large constant or slope-scaled bias, and so without the
//!   peter-panning those cause. The per-texel gradient is capped at
//!   [`SHADOW_RECEIVER_SLOPE_CAP_WORLD`] = 0.5 world units, which only binds for
//!   receivers within ~5° of edge-on to the sun (where the gradient is unbounded);
//!   such faces get under ~10% of the sun's direct light, so the cap is invisible.
//! - **[`SHADOW_DEPTH_BIAS_WORLD`] = 0.02 world units (2 cm) constant.** What the
//!   receiver-plane reconstruction cannot remove: float error in the derivatives
//!   and in the caster pass's own depth interpolation. Two centimetres is under
//!   half a map texel at the demo's fit (~4.7 cm), comfortably above that error,
//!   and small enough that the resulting peter-panning — the shadow's contact edge
//!   moving `bias / tan(elevation)` along the ground, ~3 cm under the demo's 32°
//!   sun — is below one texel and so invisible. Tyres and building bases stay
//!   attached to their shadows.
//!
//! The offscreen reference scenes confirm the pair is enough: rendering them with
//! the shadow pass's casters disabled produces **byte-identical** frames, i.e. no
//! surface shadows itself anywhere (see the re-bless notes in `tests/`).
//!
//! # Filtering
//!
//! Tent-weighted PCF over the 4×4 texels around the lookup — the weights of a 3×3
//! grid of bilinear taps one texel apart, i.e. a smooth ~4-texel (~19 cm at the
//! demo's fit) penumbra — so edges are soft at the demo's resolution rather than
//! stair-stepped. The compares are done per texel (not with a hardware compare
//! sampler) precisely so each texel can use its own receiver-plane depth above.
//! Shadows fade out over the last [`SHADOW_FADE_FRACTION`] of the fitted
//! distance, so the map's edge is never seen as a line. When the slab ends at the
//! fog that fade is hidden in it; when [`MAX_SHADOW_DISTANCE`] binds (the default
//! fog), it is a gentle ramp over the last 11 units in clear air.

use kaman_math::glam::{Mat4, Vec3, Vec4};

/// Edge length, in texels, of the square shadow map.
///
/// 2048² `Depth32Float` is 16 MiB — affordable on every target — and at the
/// playable demo's fit (a 48-unit sphere) a texel is ~4.7 cm of world, so a car
/// (2.85 units long) spans ~60 texels.
pub const SHADOW_MAP_SIZE: u32 = 2048;

/// How far (world units) the light-space projection extends **toward the sun**
/// beyond the fitted sphere, so casters outside the visible slab — a building
/// standing just off-screen between the sun and the road — still cast into it.
///
/// Generous on purpose: it costs only depth precision, and `Depth32Float` over a
/// ~175-unit range still resolves far below a millimetre.
pub const SHADOW_CASTER_MARGIN: f32 = 80.0;

/// Shadow distance (view depth, world units) used when distance fog is disabled
/// and so cannot say where the visible slab ends.
pub const DEFAULT_SHADOW_DISTANCE: f32 = 60.0;

/// The furthest (view depth, world units) the shadow slab ever reaches, however
/// far out the fog is.
///
/// The map's texel size is proportional to the slab it covers, so a slab that
/// followed the fog all the way out would trade sharp shadows near the car —
/// where they are actually looked at — for coverage of distant ground where a
/// ~25 cm-per-texel shadow would read as a smudge anyway. 55 units is the slab
/// the demo's fog produced when shadows were tuned (`start = 35`, `k = 0.1`),
/// i.e. it pins the ~4.7 cm texel and the bias/penumbra numbers below; it covers
/// the road from the camera to ~40 units ahead of the car. Beyond it shadows fade
/// out over the last [`SHADOW_FADE_FRACTION`].
pub const MAX_SHADOW_DISTANCE: f32 = 55.0;

/// The constant depth bias, in **world units**: 2 cm. See the
/// [module docs](self#bias) for why this value.
pub const SHADOW_DEPTH_BIAS_WORLD: f32 = 0.02;

/// The cap, in **world units** per map texel, on the receiver-plane depth
/// gradient the per-texel compares extrapolate along. See the
/// [module docs](self#bias).
pub const SHADOW_RECEIVER_SLOPE_CAP_WORLD: f32 = 0.5;

/// Fraction of the shadow distance over which shadows fade out (the last 20%),
/// so the map's far edge is never visible as a line — see
/// [`MAX_SHADOW_DISTANCE`].
pub const SHADOW_FADE_FRACTION: f32 = 0.2;

/// Granularity (world units) the fitted sphere's radius is rounded **up** to.
///
/// Rounding makes the radius — and therefore the texel size — bit-identical
/// across frames even though the corners it is computed from carry float noise;
/// being larger than a texel, it also pays for the half-texel the snapped centre
/// may move away from the unsnapped one.
pub const RADIUS_QUANTUM: f32 = 0.5;

/// Where the shadow slab ends for a given distance fog, in view-depth units.
///
/// The fog (`rasterization.metal`, `apply_fog`) is `1 - exp(-(k·(d - start))²)`,
/// which reaches **98%** at `d = start + 2/k`: past that, geometry at ground
/// level is indistinguishable from the horizon, so a shadow there cannot be seen.
/// That distance is then capped at [`MAX_SHADOW_DISTANCE`] so a far fog does not
/// blur the map. With the default fog (`start = 105`, `k = 1/30`) the fog reaches
/// 98% at 165 units, so the cap binds and the slab is **55** units. A fog density
/// of zero (fog off) falls back to [`DEFAULT_SHADOW_DISTANCE`].
#[must_use]
pub fn shadow_distance_for_fog(fog_start: f32, fog_density: f32) -> f32 {
    if fog_density > 0.0 {
        (fog_start.max(0.0) + 2.0 / fog_density).min(MAX_SHADOW_DISTANCE)
    } else {
        DEFAULT_SHADOW_DISTANCE
    }
}

/// The eight world-space corners of the camera frustum cut at view depth
/// `shadow_distance`: the four near-plane corners followed by the four corners
/// at the cut (or at the far plane, if that is nearer).
///
/// Derived from the inverse of the seam-provided view-projection alone (depth
/// range `0..1`, as every `kaman_camera` projection uses), so it works for any
/// camera. View depth is measured as clip-space `w`, the same quantity the lit
/// shaders fog and fade with.
#[must_use]
pub fn visible_slab_corners(view_projection: Mat4, shadow_distance: f32) -> [Vec3; 8] {
    let inverse = view_projection.inverse();
    let unproject = |x: f32, y: f32, z: f32| inverse.project_point3(Vec3::new(x, y, z));
    let view_depth = |p: Vec3| (view_projection * p.extend(1.0)).w;

    let mut corners = [Vec3::ZERO; 8];
    for (i, (x, y)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let near = unproject(x, y, 0.0);
        let far = unproject(x, y, 1.0);
        // View depth is affine along the corner ray, so the cut is one lerp. A
        // projection with no depth variation (e.g. the identity, before any camera
        // is pushed) keeps the whole ray rather than dividing by zero.
        let (dn, df) = (view_depth(near), view_depth(far));
        let t = if (df - dn).abs() > f32::EPSILON {
            ((shadow_distance - dn) / (df - dn)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        corners[i] = near;
        corners[i + 4] = near.lerp(far, t);
    }
    corners
}

/// A rotation-only view matrix looking along the light's travel `direction`.
///
/// Its origin is the **world origin**, never the camera: that is what gives the
/// texel snapping in [`fit_shadow_frustum`] a fixed grid to snap to.
#[must_use]
pub fn light_view(direction: Vec3) -> Mat4 {
    let forward = direction.try_normalize().unwrap_or(Vec3::NEG_Y);
    // Any up vector not parallel to the light works; switch away from +Y only
    // when the sun is (almost) overhead.
    let up = if forward.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Mat4::look_to_rh(Vec3::ZERO, forward, up)
}

/// One frame's fitted shadow projection — see the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowFit {
    /// World → light clip space: an orthographic projection (depth `0..1`, `0`
    /// nearest the sun) times [`light_view`]. Casters are rendered with it and
    /// receivers look themselves up in the map with it.
    pub light_view_projection: Mat4,
    /// The fitted sphere's centre in world space, **after** texel snapping.
    pub center: Vec3,
    /// The fitted sphere's radius in world units (a multiple of [`RADIUS_QUANTUM`]).
    pub radius: f32,
    /// World-space size of one shadow-map texel (`2 * radius / map size`).
    pub texel_world_size: f32,
    /// World-space length of the projection's depth range, near to far. A world
    /// distance along the light divides by this to become a map depth.
    pub depth_range: f32,
    /// The view depth the fit covers (see [`shadow_distance_for_fog`]).
    pub shadow_distance: f32,
}

/// Fit the sun's orthographic shadow projection to the camera's visible slab.
///
/// `view_projection` is the frame's camera (as pushed through the seam),
/// `light_direction` the direction the sunlight travels (the KE-0406
/// [`SunSky::direction`](kaman_render_api::SunSky::direction) — the backend passes
/// the very vector it shades with), `shadow_distance` how deep the slab is, and
/// `map_size` the shadow map's edge in texels. See the [module docs](self) for the
/// strategy and its stability guarantee.
#[must_use]
pub fn fit_shadow_frustum(
    view_projection: Mat4,
    light_direction: Vec3,
    shadow_distance: f32,
    map_size: u32,
) -> ShadowFit {
    let corners = visible_slab_corners(view_projection, shadow_distance);

    // Sphere centred on the slab's axis (near-face centre -> far-face centre), at
    // the point equidistant from the near and far corners; clamped to the axis so
    // a short, wide slab does not put it outside. The radius is then the true
    // maximum over all eight corners, so every corner is enclosed regardless.
    let near_center = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
    let far_center = (corners[4] + corners[5] + corners[6] + corners[7]) * 0.25;
    let near_extent = corners[..4]
        .iter()
        .map(|c| c.distance(near_center))
        .fold(0.0, f32::max);
    let far_extent = corners[4..]
        .iter()
        .map(|c| c.distance(far_center))
        .fold(0.0, f32::max);
    let axis = far_center - near_center;
    let length = axis.length();
    let center = if length > f32::EPSILON {
        let t = (length * length + far_extent * far_extent - near_extent * near_extent)
            / (2.0 * length);
        near_center + axis / length * t.clamp(0.0, length)
    } else {
        near_center
    };
    let raw_radius = corners
        .iter()
        .map(|c| c.distance(center))
        .fold(0.0, f32::max);
    // Round up (plus one quantum of headroom for the snap below), so the radius
    // is identical frame to frame and the snapped sphere still encloses the slab.
    let radius = ((raw_radius / RADIUS_QUANTUM).ceil() + 1.0) * RADIUS_QUANTUM;

    let map_size = map_size.max(1) as f32;
    let texel = 2.0 * radius / map_size;

    // Snap the centre to whole texels in the fixed-origin light space. `radius`
    // is `map_size / 2` texels, so the left/bottom edges land on whole texels too.
    let view = light_view(light_direction);
    let center_ls = view.transform_point3(center);
    let snapped_ls = Vec3::new(
        (center_ls.x / texel).round() * texel,
        (center_ls.y / texel).round() * texel,
        (center_ls.z / texel).round() * texel,
    );

    // Light view space looks down -Z, so the distance in front of the light is
    // -z. Near is pulled `SHADOW_CASTER_MARGIN` further toward the sun.
    let depth_center = -snapped_ls.z;
    let near = depth_center - radius - SHADOW_CASTER_MARGIN;
    let far = depth_center + radius;
    let projection = Mat4::orthographic_rh(
        snapped_ls.x - radius,
        snapped_ls.x + radius,
        snapped_ls.y - radius,
        snapped_ls.y + radius,
        near,
        far,
    );

    ShadowFit {
        light_view_projection: projection * view,
        center: view.inverse().transform_point3(snapped_ls),
        radius,
        texel_world_size: texel,
        depth_range: far - near,
        shadow_distance,
    }
}

impl ShadowFit {
    /// Project a world point into shadow-map space: `(u, v)` in texture
    /// coordinates (`0..1`, `v` down, as the shader samples) and the map depth
    /// (`0..1`, `0` nearest the sun). Test/diagnostic aid mirroring the shader.
    #[must_use]
    pub fn project(&self, world: Vec3) -> Vec3 {
        let clip: Vec4 = self.light_view_projection * world.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        Vec3::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::LightUniforms;
    use kaman_render_api::SunSky;

    /// The playable demo's chase camera clip range.
    const NEAR: f32 = 0.3;

    /// The playable demo's chase camera: 12 behind and 6 above the car, looking at
    /// a point 1.5 above it (~20.6° down), 45° vertical FOV, 0.3..300 clip.
    fn chase_camera(car: Vec3, aspect: f32) -> Mat4 {
        let eye = car + Vec3::new(0.0, 6.0, 12.0);
        let target = car + Vec3::new(0.0, 1.5, 0.0);
        Mat4::perspective_rh(45f32.to_radians(), aspect, NEAR, 300.0)
            * Mat4::look_at_rh(eye, target, Vec3::Y)
    }

    fn demo_sun() -> Vec3 {
        SunSky {
            sun_elevation_deg: 32.0,
            sun_azimuth_deg: 284.0,
            ..SunSky::default()
        }
        .direction()
    }

    const DIST: f32 = MAX_SHADOW_DISTANCE;

    #[test]
    fn slab_ends_where_the_fog_turns_opaque() {
        // A near fog decides the slab by itself: 98% opaque at start + 2/k.
        assert!((shadow_distance_for_fog(20.0, 0.1) - 40.0).abs() < 1e-4);
        assert!((shadow_distance_for_fog(35.0, 0.1) - 55.0).abs() < 1e-4);
        assert_eq!(shadow_distance_for_fog(35.0, 0.0), DEFAULT_SHADOW_DISTANCE);
    }

    #[test]
    fn a_far_fog_is_capped_so_shadows_stay_sharp() {
        // The default fog is 98% opaque at 105 + 30·2 = 165 units; following it
        // would triple the slab (and the texel size). The cap holds it at 55.
        let d = LightUniforms::default();
        assert!((d.fog_start + 2.0 / d.fog_density - 165.0).abs() < 1e-3);
        assert_eq!(
            shadow_distance_for_fog(d.fog_start, d.fog_density),
            MAX_SHADOW_DISTANCE
        );
        assert_eq!(shadow_distance_for_fog(1000.0, 0.001), MAX_SHADOW_DISTANCE);
        // ...and the capped fit is exactly as sharp as the pre-cap 55-unit one.
        let vp = chase_camera(Vec3::ZERO, 16.0 / 9.0);
        let capped = fit_shadow_frustum(
            vp,
            demo_sun(),
            shadow_distance_for_fog(105.0, 1.0 / 30.0),
            2048,
        );
        let uncapped = fit_shadow_frustum(vp, demo_sun(), 165.0, 2048);
        assert_eq!(
            capped.texel_world_size,
            fit_shadow_frustum(vp, demo_sun(), 55.0, 2048).texel_world_size
        );
        assert!(uncapped.texel_world_size > 2.5 * capped.texel_world_size);
    }

    #[test]
    fn slab_corners_stop_at_the_shadow_distance() {
        let vp = chase_camera(Vec3::ZERO, 16.0 / 9.0);
        let corners = visible_slab_corners(vp, DIST);
        for c in &corners[4..] {
            let w = (vp * c.extend(1.0)).w;
            assert!((w - DIST).abs() < 1e-2, "far corner at view depth {w}");
        }
        for c in &corners[..4] {
            let w = (vp * c.extend(1.0)).w;
            assert!((w - NEAR).abs() < 1e-3, "near corner at view depth {w}");
        }
    }

    #[test]
    fn fit_encloses_the_whole_visible_slab() {
        for aspect in [1.0, 16.0 / 10.0, 16.0 / 9.0, 2.2] {
            let vp = chase_camera(Vec3::new(3.0, 0.1, -123.4), aspect);
            let fit = fit_shadow_frustum(vp, demo_sun(), DIST, SHADOW_MAP_SIZE);
            for c in visible_slab_corners(vp, DIST) {
                let p = fit.project(c);
                assert!(
                    (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
                    "slab corner {c:?} falls outside the map at {p:?}"
                );
                assert!((0.0..=1.0).contains(&p.z), "corner depth {p:?} clipped");
            }
        }
    }

    #[test]
    fn projection_runs_along_the_sun_direction() {
        // Shadowing is driven by the KE-0406 sun: moving a point along the light's
        // travel direction keeps its texel and only pushes it deeper.
        let vp = chase_camera(Vec3::ZERO, 16.0 / 9.0);
        let sun = demo_sun();
        let fit = fit_shadow_frustum(vp, sun, DIST, SHADOW_MAP_SIZE);
        let p = Vec3::new(1.0, 0.0, -10.0);
        let a = fit.project(p);
        let b = fit.project(p + sun * 5.0);
        assert!((a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5);
        assert!(b.z > a.z, "further along the light must be deeper");
        // And a different sun is a different projection.
        let other = SunSky {
            sun_elevation_deg: 60.0,
            sun_azimuth_deg: 120.0,
            ..SunSky::default()
        }
        .direction();
        let refit = fit_shadow_frustum(vp, other, DIST, SHADOW_MAP_SIZE);
        assert_ne!(fit.light_view_projection, refit.light_view_projection);
    }

    #[test]
    fn radius_and_texel_size_are_stable_as_the_camera_moves_and_turns() {
        let sun = demo_sun();
        let base = fit_shadow_frustum(chase_camera(Vec3::ZERO, 16.0 / 9.0), sun, DIST, 2048);
        for i in 0..200 {
            let car = Vec3::new((i as f32 * 0.37).sin() * 3.0, 0.1, -(i as f32) * 0.731);
            let fit = fit_shadow_frustum(chase_camera(car, 16.0 / 9.0), sun, DIST, 2048);
            assert_eq!(fit.radius, base.radius, "radius breathed at step {i}");
            assert_eq!(fit.texel_world_size, base.texel_world_size);
        }
        // Turning the camera (yaw) does not change the sphere either.
        for deg in [10.0f32, 45.0, 90.0, 170.0] {
            let rot = Mat4::from_rotation_y(deg.to_radians());
            let vp = chase_camera(Vec3::ZERO, 16.0 / 9.0) * rot;
            let fit = fit_shadow_frustum(vp, sun, DIST, 2048);
            assert_eq!(fit.radius, base.radius, "radius changed at yaw {deg}");
        }
    }

    #[test]
    fn snapping_keeps_every_world_point_on_the_same_subtexel_position() {
        // The anti-shimmer guarantee: as the fit slides with the camera by
        // arbitrary (sub-texel) amounts, a fixed world point's position measured in
        // texels changes only by whole texels, so static geometry rasterises into
        // the map identically every frame and its shadow edge cannot crawl.
        let sun = demo_sun();
        let size = SHADOW_MAP_SIZE;
        let points = [
            Vec3::new(0.0, 0.0, -20.0),
            Vec3::new(-7.3, 4.2, -31.9),
            Vec3::new(5.5, -0.3, -8.25),
        ];
        let frac = |fit: &ShadowFit, p: Vec3| {
            let t = fit.project(p) * size as f32;
            Vec3::new(t.x - t.x.round(), t.y - t.y.round(), 0.0)
        };
        let base = fit_shadow_frustum(chase_camera(Vec3::ZERO, 16.0 / 9.0), sun, DIST, size);
        let mut moved = 0;
        for i in 1..120 {
            // Deliberately irrational-ish steps so the centre moves by sub-texel
            // amounts in every direction.
            let car = Vec3::new((i as f32 * 0.113).sin() * 2.9, 0.1, -(i as f32) * 0.0917);
            let fit = fit_shadow_frustum(chase_camera(car, 16.0 / 9.0), sun, DIST, size);
            if fit.center != base.center {
                moved += 1;
            }
            for p in points {
                let d = frac(&fit, p) - frac(&base, p);
                assert!(
                    d.x.abs() < 2e-2 && d.y.abs() < 2e-2,
                    "step {i}: {p:?} moved by a fraction of a texel ({d:?})"
                );
            }
        }
        assert!(
            moved > 50,
            "the fit must actually slide for this test to mean anything"
        );
    }

    #[test]
    fn unsnapped_fit_would_shimmer() {
        // Control for the test above: the same point's sub-texel position under an
        // un-snapped projection *does* wander, so the snapping is doing the work.
        let sun = demo_sun();
        let view = light_view(sun);
        let texel = 0.05;
        let p = Vec3::new(0.0, 0.0, -20.0);
        let sub = |center: Vec3| {
            let x = (view.transform_point3(p).x - view.transform_point3(center).x) / texel;
            x - x.round()
        };
        // Slide the (unsnapped) centre by a quarter texel along the map's x axis.
        let right = view.inverse().transform_vector3(Vec3::X);
        let a = sub(Vec3::new(0.0, 0.0, -30.0));
        let b = sub(Vec3::new(0.0, 0.0, -30.0) + right * 0.0125);
        assert!((a - b).abs() > 0.1);
    }

    #[test]
    fn identity_camera_does_not_produce_nans() {
        // The backend's state before any camera push (the overlay reference).
        let fit = fit_shadow_frustum(Mat4::IDENTITY, demo_sun(), DIST, SHADOW_MAP_SIZE);
        assert!(fit
            .light_view_projection
            .to_cols_array()
            .iter()
            .all(|v| v.is_finite()));
        assert!(fit.radius.is_finite() && fit.radius > 0.0);
    }

    #[test]
    fn overhead_sun_has_a_valid_light_view() {
        let m = light_view(Vec3::NEG_Y);
        assert!(m.to_cols_array().iter().all(|v| v.is_finite()));
        // Looking straight down: a point below the origin is in front (-Z).
        assert!(m.transform_point3(Vec3::new(0.0, -5.0, 0.0)).z < 0.0);
    }

    #[test]
    fn demo_fit_resolution_matches_the_documented_numbers() {
        // The module docs quote a 48-unit sphere and ~4.7 cm texels for the demo.
        let fit = fit_shadow_frustum(
            chase_camera(Vec3::ZERO, 16.0 / 9.0),
            demo_sun(),
            DIST,
            SHADOW_MAP_SIZE,
        );
        assert!((40.0..=55.0).contains(&fit.radius), "radius {}", fit.radius);
        assert!(
            fit.texel_world_size < 0.06,
            "texel {}",
            fit.texel_world_size
        );
        // The constant bias is about half a texel there (see module docs).
        let ratio = SHADOW_DEPTH_BIAS_WORLD / fit.texel_world_size;
        assert!((0.3..=0.7).contains(&ratio), "bias is {ratio} texels");
    }
}
