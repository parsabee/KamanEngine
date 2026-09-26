// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Every tuning constant the demo uses, grouped by area.
//!
//! Keeping every knob in one file makes the demo's feel (lane spacing, speed,
//! camera framing, asset placement) easy to find and retune without hunting
//! through game logic, asset loading, or procedural geometry. Nothing here is
//! read from the engine — these are purely game-side numbers the demo's own
//! code (`game.rs`, `assets.rs`, `scenery.rs`, `render.rs`) consumes.

use kaman_math::glam::Vec3;

// ---------------------------------------------------------------------------
// Lanes / speed / chase camera
// ---------------------------------------------------------------------------

/// Forward speed of the player along the streaming axis, in units/second.
pub(crate) const SPEED: f32 = 14.0;

/// The run speeds up as it goes: speed is multiplied by this factor every
/// [`SPEED_RAMP_SECONDS`] of driving, so a run gets progressively harder.
pub(crate) const SPEED_RAMP: f32 = 1.1;
/// How long it takes the speed to grow by one [`SPEED_RAMP`] factor, in seconds.
pub(crate) const SPEED_RAMP_SECONDS: f32 = 5.0;
/// Hard ceiling on the ramped speed, in world units per second.
///
/// This is a **correctness** limit, not just a difficulty knob. Collision is a
/// per-frame AABB overlap test at the car's current position, so if the car moved
/// more than the combined half-extents of car and obstacle (~1 world unit) in a
/// single fixed step it could pass straight through traffic. At the fixed timestep
/// that tunnelling threshold is ~60 u/s; this stays comfortably below it.
pub(crate) const SPEED_MAX: f32 = 45.0;
/// Number of discrete lanes.
pub(crate) const LANES: usize = 3;
/// Distance between adjacent lane centers along `X`, in world units.
pub(crate) const LANE_WIDTH: f32 = 3.0;
/// The car's resting height (its transform's `Y`), chosen so the wheels sit on
/// the road surface. Shared by the player and the obstacle cars so they align.
pub(crate) const PLAYER_Y: f32 = 0.1;
/// Half-extents of the player box (a 1×1×1 cube ⇒ 0.5 each) used for the
/// game-side overlap test.
pub(crate) const PLAYER_HALF: Vec3 = Vec3::new(0.5, 0.5, 0.5);
/// Half-extents of an obstacle box, used for the game-side overlap test.
pub(crate) const OBSTACLE_HALF: Vec3 = Vec3::new(0.5, 0.5, 0.5);
/// Spawn an obstacle on every Nth streaming slot; the rest are clear road.
pub(crate) const OBSTACLE_EVERY: i64 = 2;
/// On a restart, clear obstacles within this many units of the player (both
/// ahead and behind) so the fresh run has a safe runway.
pub(crate) const CLEAR_AHEAD: f32 = 10.0;
/// The starting lane (center) and the reset lane.
pub(crate) const START_LANE: usize = LANES / 2;
/// Fixed seed for the run's PRNG, so a headless run is fully reproducible.
pub(crate) const SEED: u64 = 0xC0FF_EE00_1234_5678;
/// How far behind the player the chase camera trails, along travel.
pub(crate) const CHASE_DISTANCE: f32 = 12.0;
/// How high above the player the chase camera sits.
pub(crate) const CHASE_HEIGHT: f32 = 6.0;
/// Height above the player the camera aims at, so the road ahead stays framed.
pub(crate) const CHASE_LOOK_AT_HEIGHT: f32 = 1.5;
/// Chase smoothing (per fixed step); small enough to ease, large enough to
/// keep the fast-moving box centered.
pub(crate) const CHASE_SMOOTHING: f32 = 0.2;
/// The car's travel direction (the streaming axis, `-Z`); the chase camera
/// trails along it.
pub(crate) const FORWARD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

// ---------------------------------------------------------------------------
// Car fit
// ---------------------------------------------------------------------------

/// Target car length in world units (the model is fit so its longer horizontal
/// extent matches this). Sized to sit within a lane with margin.
pub(crate) const CAR_LEN: f32 = 2.85;
/// World-space `Y` the fitted car's underside rests at, so it sits on the road
/// (the road surface top is at `y = -0.3`; the car entity is placed at
/// [`PLAYER_Y`]` = 0.1`, so the model bottom lands on the road at
/// `0.1 + CAR_BOTTOM`).
pub(crate) const CAR_BOTTOM: f32 = -0.4;
/// Yaw (about `+Y`) applied when fitting the model so it faces the travel
/// direction (`-Z`). Tuned to the imported models' authored orientation.
pub(crate) const CAR_YAW: f32 = std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Asset paths
// ---------------------------------------------------------------------------

/// The committed **player** car model, imported at startup (KE-0703): the CC0
/// Quaternius Sports Car (`.glb`, public domain). Resolved against this crate's
/// dir so the path holds regardless of the process working directory.
pub(crate) const PLAYER_CAR_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sports_car.glb");

/// The committed **traffic** car models (CC0 Quaternius, public domain). Each
/// obstacle draws one of these, chosen per streaming slot by
/// [`variant_for_slot`](crate::rng::variant_for_slot).
pub(crate) const TRAFFIC_CAR_ASSETS: [&str; 3] = [
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/car.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/car2.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/police_car.glb"),
];

/// The committed roadside **building** prefabs (CC0 Kenney City Kit, public
/// domain), imported at startup (KE-0706). Order matches [`BUILDING_WEIGHTS`].
pub(crate) const BUILDING_ASSETS: [&str; 8] = [
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/skyscraper_a.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/skyscraper_b.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/large_a.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/large_b.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/large_c.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/small_a.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/small_b.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/low_a.glb"),
];

/// Spawn weights per prefab (out of 100), parallel to [`BUILDING_ASSETS`]:
/// skyscrapers rare (8% total), large/mid common (~55%), small/low the rest
/// (~37%). Tunes how the streamed city reads.
pub(crate) const BUILDING_WEIGHTS: [u32; 8] = [4, 4, 18, 18, 19, 12, 12, 13];

/// The committed asphalt road-tile glTF (KE-0704), imported at startup: a flat
/// textured quad carrying the tiling UVs plus an embedded asphalt base-color PNG.
pub(crate) const ROAD_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/road.gltf");

/// The committed city skyline backdrop glTF (KE-0705): a vertical billboard quad
/// with an embedded skyline base-color PNG cropped from a CC0 photo.
pub(crate) const SKYLINE_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/skyline.gltf");

// ---------------------------------------------------------------------------
// Buildings
// ---------------------------------------------------------------------------

/// Target horizontal footprint (world units) each building is fit to; the model's
/// height scales with it, so tall prefabs stay tall and short ones short.
pub(crate) const BUILDING_FOOTPRINT: f32 = 8.0;
/// World `Y` of the ground plane the buildings' bases rest on — well below the road
/// surface (`≈ -0.3`), so the road reads as an elevated freeway above the city.
pub(crate) const GROUND_Y: f32 = -10.0;
/// Lateral offset (from center, `x = 0`) of the building row on each side of the
/// road. Beyond the road's outer edge (`±6`).
pub(crate) const BUILDING_SIDE_X: f32 = 11.0;
/// Per-slot lateral jitter added to [`BUILDING_SIDE_X`] so the rows aren't a flat
/// wall.
pub(crate) const BUILDING_SIDE_JITTER: f32 = 4.0;

// ---------------------------------------------------------------------------
// Guardrail
// ---------------------------------------------------------------------------

/// Lateral position of the guardrail on each side of the road — just inside the
/// road's outer edge (`±6`), outboard of the drivable lanes (`±4.5`).
pub(crate) const GUARDRAIL_X: f32 = 5.7;
/// World `Y` the guardrail's base sits at — the top of the road deck (the road tile
/// is centered at `y = -0.5` with half-height `0.2`, so its surface is `y = -0.3`).
pub(crate) const GUARDRAIL_DECK_Y: f32 = -0.3;
/// Height of the guardrail (top rail) above its base, in world units.
pub(crate) const GUARDRAIL_H: f32 = 0.6;
/// Spacing between guardrail posts, in world units.
pub(crate) const GUARDRAIL_POST_SPACING: f32 = 2.0;

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// Total width (along `X`) of the ground terrain sheet.
pub(crate) const TERRAIN_W: f32 = 260.0;
/// How far in front of the (trailing) camera the terrain sheet extends
/// (camera-locked, relative `Z`).
pub(crate) const TERRAIN_Z_NEAR: f32 = 30.0;
/// How far behind the terrain sheet extends (camera-locked, relative `Z`). The
/// far edge stays inside the camera's far plane (100) measured from the
/// trailing camera, so it never clips.
pub(crate) const TERRAIN_Z_FAR: f32 = -80.0;
/// Half-width of the flat valley floor the road and buildings sit on — the terrain
/// stays level at [`GROUND_Y`] out to here, so buildings rest flush, then rises.
pub(crate) const TERRAIN_FLAT_HALF: f32 = 13.0;
/// World `Y` the flanking hills crest at. Comfortably above the apparent horizon so
/// the hills fully eclipse the sky behind the buildings.
pub(crate) const HILL_CREST: f32 = 13.0;
/// Number of columns across the terrain sheet (smoothness of the hill profile).
pub(crate) const TERRAIN_COLUMNS: u32 = 96;

// ---------------------------------------------------------------------------
// Backdrop
// ---------------------------------------------------------------------------

/// How far ahead of the player (along the travel axis, `-Z`) the skyline backdrop
/// sits. Kept well under the camera far plane (100) so it never clips; far enough
/// that the distance fog blends it toward the horizon so it reads as a far skyline.
pub(crate) const BACKDROP_DIST: f32 = 45.0;
/// Backdrop billboard width in world units — wide enough to span the view frustum
/// at [`BACKDROP_DIST`].
pub(crate) const BACKDROP_W: f32 = 150.0;
/// Backdrop billboard height in world units. One full image height renders as 17
/// units; the extra height extends the frame **downward** (the generator's
/// `V_SPAN` matches, so the picture keeps its scale and position and the extra
/// frame just shows more of the image's bottom). Keep in sync with the
/// `FRAME_WORLD_H` constant in `examples/gen_skyline.rs`.
pub(crate) const BACKDROP_H: f32 = 24.0;
/// World `Y` of the backdrop's center. Chosen so the frame's **top edge stays at
/// `13.5`** while [`BACKDROP_H`] grows downward (`Y = 13.5 - H/2`), extending the
/// frame's bottom to cover the mid-ground without moving the skyline picture.
pub(crate) const BACKDROP_Y: f32 = 1.5;

// ---------------------------------------------------------------------------
// HUD (KE-0404 / KE-0707)
// ---------------------------------------------------------------------------

/// The committed SDF font atlas the HUD draws with, baked by the `gen_font`
/// example from the OFL-licensed `assets/font.ttf`.
pub(crate) const FONT_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/font.bin");

/// Padding, in pixels, between the safe-area edge and the HUD's live score.
pub(crate) const HUD_MARGIN: f32 = 24.0;
/// Em size, in pixels, of the live score readout.
pub(crate) const HUD_SCORE_PX: f32 = 30.0;
/// Em size of the "GAME OVER" banner headline.
pub(crate) const HUD_TITLE_PX: f32 = 64.0;
/// Em size of the banner's supporting lines (final score, best, replay prompt).
pub(crate) const HUD_BANNER_PX: f32 = 30.0;
/// HUD text color (near-white, fully opaque).
pub(crate) const HUD_TEXT_COLOR: [f32; 4] = [0.96, 0.96, 0.98, 1.0];
/// Full-screen dim drawn behind the game-over banner so it stays readable.
pub(crate) const HUD_DIM_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.55];

/// How long the opening black screen takes to fade away once a run starts.
pub(crate) const HUD_FADE_SECONDS: f32 = 0.75;
/// Opaque black, used for the title screen and the fade that dissolves it.
pub(crate) const HUD_BLACK_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
