// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! `playable-demo` — KamanEngine's first title: a playable endless runner, and the
//! host for the headless smoke oracle.
//!
//! This binary is the *only* place game-specific code lives: it implements
//! [`kaman_core::Game`] and is driven by the engine loop through the
//! [`EngineCtx`](kaman_core::EngineCtx) seam. The engine crates never see any of
//! the car / road / lane / obstacle / score concepts modelled here — they are
//! composed entirely from the engine-generic ECS, physics, and scene-streaming
//! primitives.
//!
//! # The game
//!
//! The player is a box that drives forward at a constant speed along the
//! streaming axis (`-Z`). Three discrete lanes run along `X`; Left/A and Right/D
//! move the box between them (kinematic — the transform is set directly, the
//! physics solver never drives the box, per ARCHITECTURE §5). The road and its
//! obstacles are produced by [`Scene::stream`](kaman_scene::Scene::stream) with
//! the box as the focus, so the world scrolls endlessly and content behind the
//! player despawns. Obstacles appear in deterministic (seeded PRNG) lanes ahead;
//! colliding with one resets the run. Score climbs with distance travelled.
//!
//! # Controls
//!
//! - **Left / A** — move one lane left.
//! - **Right / D** — move one lane right.
//! - **Space** — restart after a crash (or any time; resets the run).
//! - **Escape** — quit (handled by the engine's windowed entry).
//!
//! # The smoke oracle
//!
//! Without a window the binary runs the `--smoke` oracle: it boots the [`Game`]
//! via the engine's headless driver, runs 120 frames offscreen against a
//! `NullRenderer` with **no GPU / Metal device**, and exits 0. Headless input is
//! empty, so the box just runs straight down the middle lane — a deterministic
//! run valid on headless CI (see `docs/INTEGRATION.md` §1).

use clap::Parser;

use kaman_assets::AssetCache;
use kaman_camera::ChaseController;
use kaman_core::input::Key;
use kaman_core::{EngineCtx, Game, Renderer};
use kaman_ecs::hecs::Entity;
use kaman_ecs::{DynamicTag, PhysicsBodyComponent, RenderComponent, StaticTag, TransformComponent};
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render_api::{
    MaterialParams, MeshData, MeshHandle, PipelineDescriptor, PipelineHandle, TextureData,
    TextureHandle, VertexLayout,
};
use kaman_scene::Scene;

/// Number of frames the smoke oracle simulates before exiting.
const SMOKE_FRAMES: u32 = 120;

/// One drawable piece of an imported car: its `(fitted transform, mesh handle,
/// base-color texture)`. A textured part (`Some`) draws on the textured pipeline
/// with its texture bound; an untextured part (`None`) draws on the Phong pipeline
/// with the material base-color the importer packed as its vertex color.
type CarPart = (Transform, MeshHandle, Option<TextureHandle>);

/// Which imported model a placed entity draws: the player's car, one of the
/// traffic-car variants, or one of the roadside building prefabs.
#[derive(Debug, Clone, Copy)]
enum PlacedModel {
    /// The player's car.
    Player,
    /// A traffic car, by variant index (into [`CarRunner::traffic_cars`]).
    Traffic(usize),
    /// A roadside building, by prefab index (into [`CarRunner::buildings`]).
    Building(usize),
}

/// A game-side ECS component tagging an obstacle with which traffic car variant it
/// draws (index into [`TRAFFIC_CAR_ASSETS`]). Assigned once at spawn from the
/// streaming slot so an obstacle keeps the same car for its whole life.
#[derive(Debug, Clone, Copy)]
struct TrafficVariant(usize);

/// A game-side ECS component tagging a streamed roadside building with which prefab
/// it draws (index into [`BUILDING_ASSETS`]). Assigned once at spawn (deterministic
/// per slot + side) so a building keeps the same prefab for its whole life. A
/// building is **non-colliding** decoration — it carries no physics body.
#[derive(Debug, Clone, Copy)]
struct BuildingVariant(usize);

/// A game-side ECS marker for a streamed guardrail segment (KE-0706), so the
/// renderer draws it with the shared guardrail mesh and excludes it from the road
/// pass. Non-colliding decoration.
#[derive(Debug, Clone, Copy)]
struct GuardrailTag;

/// Command-line arguments for `playable-demo`.
#[derive(Parser, Debug)]
#[command(name = "playable-demo", about = "KamanEngine playable-demo + headless smoke oracle")]
struct Cli {
    /// Run the headless smoke oracle: boot the game and drive a fixed frame loop, then exit 0.
    ///
    /// Requires no GPU/Metal device — safe on headless CI runners.
    #[arg(long)]
    smoke: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.smoke {
        run_smoke(SMOKE_FRAMES);
        return;
    }

    // Windowed path: boots the same `Game` under the winit entry, driving a real
    // Metal backend. The backend is constructed below the render seam by
    // `kaman-render` and injected via a factory, so `kaman-core` never depends on
    // `metal` (ARCHITECTURE §2).
    let mut game = CarRunner::new();
    run_windowed(&mut game);
}

/// Launch the windowed engine with the Metal backend on macOS.
#[cfg(target_os = "macos")]
fn run_windowed(game: &mut CarRunner) {
    use kaman_core::Renderer;
    kaman_core::run_with_backend(
        game,
        Box::new(|window, width, height| {
            Box::new(kaman_render::MetalRenderer::new(window, width, height)) as Box<dyn Renderer>
        }),
    );
}

/// Non-macOS fallback: no Metal backend, run against the null seam.
#[cfg(not(target_os = "macos"))]
fn run_windowed(game: &mut CarRunner) {
    kaman_core::run(game);
}

/// Boot the `playable-demo` [`Game`] and drive `frames` frames headlessly, then report success.
fn run_smoke(frames: u32) {
    let mut game = CarRunner::new();
    let harness = kaman_core::headless::run(&mut game, frames);

    // The engine must have driven the game for exactly the requested frame count.
    assert_eq!(harness.frames_run(), frames, "driver ran the wrong frame count");

    println!("{}", smoke_report(frames));
}

/// The exact stdout contract line the smoke oracle prints on success.
fn smoke_report(frames: u32) -> String {
    format!("smoke: {frames} frames OK")
}

/// A tiny deterministic PRNG (SplitMix64) so obstacle placement is reproducible
/// from a seed — no wall-clock time is ever read, which the smoke path and the
/// game-logic tests rely on.
///
/// This lives in the game crate on purpose: randomness is a *game* concern, not
/// an engine one, so it stays out of the `kaman-*` crates.
#[derive(Debug, Clone)]
struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the generator. The same seed always yields the same sequence.
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64-bit value (SplitMix64).
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n > 0`).
    fn next_below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n)) as u32
    }
}

/// The playable-demo game state.
///
/// The scene (ECS world + physics world + streaming) is owned by the engine loop
/// and reached through [`EngineCtx::scene_mut`]. This struct holds only the
/// *game's* own bookkeeping.
///
/// # Two distances
///
/// The world scrolls forever, so the player's **`travel`** (its monotonically
/// increasing world position along the axis) never rewinds — that keeps the
/// scene's spawn frontier valid, since the engine-owned streaming frontier is
/// private and cannot be reset. The **`distance`** the *score* is built from is a
/// separate tally that resets to zero on a crash. So "restart the run" means:
/// score back to zero and the obstacles around the player cleared, while the road
/// keeps scrolling seamlessly.
struct CarRunner {
    /// The player-controlled box entity (spawned once in `init`).
    player: Option<Entity>,
    /// The lane the player currently occupies (`0..LANES`).
    lane: usize,
    /// Monotonic world distance travelled (drives the player's `Z` and the
    /// streaming focus). Never rewinds, so the scene frontier stays valid.
    travel: f32,
    /// Score distance for the current run; resets to zero on a crash.
    distance: f32,
    /// Best run distance this session, for the crash report.
    best: f32,
    /// Deterministic obstacle-placement PRNG (seeded once; the same seed always
    /// streams the same obstacle world).
    rng: Rng,
    /// The untextured Phong pipeline (`[pos,normal,color]`), created in `init`.
    /// The cars are drawn on it; per-vertex color carries their look.
    pipeline: Option<PipelineHandle>,
    /// The **textured** pipeline (`[pos,normal,uv]`), created in `init`. The road
    /// tiles are drawn on it, sampling the asphalt base-color texture.
    textured_pipeline: Option<PipelineHandle>,
    /// The road-tile mesh — a flat quad carrying the tiling UVs, imported once from
    /// `assets/road.gltf` (KE-0704) and scaled by each tile's transform.
    road_mesh: Option<MeshHandle>,
    /// The asphalt base-color texture uploaded once in `init` (KE-0704); bound
    /// before the road-tile draws so the textured pipeline samples it.
    asphalt: Option<TextureHandle>,
    /// The guardrail segment mesh (KE-0706) — a procedural rail + posts one
    /// `spawn_interval` long, built once and streamed along both road edges. Drawn
    /// on the untextured pipeline (its metal color is baked into the vertices).
    guardrail_mesh: Option<MeshHandle>,
    /// The hill-terrain ground mesh (KE-0706) — a wide ground surface built once and
    /// drawn camera-locked: a flat valley floor **under** the road and buildings
    /// that rises into hills on both flanks, so the buildings sit on ground and the
    /// sky doesn't show through the mid-ground.
    hills_mesh: Option<MeshHandle>,
    /// The distant city skyline backdrop — a billboard quad imported once from
    /// `assets/skyline.gltf` (KE-0705), drawn far ahead and locked to the camera's
    /// XZ so it reads as a far skyline behind the fog.
    backdrop_mesh: Option<MeshHandle>,
    /// The skyline base-color texture (KE-0705), uploaded once and bound before the
    /// backdrop draw.
    backdrop_texture: Option<TextureHandle>,
    /// The **player** car model, imported once from the CC0 `assets/sports_car.glb`
    /// (KE-0703): one [`CarPart`] per drawable mesh-node, fit to the road. Uploaded
    /// once (KE-0103) and reused every frame.
    player_car: Vec<CarPart>,
    /// The **traffic** car models — the CC0 `assets/{car,car2,police_car}.glb`
    /// ([`TRAFFIC_CAR_ASSETS`]). Each obstacle draws one of these, chosen
    /// deterministically per streaming slot ([`variant_for_slot`]). All uploaded
    /// once (KE-0103) and shared across every obstacle on screen.
    traffic_cars: Vec<Vec<CarPart>>,
    /// The roadside **building** prefabs — the CC0 Kenney City Kit models
    /// ([`BUILDING_ASSETS`]), imported once and streamed along both sides of the
    /// elevated road. Each streamed building draws one of these, chosen by a
    /// weighted per-slot pick ([`building_for_slot`]); shared across all instances.
    buildings: Vec<Vec<CarPart>>,
    /// The chase camera controller that keeps the player framed (KE-0205). It
    /// trails the box from behind and above along the travel axis, with light
    /// smoothing so the follow eases rather than snapping.
    chase: ChaseController,
    /// Whether the run is live or crashed. `Playing` advances the world and reads
    /// Left/Right; `GameOver` freezes the run and waits for the replay key.
    state: GameState,
}

/// The demo's tiny game-state machine: drive until you crash, then replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameState {
    /// Driving: the world scrolls, Left/Right change lanes, collisions end the run.
    Playing,
    /// Crashed: the world is frozen and the score reported; a replay key restarts.
    GameOver,
}

impl CarRunner {
    /// Forward speed of the player along the streaming axis, in units/second.
    const SPEED: f32 = 14.0;
    /// Number of discrete lanes.
    const LANES: usize = 3;
    /// Distance between adjacent lane centers along `X`, in world units.
    const LANE_WIDTH: f32 = 3.0;
    /// The car's resting height (its transform's `Y`), chosen so the wheels sit on
    /// the road surface. Shared by the player and the obstacle cars so they align.
    const PLAYER_Y: f32 = 0.1;
    /// Half-extents of the player box (a 1×1×1 cube ⇒ 0.5 each) used for the
    /// game-side overlap test.
    const PLAYER_HALF: Vec3 = Vec3::new(0.5, 0.5, 0.5);
    /// Half-extents of an obstacle box, used for the game-side overlap test.
    const OBSTACLE_HALF: Vec3 = Vec3::new(0.5, 0.5, 0.5);
    /// Spawn an obstacle on every Nth streaming slot; the rest are clear road.
    const OBSTACLE_EVERY: i64 = 2;
    /// On a restart, clear obstacles within this many units of the player (both
    /// ahead and behind) so the fresh run has a safe runway.
    const CLEAR_AHEAD: f32 = 10.0;
    /// The starting lane (center) and the reset lane.
    const START_LANE: usize = Self::LANES / 2;
    /// Fixed seed for the run's PRNG, so a headless run is fully reproducible.
    const SEED: u64 = 0xC0FF_EE00_1234_5678;
    /// How far behind the player the chase camera trails, along travel.
    const CHASE_DISTANCE: f32 = 12.0;
    /// How high above the player the chase camera sits.
    const CHASE_HEIGHT: f32 = 6.0;
    /// Height above the player the camera aims at, so the road ahead stays framed.
    const CHASE_LOOK_AT_HEIGHT: f32 = 1.5;
    /// Chase smoothing (per fixed step); small enough to ease, large enough to
    /// keep the fast-moving box centered.
    const CHASE_SMOOTHING: f32 = 0.2;
    /// The car's travel direction (the streaming axis, `-Z`); the chase camera
    /// trails along it.
    const FORWARD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

    /// World-space `X` of a lane center.
    fn lane_x(lane: usize) -> f32 {
        // Center lanes about x=0: lane 0 → -LANE_WIDTH, center → 0, etc.
        (lane as f32 - (Self::LANES as f32 - 1.0) / 2.0) * Self::LANE_WIDTH
    }

    /// The player's current world position (lane along `X`, monotonic travel along
    /// `-Z`). A pure function of game state, so it matches the ECS transform and is
    /// used for the streaming focus and the overlap test.
    fn player_position(&self) -> Vec3 {
        Vec3::new(Self::lane_x(self.lane), Self::PLAYER_Y, -self.travel)
    }

    /// Create an unspawned game; [`init`](Game::init) populates the world.
    fn new() -> Self {
        Self {
            player: None,
            lane: Self::START_LANE,
            travel: 0.0,
            distance: 0.0,
            best: 0.0,
            rng: Rng::new(Self::SEED),
            pipeline: None,
            textured_pipeline: None,
            road_mesh: None,
            asphalt: None,
            guardrail_mesh: None,
            hills_mesh: None,
            backdrop_mesh: None,
            backdrop_texture: None,
            player_car: Vec::new(),
            traffic_cars: Vec::new(),
            buildings: Vec::new(),
            chase: ChaseController::new(Self::CHASE_DISTANCE, Self::CHASE_HEIGHT)
                .with_look_at_height(Self::CHASE_LOOK_AT_HEIGHT)
                .with_smoothing(Self::CHASE_SMOOTHING),
            state: GameState::Playing,
        }
    }

    /// The score derived from the current run's distance (1 point per world unit).
    fn score(&self) -> u64 {
        self.distance.max(0.0) as u64
    }

    /// Apply lane-switch input for this fixed step: **discrete, edge-triggered**.
    ///
    /// `Left`/`Right` are the only gameplay keys; each *press* snaps the car exactly
    /// one lane toward the edge (clamped), so a held key does not glide across lanes
    /// (KE-0702, via [`InputState::is_key_just_pressed`]). Both at once cancel.
    fn apply_lane_input(&mut self, ctx: &EngineCtx) {
        let input = ctx.input();
        let left = input.is_key_just_pressed(Key::Left);
        let right = input.is_key_just_pressed(Key::Right);
        match (left, right) {
            (true, false) => self.lane = self.lane.saturating_sub(1),
            (false, true) => self.lane = (self.lane + 1).min(Self::LANES - 1),
            _ => {}
        }
    }

    /// Whether the player box overlaps `obstacle_pos` — a game-side AABB test
    /// (same-lane proximity along `X` and overlap along the travel axis `Z`).
    ///
    /// This is deliberately a *game* computation over entity transforms: it does
    /// **not** query `kaman-physics` for contacts (that would be an engine API
    /// change, out of scope for this A0 ticket).
    fn overlaps(&self, obstacle_pos: Vec3) -> bool {
        let player = self.player_position();
        let dx = (player.x - obstacle_pos.x).abs();
        let dz = (player.z - obstacle_pos.z).abs();
        dx < Self::PLAYER_HALF.x + Self::OBSTACLE_HALF.x
            && dz < Self::PLAYER_HALF.z + Self::OBSTACLE_HALF.z
    }

    /// Spawn the player box at `position` and return its entity.
    fn spawn_player(scene: &mut Scene, position: Vec3) -> Entity {
        scene.world_mut().spawn((
            TransformComponent::from_position(position),
            RenderComponent::cube([0.9, 0.15, 0.1]),
            DynamicTag,
        ))
    }

    /// End the run: record the best score, report it, and enter `GameOver`.
    ///
    /// The world is left frozen (the crashed car stays put) until the player hits
    /// the replay key; reporting is stdout for now (the on-screen HUD is KE-0707).
    fn game_over(&mut self) {
        self.best = self.best.max(self.distance);
        self.state = GameState::GameOver;
        println!(
            "GAME OVER — score {} — best {}. Press Space to replay.",
            self.score(),
            self.best as u64
        );
    }

    /// Start a fresh run (from `GameOver`): recenter the lane, zero the score,
    /// clear the obstacles around the player so it doesn't instantly re-collide,
    /// and go back to `Playing`. The world keeps scrolling — `travel` is monotonic,
    /// so the scene's (private) spawn frontier stays valid and the road ahead is
    /// unbroken. Fully deterministic: no PRNG reseed, no wall-clock read.
    fn start_new_run(&mut self, ctx: &mut EngineCtx) {
        // Clear obstacles within a window around the player so the restart lane is
        // safe. Obstacles are the streamed entities that carry a physics body;
        // despawn removes the ECS entity and its rigid body atomically.
        let player_along = self.player_position().dot(ctx.scene().config().axis);
        let window = Self::CLEAR_AHEAD;
        let victims: Vec<Entity> = ctx
            .world()
            .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
            .iter()
            .filter(|(_e, (t, _))| {
                let d = t.transform.position.dot(ctx.scene().config().axis);
                (d - player_along).abs() <= window
            })
            .map(|(e, _)| e)
            .collect();
        let scene = ctx.scene_mut();
        for e in victims {
            scene.despawn(e);
        }

        self.lane = Self::START_LANE;
        self.distance = 0.0;
        self.state = GameState::Playing;
        println!("replay — score {}. go!", self.best as u64);
    }

    /// Fill one streaming slot with a road tile and, on the obstacle cadence, an
    /// obstacle in a deterministically-chosen lane.
    ///
    /// Both road and obstacle entities are reported via
    /// [`SpawnCtx::spawned`](kaman_scene::SpawnCtx::spawned) so the scene despawns
    /// them (and any physics body) once they fall behind the player.
    fn spawn_slot(rng: &mut Rng, tile_depth: f32, cx: &mut kaman_scene::SpawnCtx<'_>) {
        // Road tile: a wide, flat, dark box centered across all lanes at this slot.
        let road_z = cx.position.z;
        let road_width = Self::LANES as f32 * Self::LANE_WIDTH + Self::LANE_WIDTH;
        let road = cx.world.spawn((
            TransformComponent::new(Transform {
                position: Vec3::new(0.0, -0.5, road_z),
                scale: Vec3::new(road_width, 0.4, tile_depth),
                ..Transform::identity()
            }),
            RenderComponent::cube([0.16, 0.16, 0.2]),
            StaticTag,
        ));
        cx.spawned(road);

        // Obstacle cadence: not every slot, and never at slot 0 (right on the
        // player's start) so the very first frame is always survivable.
        if cx.slot != 0 && cx.slot.rem_euclid(Self::OBSTACLE_EVERY) == 0 {
            let lane = rng.next_below(Self::LANES as u32) as usize;
            let pos = Vec3::new(Self::lane_x(lane), Self::PLAYER_Y, road_z);

            // Give the obstacle a static physics body + collider so streaming's
            // atomic despawn (ECS entity + rigid body together) is exercised. The
            // body is not used to drive the player; collision is a game-side AABB.
            let handle = cx.physics.create_static_body(Transform::from_position(pos));
            cx.physics.add_box_collider(handle, Self::OBSTACLE_HALF);

            // Pick a traffic car variant for this obstacle deterministically from
            // its slot — independent of the lane PRNG, so the obstacle world (and
            // the smoke run) is unchanged by adding variety.
            let variant = variant_for_slot(cx.slot, TRAFFIC_CAR_ASSETS.len());

            let obstacle = cx.world.spawn((
                TransformComponent::from_position(pos),
                kaman_ecs::PhysicsBodyComponent::new(handle),
                RenderComponent::cube([0.95, 0.8, 0.1]),
                TrafficVariant(variant),
                StaticTag,
            ));
            cx.spawned(obstacle);
        }

        // Roadside buildings (KE-0706): one on each side of the road at this slot,
        // sitting on the ground plane BELOW the road so the freeway reads as raised.
        // Non-colliding decoration (no physics body); deterministic per slot + side
        // (weighted prefab, jittered offset) and independent of the lane PRNG.
        for side in 0..2u64 {
            let sign = if side == 0 { -1.0 } else { 1.0 };
            let variant = building_for_slot(cx.slot, side);
            let jitter = (hash_u64(cx.slot as u64 ^ (side << 40) ^ 0xB57D) % 1000) as f32 / 1000.0;
            let x = sign * (BUILDING_SIDE_X + jitter * BUILDING_SIDE_JITTER);
            // Turn each building 90° to face the freeway: the left row (`x < 0`)
            // faces `+X` toward the road, the right row (`x > 0`) faces `-X`.
            let facing = Quat::from_rotation_y(-sign * std::f32::consts::FRAC_PI_2);
            let building = cx.world.spawn((
                TransformComponent::new(Transform {
                    position: Vec3::new(x, GROUND_Y, road_z),
                    rotation: facing,
                    ..Transform::identity()
                }),
                RenderComponent::cube([0.5, 0.5, 0.5]),
                BuildingVariant(variant),
                StaticTag,
            ));
            cx.spawned(building);
        }

        // Guardrails (KE-0706): a rail + posts segment along each road edge at this
        // slot, sitting on the road deck. The shared mesh is one `spawn_interval`
        // long, so tile-to-tile segments abut seamlessly. Non-colliding decoration.
        for side in 0..2u64 {
            let sign = if side == 0 { -1.0 } else { 1.0 };
            let rail = cx.world.spawn((
                TransformComponent::from_position(Vec3::new(sign * GUARDRAIL_X, GUARDRAIL_DECK_Y, road_z)),
                RenderComponent::cube([0.6, 0.6, 0.6]),
                GuardrailTag,
                StaticTag,
            ));
            cx.spawned(rail);
        }
    }

    /// Test the player against every streamed obstacle; return `true` on a hit.
    ///
    /// An obstacle is any streamed entity with `PhysicsBodyComponent` (only
    /// obstacles get one — road tiles do not), so this reads their transforms and
    /// runs the game-side overlap test.
    fn hit_any_obstacle(&self, ctx: &EngineCtx) -> bool {
        for (_e, (t, _body)) in ctx
            .world()
            .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
            .iter()
        {
            if self.overlaps(t.transform.position) {
                return true;
            }
        }
        false
    }
}

impl Game for CarRunner {
    fn init(&mut self, ctx: &mut EngineCtx) {
        // The scene is owned by the loop and created with the engine-default
        // `StreamingConfig` — which streams along `-Z`, exactly our travel axis
        // (adjusting it would need an engine API, out of scope for this A0
        // ticket). We drive `stream` / `maybe_rebase` against that scene; road
        // tiles are sized to the scene's own `spawn_interval` so they tile flush.
        let start = self.player_position();
        let scene = ctx.scene_mut();
        self.player = Some(Self::spawn_player(scene, start));

        // The cars are **imported** through `kaman-assets` (KE-0402/KE-0703) from
        // committed CC0 `.glb` models (Quaternius, public domain): the player drives
        // `assets/sports_car.glb`; the traffic cars are `assets/{car,car2,police_car}.glb`.
        // `load_car` fits each model to the road (uniform scale + orientation +
        // resting height) and uploads its meshes once (KE-0103) plus any base-color
        // textures. A car part with a texture draws on the **textured** pipeline
        // (KE-0403); an untextured part draws on the Phong pipeline with its packed
        // material color. The road tile is likewise a textured quad from
        // `assets/road.gltf` (KE-0704), drawn on the textured pipeline with the
        // asphalt texture.
        //
        // The guardrail segment mesh (KE-0706) is sized to the scene's
        // `spawn_interval` so streamed segments abut seamlessly.
        let tile_depth = ctx.scene().config().spawn_interval;
        let renderer = ctx.renderer();

        // Untextured Phong pipeline (kept for any untextured car part / future use).
        self.pipeline = Some(renderer.create_pipeline(&PipelineDescriptor {
            vertex_shader: "vertex_main".into(),
            fragment_shader: "fragment_main".into(),
            vertex_layout: color_layout(),
        }));

        // Textured pipeline (`[pos,normal,uv]`) for the asphalt road + the car
        // (KE-0403).
        self.textured_pipeline = Some(renderer.create_pipeline(&PipelineDescriptor {
            vertex_shader: "textured_vertex_main".into(),
            fragment_shader: "textured_fragment_main".into(),
            vertex_layout: textured_layout(),
        }));

        let mut cache = AssetCache::new();

        // Import the road tile once: the imported quad carries the tiling UVs and
        // is packed on the textured layout (its material has a base-color texture),
        // so `AssetCache::load` uploads it as a textured mesh. Its decoded asphalt
        // base-color PNG becomes the bound texture.
        let (road_mesh, asphalt) = load_textured_mesh(&mut cache, renderer, ROAD_ASSET);
        self.road_mesh = Some(road_mesh);
        self.asphalt = Some(asphalt);

        // Distant city skyline backdrop (KE-0705): a textured billboard quad,
        // uploaded once and drawn far ahead, locked to the camera's XZ.
        let (backdrop_mesh, backdrop_texture) =
            load_textured_mesh(&mut cache, renderer, SKYLINE_ASSET);
        self.backdrop_mesh = Some(backdrop_mesh);
        self.backdrop_texture = Some(backdrop_texture);

        self.player_car = load_model(&mut cache, renderer, PLAYER_CAR_ASSET, fit_transform);
        self.traffic_cars = TRAFFIC_CAR_ASSETS
            .iter()
            .map(|path| load_model(&mut cache, renderer, path, fit_transform))
            .collect();

        // Roadside building prefabs (KE-0706): imported + uploaded once each, fit to
        // a common footprint (heights preserved), base at the model origin so the
        // spawn transform drops each onto the ground plane below the road.
        self.buildings = BUILDING_ASSETS
            .iter()
            .map(|path| load_model(&mut cache, renderer, path, building_fit))
            .collect();

        // Guardrail segment mesh (KE-0706): built once (a rail + posts one
        // `spawn_interval` long) and streamed along both road edges.
        let (gr_vertices, gr_indices) = guardrail_geometry(tile_depth);
        self.guardrail_mesh = Some(renderer.create_mesh(&MeshData {
            vertices: &gr_vertices,
            indices: &gr_indices,
            layout: color_layout(),
        }));

        // Ground terrain sheet (KE-0706): built once, drawn camera-locked — a level
        // valley floor under the road/buildings that climbs into hills on the flanks.
        let (terrain_vertices, terrain_indices) = terrain_geometry();
        self.hills_mesh = Some(renderer.create_mesh(&MeshData {
            vertices: &terrain_vertices,
            indices: &terrain_indices,
            layout: color_layout(),
        }));

        // Prime the road ahead so the first frame is not empty.
        let focus = self.player_position();
        let tile_depth = ctx.scene().config().spawn_interval;
        let rng = &mut self.rng;
        ctx.scene_mut()
            .stream(focus, |cx| Self::spawn_slot(rng, tile_depth, cx));
    }

    fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
        // Game over: the world is frozen; wait for the replay key, keep the camera
        // framing the crashed car, and do nothing else.
        if self.state == GameState::GameOver {
            if ctx.input().is_key_just_pressed(Key::Space) {
                self.start_new_run(ctx);
            }
            let player_pos = self.player_position();
            self.chase.follow(ctx.camera_mut(), player_pos, Self::FORWARD);
            return;
        }

        // Playing: discrete Left/Right lane changes (kinematic, no solver).
        self.apply_lane_input(ctx);

        // Advance forward by a fixed step. `dt` is always `FIXED_DT`, so the
        // simulation is framerate-independent (KE-0201) — no wall-clock is read.
        // `travel` (world position) and `distance` (score) advance together; only
        // `distance` rewinds on a crash.
        self.travel += Self::SPEED * dt;
        self.distance += Self::SPEED * dt;

        // Streaming cadence: rebase FIRST (between physics steps), against the
        // focus. A rebase shifts every streamed transform back toward the origin;
        // the player is kinematic and rebuilt from `travel`, so we fold the same
        // shift into `travel` to stay aligned with the shifted world.
        let axis = ctx.scene().config().axis;
        let offset = ctx.scene_mut().maybe_rebase(self.player_position());
        if offset != Vec3::ZERO {
            // `travel` is measured along +axis from the origin; shifting the world
            // by `offset` moves the player's along-axis coordinate by offset·axis.
            self.travel += offset.dot(axis);
        }

        // Drive the player's transform kinematically to its lane + travel position,
        // now consistent with any rebase this step.
        let player_pos = self.player_position();
        if let Some(p) = self.player {
            if let Ok(mut t) = ctx.world_mut().get::<&mut TransformComponent>(p) {
                t.transform.position = player_pos;
            }
        }

        // Follow the box with the chase camera (KE-0205): trail it from behind and
        // above along the travel axis, looking at it. The engine pushes the
        // camera's view-projection across the render seam before `render`, so the
        // car stays framed. Done in `update` (post-rebase) so the camera tracks
        // the same shifted world the draws use.
        self.chase
            .follow(ctx.camera_mut(), player_pos, Self::FORWARD);

        // Then stream ahead / despawn behind, with the player as the focus.
        let tile_depth = ctx.scene().config().spawn_interval;
        let rng = &mut self.rng;
        ctx.scene_mut()
            .stream(player_pos, |cx| Self::spawn_slot(rng, tile_depth, cx));

        // Collision → game over (freeze the run; the player replays with Space).
        if self.hit_any_obstacle(ctx) {
            self.game_over();
            return;
        }

        // Score: report on each whole-unit milestone so stdout shows progress
        // without spamming every frame.
        let prev = (self.distance - Self::SPEED * dt).max(0.0) as u64;
        if self.score() / 50 != prev / 50 {
            println!("score: {}", self.score());
        }
    }

    fn render(&mut self, ctx: &mut EngineCtx) {
        // Draws are grouped by pipeline. Road tiles (every renderable that is
        // neither the player nor an obstacle) go on the **textured** pipeline with
        // the asphalt texture bound; the cars go on the untextured Phong pipeline.
        //
        // Pick each entity's mesh(es) by role: the player is the red car; obstacles
        // (the streamed entities that carry a physics body) are the white car; every
        // other renderable is a road tile. The cars are imported glTF made of
        // several primitives, so a car entity expands into one draw per part, each
        // at the entity's transform composed with the part's baked node transform.
        // The road tile is a single textured quad, scaled per tile by its transform.
        // All meshes/textures are persistent (KE-0103).
        let player_entity = self.player;
        let road = self.road_mesh.expect("road mesh created in init");
        let guardrail = self.guardrail_mesh.expect("guardrail mesh created in init");

        // Road-tile transforms (textured pass): every renderable that is not a car
        // (player / obstacle), not a building, and not a guardrail — i.e. road tiles.
        let road_draws: Vec<Transform> = ctx
            .world()
            .query::<(&TransformComponent, &RenderComponent)>()
            .iter()
            .filter(|(e, _)| {
                Some(*e) != player_entity
                    && ctx.world().get::<&PhysicsBodyComponent>(*e).is_err()
                    && ctx.world().get::<&BuildingVariant>(*e).is_err()
                    && ctx.world().get::<&GuardrailTag>(*e).is_err()
            })
            .map(|(_, (t, _))| t.transform)
            .collect();

        // Guardrail segment transforms (untextured pass).
        let guardrail_draws: Vec<Transform> = ctx
            .world()
            .query::<(&TransformComponent, &GuardrailTag)>()
            .iter()
            .map(|(_, (t, _))| t.transform)
            .collect();

        // Model placements: each imported-model entity's transform + which model it
        // draws — the player's car, a traffic car (obstacle), or a roadside building.
        let model_draws: Vec<(Transform, PlacedModel)> = ctx
            .world()
            .query::<(&TransformComponent, &RenderComponent)>()
            .iter()
            .filter_map(|(e, (t, _))| {
                if Some(e) == player_entity {
                    Some((t.transform, PlacedModel::Player))
                } else if ctx.world().get::<&PhysicsBodyComponent>(e).is_ok() {
                    let variant = ctx.world().get::<&TrafficVariant>(e).map(|v| v.0).unwrap_or(0);
                    Some((t.transform, PlacedModel::Traffic(variant)))
                } else if let Ok(b) = ctx.world().get::<&BuildingVariant>(e) {
                    Some((t.transform, PlacedModel::Building(b.0)))
                } else {
                    None
                }
            })
            .collect();

        let textured_pipeline = self.textured_pipeline.expect("textured pipeline in init");
        let pipeline = self.pipeline.expect("pipeline created in init");
        let asphalt = self.asphalt.expect("asphalt texture created in init");
        let backdrop = self.backdrop_mesh.expect("backdrop mesh created in init");
        let backdrop_texture = self.backdrop_texture.expect("backdrop texture created in init");
        let backdrop_transform = self.backdrop_transform();
        let terrain = self.hills_mesh.expect("terrain mesh created in init");
        let terrain_transform = self.terrain_transform();
        let renderer = ctx.renderer();
        renderer.begin_frame();

        // Distant skyline backdrop first (KE-0705): far ahead and locked to the
        // camera's XZ, so it sits behind the gameplay and in front of the gradient
        // sky, blended toward the horizon by the distance fog.
        renderer.set_pipeline(textured_pipeline);
        renderer.bind_texture(backdrop_texture);
        renderer.draw_mesh(backdrop, &backdrop_transform, &MaterialParams::default());

        // Textured pass: the asphalt road, then every textured car part (each binds
        // its own base-color texture).
        renderer.bind_texture(asphalt);
        for transform in &road_draws {
            renderer.draw_mesh(road, transform, &MaterialParams::default());
        }
        for (entity_transform, model) in &model_draws {
            for (local, mesh, texture) in self.model_parts(*model) {
                if let Some(texture) = texture {
                    renderer.bind_texture(*texture);
                    renderer.draw_mesh(*mesh, &compose(*entity_transform, local), &MaterialParams::default());
                }
            }
        }

        // Untextured pass: the ground terrain first (it sits under/behind
        // everything; depth sorts it), then the guardrails, then any model part with
        // no base-color texture (drawn on the Phong pipeline with its vertex color).
        renderer.set_pipeline(pipeline);
        renderer.draw_mesh(terrain, &terrain_transform, &MaterialParams::default());
        for transform in &guardrail_draws {
            renderer.draw_mesh(guardrail, transform, &MaterialParams::default());
        }
        for (entity_transform, model) in &model_draws {
            for (local, mesh, texture) in self.model_parts(*model) {
                if texture.is_none() {
                    renderer.draw_mesh(*mesh, &compose(*entity_transform, local), &MaterialParams::default());
                }
            }
        }
        renderer.submit();
    }
}

impl CarRunner {
    /// The parts of the model a placement draws: the player's car, a traffic-car
    /// variant, or a building prefab (each clamped to its loaded set).
    fn model_parts(&self, model: PlacedModel) -> &[CarPart] {
        match model {
            PlacedModel::Player => &self.player_car,
            PlacedModel::Traffic(i) => pick_model(&self.traffic_cars, i),
            PlacedModel::Building(i) => pick_model(&self.buildings, i),
        }
    }

    /// The world transform for the skyline backdrop billboard this frame (KE-0705):
    /// a wide, tall quad placed [`BACKDROP_DIST`] units ahead of the player along
    /// the travel axis and **locked to the player's XZ** (centered on `x = 0`) so a
    /// world stream/rebase never shifts it — it reads as a fixed far skyline. Sits
    /// at [`BACKDROP_DIST`] < the camera far plane (100) so it is not clipped.
    /// The world transform for the ground terrain this frame (KE-0706): the sheet is
    /// baked in player-relative `Z`, so translating it to the player's `Z` keeps it
    /// camera-locked — the ground always fills the view and a stream/rebase never
    /// slides it.
    fn terrain_transform(&self) -> Transform {
        Transform::from_position(Vec3::new(0.0, 0.0, self.player_position().z))
    }

    fn backdrop_transform(&self) -> Transform {
        let player = self.player_position();
        Transform {
            position: Vec3::new(0.0, BACKDROP_Y, player.z - BACKDROP_DIST),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(BACKDROP_W, BACKDROP_H, 1.0),
        }
    }
}

/// The committed **player** car model, imported at startup (KE-0703): the CC0
/// Quaternius Sports Car (`.glb`, public domain). Resolved against this crate's
/// dir so the path holds regardless of the process working directory.
const PLAYER_CAR_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sports_car.glb");

/// The committed **traffic** car models (CC0 Quaternius, public domain). Each
/// obstacle draws one of these, chosen per streaming slot by [`variant_for_slot`].
const TRAFFIC_CAR_ASSETS: [&str; 3] = [
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/car.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/car2.glb"),
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/police_car.glb"),
];

/// A SplitMix64 finalizer over `x`, used for all deterministic per-slot choices so
/// they are independent of the lane PRNG (adding variety never perturbs the
/// obstacle world / the smoke run).
fn hash_u64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Pick a traffic car variant (index into [`TRAFFIC_CAR_ASSETS`]) for a streaming
/// `slot`. Deterministic and independent of the lane PRNG. Returns `0` if there are
/// no traffic models.
fn variant_for_slot(slot: i64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (hash_u64(slot as u64) % n as u64) as usize
}

/// Pick a roadside building prefab (index into [`BUILDING_ASSETS`]) for a streaming
/// `slot` and `side` (0 = left, 1 = right), weighted by [`BUILDING_WEIGHTS`] so
/// skyscrapers are rare and mid/small buildings common. Deterministic and
/// independent of the lane PRNG.
fn building_for_slot(slot: i64, side: u64) -> usize {
    let total: u32 = BUILDING_WEIGHTS.iter().sum();
    if total == 0 {
        return 0;
    }
    let r = (hash_u64((slot as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (side + 1)) % total as u64)
        as u32;
    let mut acc = 0;
    for (i, &w) in BUILDING_WEIGHTS.iter().enumerate() {
        acc += w;
        if r < acc {
            return i;
        }
    }
    BUILDING_WEIGHTS.len() - 1
}

/// The parts of one of `models` by index, falling back to the first model when the
/// index is out of range (so a stale/oversized variant never panics).
fn pick_model(models: &[Vec<CarPart>], i: usize) -> &[CarPart] {
    models
        .get(i)
        .or_else(|| models.first())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Target car length in world units (the model is fit so its longer horizontal
/// extent matches this). Sized to sit within a lane with margin.
const CAR_LEN: f32 = 2.85;
/// World-space `Y` the fitted car's underside rests at, so it sits on the road
/// (the road surface top is at `y = -0.3`; the car entity is placed at
/// `PLAYER_Y = 0.1`, so the model bottom lands on the road at `0.1 + CAR_BOTTOM`).
const CAR_BOTTOM: f32 = -0.4;
/// Yaw (about `+Y`) applied when fitting the model so it faces the travel
/// direction (`-Z`). Tuned to the ToyCar model's authored orientation.
const CAR_YAW: f32 = std::f32::consts::PI;

/// The committed roadside **building** prefabs (CC0 Kenney City Kit, public
/// domain), imported at startup (KE-0706). Order matches [`BUILDING_WEIGHTS`].
const BUILDING_ASSETS: [&str; 8] = [
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
const BUILDING_WEIGHTS: [u32; 8] = [4, 4, 18, 18, 19, 12, 12, 13];

/// Target horizontal footprint (world units) each building is fit to; the model's
/// height scales with it, so tall prefabs stay tall and short ones short.
const BUILDING_FOOTPRINT: f32 = 8.0;
/// World `Y` of the ground plane the buildings' bases rest on — well below the road
/// surface (`≈ -0.3`), so the road reads as an elevated freeway above the city.
const GROUND_Y: f32 = -10.0;
/// Lateral offset (from center, `x = 0`) of the building row on each side of the
/// road. Beyond the road's outer edge (`±6`).
const BUILDING_SIDE_X: f32 = 11.0;
/// Per-slot lateral jitter added to [`BUILDING_SIDE_X`] so the rows aren't a flat
/// wall.
const BUILDING_SIDE_JITTER: f32 = 4.0;

/// Lateral position of the guardrail on each side of the road — just inside the
/// road's outer edge (`±6`), outboard of the drivable lanes (`±4.5`).
const GUARDRAIL_X: f32 = 5.7;
/// World `Y` the guardrail's base sits at — the top of the road deck (the road tile
/// is centered at `y = -0.5` with half-height `0.2`, so its surface is `y = -0.3`).
const GUARDRAIL_DECK_Y: f32 = -0.3;
/// Height of the guardrail (top rail) above its base, in world units.
const GUARDRAIL_H: f32 = 0.6;
/// Spacing between guardrail posts, in world units.
const GUARDRAIL_POST_SPACING: f32 = 2.0;

/// Total width (along `X`) of the ground terrain sheet.
const TERRAIN_W: f32 = 260.0;
/// How far behind / ahead of the player the terrain sheet extends (camera-locked,
/// relative `Z`). The far edge stays inside the camera's far plane (100) measured
/// from the trailing camera, so it never clips.
const TERRAIN_Z_NEAR: f32 = 30.0;
const TERRAIN_Z_FAR: f32 = -80.0;
/// Half-width of the flat valley floor the road and buildings sit on — the terrain
/// stays level at [`GROUND_Y`] out to here, so buildings rest flush, then rises.
const TERRAIN_FLAT_HALF: f32 = 13.0;
/// World `Y` the flanking hills crest at. Comfortably above the apparent horizon so
/// the hills fully eclipse the sky behind the buildings.
const HILL_CREST: f32 = 13.0;
/// Number of columns across the terrain sheet (smoothness of the hill profile).
const TERRAIN_COLUMNS: u32 = 96;

/// Ground height at lateral position `x`: a flat valley floor at [`GROUND_Y`] out to
/// [`TERRAIN_FLAT_HALF`] (so the road platform and the building rows sit on level
/// ground), then rising into rolling hills that crest near [`HILL_CREST`] at the
/// sheet's edges.
fn terrain_height(x: f32) -> f32 {
    let ax = x.abs();
    let span = (TERRAIN_W * 0.5 - TERRAIN_FLAT_HALF).max(f32::EPSILON);
    let t = ((ax - TERRAIN_FLAT_HALF).max(0.0) / span).clamp(0.0, 1.0);
    // Ease in so the ground leaves the valley floor gently, then climbs.
    let climb = t.powf(1.4) * (HILL_CREST - GROUND_Y);
    // Rolling variation, faded in with the climb so the valley floor stays level.
    let roll = 2.2 * ((x * 0.11).sin() * 0.6 + (x * 0.29 + 1.3).sin() * 0.4);
    GROUND_Y + climb + t * roll
}

/// Build the ground terrain sheet: one quad strip spanning
/// [`TERRAIN_Z_NEAR`]..[`TERRAIN_Z_FAR`] in relative `Z`, with each column's height
/// from [`terrain_height`]. Colored green, lighter toward the hilltops. Packed on
/// the `[pos,normal,color]` layout (untextured pipeline); drawn camera-locked.
fn terrain_geometry() -> (Vec<u8>, Vec<u32>) {
    let valley_color = [0.20, 0.28, 0.17];
    let hill_color = [0.36, 0.42, 0.33];

    let mut bytes: Vec<u8> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let push_vertex = |bytes: &mut Vec<u8>, pos: [f32; 3], color: [f32; 3]| {
        for f in pos {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in [0.0f32, 1.0, 0.0] {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        for f in color {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
    };

    for i in 0..=TERRAIN_COLUMNS {
        let u = i as f32 / TERRAIN_COLUMNS as f32;
        let x = (u - 0.5) * TERRAIN_W;
        let y = terrain_height(x);
        // Blend the color with how high this column climbed.
        let t = ((y - GROUND_Y) / (HILL_CREST - GROUND_Y)).clamp(0.0, 1.0);
        let color = [
            valley_color[0] + (hill_color[0] - valley_color[0]) * t,
            valley_color[1] + (hill_color[1] - valley_color[1]) * t,
            valley_color[2] + (hill_color[2] - valley_color[2]) * t,
        ];
        push_vertex(&mut bytes, [x, y, TERRAIN_Z_NEAR], color);
        push_vertex(&mut bytes, [x, y, TERRAIN_Z_FAR], color);
    }

    for i in 0..TERRAIN_COLUMNS {
        let n = 2 * i; // near, this column
        let f = n + 1; // far, this column
        let n2 = n + 2; // near, next column
        let f2 = n + 3; // far, next column
        indices.extend_from_slice(&[n, f, f2, n, f2, n2]);
    }

    (bytes, indices)
}

/// The committed asphalt road-tile glTF (KE-0704), imported at startup: a flat
/// textured quad carrying the tiling UVs plus an embedded asphalt base-color PNG.
const ROAD_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/road.gltf");

/// The committed city skyline backdrop glTF (KE-0705): a vertical billboard quad
/// with an embedded skyline base-color PNG cropped from a CC0 photo.
const SKYLINE_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/skyline.gltf");

/// How far ahead of the player (along the travel axis, `-Z`) the skyline backdrop
/// sits. Kept well under the camera far plane (100) so it never clips; far enough
/// that the distance fog blends it toward the horizon so it reads as a far skyline.
const BACKDROP_DIST: f32 = 45.0;
/// Backdrop billboard width in world units — wide enough to span the view frustum
/// at [`BACKDROP_DIST`].
const BACKDROP_W: f32 = 150.0;
/// Backdrop billboard height in world units. One full image height renders as 17
/// units; the extra height extends the frame **downward** (the generator's
/// `V_SPAN` matches, so the picture keeps its scale and position and the extra
/// frame just shows more of the image's bottom). Keep in sync with the
/// `FRAME_WORLD_H` constant in `examples/gen_skyline.rs`.
const BACKDROP_H: f32 = 24.0;
/// World `Y` of the backdrop's center. Chosen so the frame's **top edge stays at
/// `13.5`** while `BACKDROP_H` grows downward (`Y = 13.5 - H/2`), extending the
/// frame's bottom to cover the mid-ground without moving the skyline picture.
const BACKDROP_Y: f32 = 1.5;

/// Import a model through the asset cache and return its drawable parts: one
/// `(fitted transform, mesh handle, base-color texture)` per mesh-node. The file is
/// parsed + its meshes uploaded exactly once (KE-0103); each mesh's decoded
/// base-color image is uploaded here via `create_texture` (the cache uploads
/// meshes, not textures). `fit` computes the model→world fit transform from the
/// scene bounds ([`fit_transform`] for cars, [`building_fit`] for buildings); each
/// part's stored transform is `fit ∘ node.transform` so the render pass only
/// composes it with the entity's placement.
fn load_model(
    cache: &mut AssetCache,
    renderer: &mut dyn Renderer,
    path: &str,
    fit: fn(&kaman_assets::SceneAsset) -> Transform,
) -> Vec<CarPart> {
    let asset = cache
        .load(renderer, path)
        .unwrap_or_else(|e| panic!("import model asset {path}: {e}"));

    let fit = fit(&asset.scene);

    asset
        .scene
        .mesh_nodes()
        .map(|(_, node)| {
            let mesh_idx = node.mesh.expect("mesh_nodes yields only mesh-bearing nodes");
            let local = compose(fit, &node.transform);
            let handle = asset.mesh_handles[mesh_idx];
            // Upload this mesh's base-color image, if it has one, so the textured
            // pipeline can sample it. A mesh with no texture stays `None` and is
            // drawn on the untextured pipeline.
            let texture = asset.scene.meshes[mesh_idx].base_color.as_ref().map(|tex| {
                renderer.create_texture(&TextureData {
                    width: tex.width,
                    height: tex.height,
                    rgba8: &tex.rgba8,
                })
            });
            (local, handle, texture)
        })
        .collect()
}

/// Compute the transform that fits a building prefab: a uniform scale so its
/// horizontal footprint is [`BUILDING_FOOTPRINT`] (height scales with it, keeping
/// tall/short character), centered in `X`/`Z`, with its **underside at the model
/// origin** so the spawn transform drops the base onto the ground plane. No
/// rotation — buildings keep their authored upright orientation.
fn building_fit(scene: &kaman_assets::SceneAsset) -> Transform {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (_, node) in scene.mesh_nodes() {
        let mesh = &scene.meshes[node.mesh.expect("mesh node has a mesh")];
        for p in &mesh.positions {
            let w = node.transform.transform_point(Vec3::from_array(*p));
            min = min.min(w);
            max = max.max(w);
        }
    }

    let size = max - min;
    let footprint = size.x.max(size.z).max(f32::EPSILON);
    let scale = BUILDING_FOOTPRINT / footprint;

    // Pivot at the footprint center / underside → origin, so `base = 0`, centered.
    let pivot = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    Transform {
        position: -(pivot * scale),
        rotation: Quat::IDENTITY,
        scale: Vec3::splat(scale),
    }
}

/// Compute the transform that fits an imported car model to the road: a uniform
/// scale so its longer horizontal extent is [`CAR_LEN`], a [`CAR_YAW`] rotation so
/// it faces the travel direction, and a translation centering it in `X`/`Z` with
/// its underside at [`CAR_BOTTOM`]. Robust to any model's authored scale /
/// orientation, since it works from the baked world-space bounds of the geometry.
fn fit_transform(scene: &kaman_assets::SceneAsset) -> Transform {
    // World-space AABB over every mesh-node's baked vertices.
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (_, node) in scene.mesh_nodes() {
        let mesh = &scene.meshes[node.mesh.expect("mesh node has a mesh")];
        for p in &mesh.positions {
            let w = node.transform.transform_point(Vec3::from_array(*p));
            min = min.min(w);
            max = max.max(w);
        }
    }

    let size = max - min;
    let horizontal = size.x.max(size.z).max(f32::EPSILON);
    let scale = CAR_LEN / horizontal;
    let rotation = Quat::from_rotation_y(CAR_YAW);

    // Pivot: footprint center in X/Z, underside in Y. Scaling + rotating about the
    // pivot keeps the car centered and level; then lift the underside to CAR_BOTTOM.
    let pivot = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    let translation = -(rotation * (pivot * scale)) + Vec3::new(0.0, CAR_BOTTOM, 0.0);

    Transform {
        position: translation,
        rotation,
        scale: Vec3::splat(scale),
    }
}

/// Import a **single-mesh textured** glTF (a unit quad whose material carries an
/// embedded base-color PNG — the asphalt road tile, KE-0704, or the skyline
/// billboard, KE-0705) and return `(mesh handle, base-color texture handle)`.
///
/// The importer packs the quad on the `[pos,normal,uv]` textured layout and
/// `AssetCache::load` uploads it as a textured mesh (one handle); the decoded
/// base-color RGBA8 is handed straight to `create_texture`. Parsed + uploaded once
/// (KE-0103); the mesh's node transform is identity (the quad is a unit tile
/// placed/scaled by the caller's transform), so only its handle is needed.
fn load_textured_mesh(
    cache: &mut AssetCache,
    renderer: &mut dyn Renderer,
    path: &str,
) -> (MeshHandle, TextureHandle) {
    let asset = cache
        .load(renderer, path)
        .unwrap_or_else(|e| panic!("import textured asset {path}: {e}"));

    // A single-mesh glTF; grab its uploaded handle and its base-color image.
    let mesh = *asset
        .mesh_handles
        .first()
        .expect("textured asset has one uploaded mesh");
    let base_color = asset.scene.meshes[0]
        .base_color
        .as_ref()
        .expect("textured mesh carries a base-color texture");
    let texture = renderer.create_texture(&TextureData {
        width: base_color.width,
        height: base_color.height,
        rgba8: &base_color.rgba8,
    });
    (mesh, texture)
}

/// Compose a `parent` world transform with a `child` (local) transform, so an
/// imported mesh part draws at its entity's placement times its baked node
/// transform. Done via matrices so scale/rotation/translation all combine
/// correctly (the demo's car nodes are authored at the origin, but this stays
/// correct for any baked hierarchy).
fn compose(parent: Transform, child: &Transform) -> Transform {
    let m = parent.to_matrix() * child.to_matrix();
    let (scale, rotation, position) = m.to_scale_rotation_translation();
    Transform {
        position,
        rotation,
        scale,
    }
}

/// The untextured `[pos,normal,color]` layout (36-byte stride) every mesh uses —
/// the same layout the untextured Phong pipeline binds.
fn color_layout() -> VertexLayout {
    kaman_assets::render_vertex_layout()
}

/// Append an axis-aligned box (6 outward-facing quads) at `center` with the given
/// `half`-extents and a flat `color`, packed onto the `[pos,normal,color]` layout.
fn push_box(bytes: &mut Vec<u8>, indices: &mut Vec<u32>, center: [f32; 3], half: [f32; 3], color: [f32; 3]) {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, 1.0, 1.0], [-1.0, 1.0, 1.0]]),
        ([0.0, 0.0, -1.0], [[1.0, -1.0, -1.0], [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0]]),
        ([0.0, 1.0, 0.0], [[-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, -1.0], [-1.0, 1.0, -1.0]]),
        ([0.0, -1.0, 0.0], [[-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0]]),
        ([1.0, 0.0, 0.0], [[1.0, -1.0, 1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0]]),
        ([-1.0, 0.0, 0.0], [[-1.0, -1.0, -1.0], [-1.0, -1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0]]),
    ];
    for (n, corners) in faces {
        let base = (bytes.len() / 36) as u32;
        for c in corners {
            let pos = [center[0] + c[0] * half[0], center[1] + c[1] * half[1], center[2] + c[2] * half[2]];
            for f in pos {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in n {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in color {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Build one guardrail segment `length` units long (along `Z`), local origin at its
/// base center so a spawn transform drops it onto the road deck: two horizontal
/// metal rails plus evenly-spaced vertical posts. Packed on the `[pos,normal,color]`
/// layout (untextured pipeline).
fn guardrail_geometry(length: f32) -> (Vec<u8>, Vec<u32>) {
    let rail_color = [0.62, 0.63, 0.66];
    let post_color = [0.40, 0.41, 0.44];
    let half_len = length / 2.0;

    let mut bytes = Vec::new();
    let mut indices = Vec::new();

    // Two horizontal rails running the length of the segment (thin in X, at the
    // road edge; the segment is placed at ±GUARDRAIL_X).
    push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H, 0.0], [0.05, 0.07, half_len], rail_color);
    push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H * 0.6, 0.0], [0.05, 0.055, half_len], rail_color);

    // Vertical posts, evenly spaced along the segment (endpoints included).
    let posts = ((length / GUARDRAIL_POST_SPACING).round() as i32).max(1);
    for i in 0..=posts {
        let z = -half_len + (i as f32 / posts as f32) * length;
        push_box(&mut bytes, &mut indices, [0.0, GUARDRAIL_H * 0.5, z], [0.06, GUARDRAIL_H * 0.5, 0.06], post_color);
    }

    (bytes, indices)
}

/// The textured `[pos,normal,uv]` layout (32-byte stride) the road tiles use — the
/// same layout the textured pipeline binds and the importer packs the road quad on.
fn textured_layout() -> VertexLayout {
    kaman_assets::textured_vertex_layout()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_runs_without_panicking() {
        // The oracle must complete for the fixed frame count without requiring a GPU.
        run_smoke(SMOKE_FRAMES);
    }

    #[test]
    fn smoke_prints_the_expected_contract_line() {
        // Guards the exact `--smoke` stdout contract (`smoke: 120 frames OK`).
        assert_eq!(smoke_report(SMOKE_FRAMES), "smoke: 120 frames OK");
    }

    #[test]
    fn init_spawns_player_and_streams_road() {
        let mut game = CarRunner::new();
        let harness = kaman_core::headless::run(&mut game, 0);
        assert!(game.player.is_some(), "player spawned in init");
        // Streaming primed some road/obstacles ahead.
        assert!(
            harness.scene().streamed_count() > 0,
            "init primed streamed content ahead of the player"
        );
    }

    #[test]
    fn lane_x_is_centered_and_ordered() {
        // Three lanes centered on 0: -W, 0, +W.
        assert!((CarRunner::lane_x(0) + CarRunner::LANE_WIDTH).abs() < 1e-6);
        assert!(CarRunner::lane_x(1).abs() < 1e-6);
        assert!((CarRunner::lane_x(2) - CarRunner::LANE_WIDTH).abs() < 1e-6);
    }

    #[test]
    fn lane_input_clamps_at_both_edges() {
        use kaman_core::headless::Headless;

        // Tap a key (press, step, release, step) so each tap is one edge.
        let tap = |h: &mut Headless, game: &mut CarRunner, key: Key| {
            h.input_mut().press_key(key);
            h.run(game, 1);
            h.input_mut().release_key(key);
            h.run(game, 1);
        };

        // Tap Left three times from the center (lane 1): reach lane 0 and clamp.
        let mut left_game = CarRunner::new();
        let mut hl = Headless::new();
        for _ in 0..3 {
            tap(&mut hl, &mut left_game, Key::Left);
        }
        assert_eq!(left_game.lane, 0, "one lane per press, clamped at the left edge");

        // Tap Right three times from the center: reach the last lane and clamp.
        let mut right_game = CarRunner::new();
        let mut hr = Headless::new();
        for _ in 0..3 {
            tap(&mut hr, &mut right_game, Key::Right);
        }
        assert_eq!(right_game.lane, CarRunner::LANES - 1, "clamped at the right edge");
    }

    #[test]
    fn a_held_key_moves_exactly_one_lane() {
        // Edge-triggered control: holding a key advances exactly one lane, not a
        // glide across lanes (which is what level-triggered input would do).
        let mut game = CarRunner::new();
        game.lane = 0; // start at the left edge
        let mut h = kaman_core::headless::Headless::new();
        h.input_mut().press_key(Key::Right);
        h.run(&mut game, 20); // hold Right for many frames
        assert_eq!(
            game.lane, 1,
            "a held key advances one lane (edge-triggered), not a glide to the far lane"
        );
    }

    #[test]
    fn score_increases_with_distance() {
        let mut game = CarRunner::new();
        let mut h = kaman_core::headless::Headless::new();
        h.run(&mut game, 1);
        let after_one = game.distance;
        assert!(after_one > 0.0, "distance advanced after one step");
        h.run(&mut game, 60);
        assert!(game.distance > after_one, "distance keeps increasing");
        // Score tracks distance monotonically.
        assert!(game.score() >= after_one as u64);
    }

    #[test]
    fn obstacle_placement_is_deterministic_from_seed() {
        // Two independent runs of the same seed produce the identical lane sequence.
        let lanes = |n: usize| {
            let mut rng = Rng::new(CarRunner::SEED);
            (0..n)
                .map(|_| rng.next_below(CarRunner::LANES as u32))
                .collect::<Vec<_>>()
        };
        assert_eq!(lanes(50), lanes(50), "same seed ⇒ same obstacle lanes");

        // And the full game streams the same obstacle world twice for the same seed.
        let obstacle_zs = || {
            let mut game = CarRunner::new();
            let harness = kaman_core::headless::run(&mut game, 30);
            let mut zs: Vec<i64> = harness
                .scene()
                .world()
                .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
                .iter()
                .map(|(_e, (t, _))| (t.transform.position.z * 100.0) as i64)
                .collect();
            zs.sort_unstable();
            zs
        };
        assert_eq!(obstacle_zs(), obstacle_zs(), "deterministic obstacle stream");
    }

    #[test]
    fn overlap_detects_same_lane_hit_only() {
        let mut game = CarRunner::new();
        game.lane = 1;
        game.travel = 10.0;
        let player = game.player_position();
        // Same lane, same Z ⇒ hit.
        assert!(game.overlaps(Vec3::new(player.x, player.y, player.z)));
        // Different lane, same Z ⇒ no hit.
        assert!(!game.overlaps(Vec3::new(CarRunner::lane_x(0), player.y, player.z)));
        // Same lane, far ahead ⇒ no hit.
        assert!(!game.overlaps(Vec3::new(player.x, player.y, player.z - 5.0)));
    }

    /// Drive the center runner (no input) until the deterministic stream puts an
    /// obstacle in its path. Returns the game at the moment it enters `GameOver`.
    fn drive_to_game_over() -> (CarRunner, kaman_core::headless::Headless) {
        let mut game = CarRunner::new();
        let mut h = kaman_core::headless::Headless::new();
        for _ in 0..600 {
            h.run(&mut game, 1);
            if game.state == GameState::GameOver {
                return (game, h);
            }
        }
        panic!("the center runner never crashed in 600 frames");
    }

    #[test]
    fn collision_ends_the_run_at_game_over() {
        // A crash enters GameOver; the score is NOT reset (only a replay resets it).
        let (game, _h) = drive_to_game_over();
        assert_eq!(game.state, GameState::GameOver);
        assert!(game.distance > 0.0, "distance is preserved at game over (score to report)");
    }

    #[test]
    fn replay_from_game_over_resets_score_keeps_world() {
        // From GameOver, pressing Space starts a fresh run: score zeroed, lane
        // recentered — but the monotonic world position keeps advancing so the road
        // never breaks.
        let (mut game, mut h) = drive_to_game_over();
        let travel_before = game.travel;
        let score_at_crash = game.distance;
        assert!(score_at_crash > 0.0);

        // Tap Space (edge-triggered) to replay.
        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);

        assert_eq!(game.state, GameState::Playing, "replay resumes play");
        assert!(game.distance < score_at_crash, "score reset on replay");
        assert_eq!(game.lane, CarRunner::START_LANE, "lane recentered");
        assert!(game.travel >= travel_before, "world travel stays monotonic");
    }

    #[test]
    fn streaming_stays_bounded_over_a_long_run() {
        // Endless streaming must not leak: entity/body counts stay flat across a
        // long headless run (also exercises a floating-origin rebase or two).
        let mut game = CarRunner::new();
        let harness = kaman_core::headless::run(&mut game, 5000);
        let entities = harness.scene().world().len();
        // A generous bound: the streaming window holds only a handful of slots.
        assert!(entities < 200, "entity count stayed bounded: {entities}");
    }
}
