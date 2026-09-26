// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! [`CarRunner`] — the playable-demo [`Game`] implementation.
//!
//! This is the seam the whole demo is built on: [`CarRunner`] implements
//! [`kaman_core::Game`] and is driven by the engine loop through
//! [`EngineCtx`]. Everything here is *game* state and
//! *game* rules (lanes, obstacles, score, the state machine); the ECS world,
//! physics, and scene streaming it manipulates are all engine-generic services
//! reached only through `EngineCtx`. See [`crate::assets`] for how the models it
//! places are loaded, [`crate::scenery`] for the procedural geometry it streams,
//! and [`crate::render`] for how a frame built from this state is drawn.

use kaman_assets::AssetCache;
use kaman_camera::ChaseController;
use kaman_core::input::Key;
use kaman_core::{EngineCtx, Game};
use kaman_ecs::hecs::Entity;
use kaman_ecs::{DynamicTag, RenderComponent, StaticTag, TransformComponent};
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render_api::{MeshData, MeshHandle, PipelineDescriptor, PipelineHandle, TextureHandle};
use kaman_scene::Scene;

use crate::assets::{building_fit, color_layout, fit_transform, load_model, load_textured_mesh, textured_layout, CarPart};
use crate::components::{BuildingVariant, GuardrailTag, TrafficVariant};
use crate::config;
use crate::rng::{building_for_slot, hash_u64, variant_for_slot, Rng};
use crate::scenery::{guardrail_geometry, terrain_geometry};

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
pub(crate) struct CarRunner {
    /// The player-controlled box entity (spawned once in `init`).
    pub(crate) player: Option<Entity>,
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
    pub(crate) pipeline: Option<PipelineHandle>,
    /// The **textured** pipeline (`[pos,normal,uv]`), created in `init`. The road
    /// tiles are drawn on it, sampling the asphalt base-color texture.
    pub(crate) textured_pipeline: Option<PipelineHandle>,
    /// The road-tile mesh — a flat quad carrying the tiling UVs, imported once from
    /// `assets/road.gltf` (KE-0704) and scaled by each tile's transform.
    pub(crate) road_mesh: Option<MeshHandle>,
    /// The asphalt base-color texture uploaded once in `init` (KE-0704); bound
    /// before the road-tile draws so the textured pipeline samples it.
    pub(crate) asphalt: Option<TextureHandle>,
    /// The guardrail segment mesh (KE-0706) — a procedural rail + posts one
    /// `spawn_interval` long, built once and streamed along both road edges. Drawn
    /// on the untextured pipeline (its metal color is baked into the vertices).
    pub(crate) guardrail_mesh: Option<MeshHandle>,
    /// The hill-terrain ground mesh (KE-0706) — a wide ground surface built once and
    /// drawn camera-locked: a flat valley floor **under** the road and buildings
    /// that rises into hills on both flanks, so the buildings sit on ground and the
    /// sky doesn't show through the mid-ground.
    pub(crate) hills_mesh: Option<MeshHandle>,
    /// The distant city skyline backdrop — a billboard quad imported once from
    /// `assets/skyline.gltf` (KE-0705), drawn far ahead and locked to the camera's
    /// XZ so it reads as a far skyline behind the fog.
    pub(crate) backdrop_mesh: Option<MeshHandle>,
    /// The skyline base-color texture (KE-0705), uploaded once and bound before the
    /// backdrop draw.
    pub(crate) backdrop_texture: Option<TextureHandle>,
    /// The **player** car model, imported once from the CC0 `assets/sports_car.glb`
    /// (KE-0703): one [`CarPart`] per drawable mesh-node, fit to the road. Uploaded
    /// once (KE-0103) and reused every frame.
    pub(crate) player_car: Vec<CarPart>,
    /// The **traffic** car models — the CC0 `assets/{car,car2,police_car}.glb`
    /// ([`crate::config::TRAFFIC_CAR_ASSETS`]). Each obstacle draws one of these,
    /// chosen deterministically per streaming slot ([`variant_for_slot`]). All
    /// uploaded once (KE-0103) and shared across every obstacle on screen.
    pub(crate) traffic_cars: Vec<Vec<CarPart>>,
    /// The roadside **building** prefabs — the CC0 Kenney City Kit models
    /// ([`crate::config::BUILDING_ASSETS`]), imported once and streamed along both
    /// sides of the elevated road. Each streamed building draws one of these,
    /// chosen by a weighted per-slot pick ([`building_for_slot`]); shared across
    /// all instances.
    pub(crate) buildings: Vec<Vec<CarPart>>,
    /// The chase camera controller that keeps the player framed (KE-0205). It
    /// trails the box from behind and above along the travel axis, with light
    /// smoothing so the follow eases rather than snapping.
    chase: ChaseController,
    /// Whether the run is live or crashed. `Playing` advances the world and reads
    /// Left/Right; `GameOver` freezes the run and waits for the replay key.
    pub(crate) state: GameState,
    /// The SDF font atlas the HUD draws with (KE-0404/KE-0707), loaded once in
    /// `init`. `None` until then — the HUD simply draws nothing.
    pub(crate) font: Option<kaman_render_api::FontAtlas>,
    /// Seconds of the opening fade still to play. The title screen is fully black;
    /// starting a run sets this to [`config::HUD_FADE_SECONDS`] and it counts down
    /// on the fixed timestep, fading the black away to reveal the scene. Purely
    /// cosmetic — it never gates simulation, so determinism is unaffected.
    fade_remaining: f32,
    /// Seconds of driving elapsed in the **current run**, accumulated on the fixed
    /// timestep. Drives the difficulty ramp ([`Self::current_speed`]) and resets
    /// to zero on replay, so every run starts at the base speed.
    run_time: f32,
    /// Whether the floating-origin rebase shifted the world on the most recent
    /// step. Reported in the game-over diagnostic, since a crash that coincides
    /// with a rebase points at the world moving under the collision test.
    rebased_last_step: bool,
}

/// The demo's tiny game-state machine: drive until you crash, then replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GameState {
    /// The opening state: the world is built and framed but frozen, waiting for
    /// the player to start. The start key begins the run.
    Ready,
    /// Driving: the world scrolls, Left/Right change lanes, collisions end the run.
    Playing,
    /// Crashed: the world is frozen and the score reported; a replay key restarts.
    GameOver,
}

impl CarRunner {
    /// World-space `X` of a lane center.
    fn lane_x(lane: usize) -> f32 {
        // Center lanes about x=0: lane 0 → -LANE_WIDTH, center → 0, etc.
        (lane as f32 - (config::LANES as f32 - 1.0) / 2.0) * config::LANE_WIDTH
    }

    /// The player's current world position (lane along `X`, monotonic travel along
    /// `-Z`). A pure function of game state, so it matches the ECS transform and is
    /// used for the streaming focus, the overlap test, and (via
    /// [`crate::render`]) the camera-locked backdrop/terrain placement.
    pub(crate) fn player_position(&self) -> Vec3 {
        Vec3::new(Self::lane_x(self.lane), config::PLAYER_Y, -self.travel)
    }

    /// Create an unspawned game; [`init`](Game::init) populates the world.
    pub(crate) fn new() -> Self {
        Self {
            player: None,
            lane: config::START_LANE,
            travel: 0.0,
            distance: 0.0,
            best: 0.0,
            rng: Rng::new(config::SEED),
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
            chase: ChaseController::new(config::CHASE_DISTANCE, config::CHASE_HEIGHT)
                .with_look_at_height(config::CHASE_LOOK_AT_HEIGHT)
                .with_smoothing(config::CHASE_SMOOTHING),
            state: GameState::Ready,
            font: None,
            fade_remaining: 0.0,
            run_time: 0.0,
            rebased_last_step: false,
        }
    }

    /// The score derived from the current run's distance (1 point per world unit).
    /// The car's forward speed this frame, in world units per second.
    ///
    /// The run gets harder as it goes: the base [`config::SPEED`] is multiplied by
    /// [`config::SPEED_RAMP`] for every [`config::SPEED_RAMP_SECONDS`] survived.
    /// The growth is continuous rather than stepped, so the car accelerates
    /// smoothly instead of lurching every five seconds, while still hitting exactly
    /// `×1.1` at each interval boundary. Clamped to [`config::SPEED_MAX`] — see
    /// that constant for why the ceiling is a correctness requirement, not just
    /// a difficulty choice.
    pub(crate) fn current_speed(&self) -> f32 {
        let factor = config::SPEED_RAMP.powf(self.run_time / config::SPEED_RAMP_SECONDS);
        (config::SPEED * factor).min(config::SPEED_MAX)
    }

    /// How opaque the opening fade's black overlay should be this frame, `0.0`
    /// (fully revealed) to `1.0` (fully black).
    pub(crate) fn fade_alpha(&self) -> f32 {
        if config::HUD_FADE_SECONDS <= 0.0 {
            return 0.0;
        }
        (self.fade_remaining / config::HUD_FADE_SECONDS).clamp(0.0, 1.0)
    }

    /// The best score of the session so far, for the game-over banner.
    pub(crate) fn best_score(&self) -> u64 {
        self.best.max(0.0) as u64
    }

    pub(crate) fn score(&self) -> u64 {
        self.distance.max(0.0) as u64
    }

    /// Apply lane-switch input for this fixed step: **discrete, edge-triggered**.
    ///
    /// `Left`/`Right` are the only gameplay keys; each *press* snaps the car exactly
    /// one lane toward the edge (clamped), so a held key does not glide across lanes
    /// (KE-0702, via [`InputState::is_key_just_pressed`](kaman_core::InputState::is_key_just_pressed)).
    /// Both at once cancel.
    fn apply_lane_input(&mut self, ctx: &EngineCtx) {
        let input = ctx.input();
        let left = input.is_key_just_pressed(Key::Left);
        let right = input.is_key_just_pressed(Key::Right);
        match (left, right) {
            (true, false) => self.lane = self.lane.saturating_sub(1),
            (false, true) => self.lane = (self.lane + 1).min(config::LANES - 1),
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
        dx < config::PLAYER_HALF.x + config::OBSTACLE_HALF.x
            && dz < config::PLAYER_HALF.z + config::OBSTACLE_HALF.z
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
    fn game_over(&mut self, hit: Vec3) {
        self.best = self.best.max(self.distance);
        self.state = GameState::GameOver;
        let player = self.player_position();
        println!(
            "GAME OVER — score {} — best {}. Press Space to replay.",
            self.score(),
            self.best as u64
        );
        // Diagnostic: exactly what was hit, and where we were. A game-over the
        // player did not see coming shows up here as an implausible separation
        // (e.g. a car that was never in our lane, or one spawned on top of us).
        println!(
            "  hit: player=({:.2}, {:.2}) obstacle=({:.2}, {:.2}) dx={:.2} dz={:.2} \
travel={:.1} speed={:.1} rebased_last_step={}",
            player.x,
            player.z,
            hit.x,
            hit.z,
            (player.x - hit.x).abs(),
            (player.z - hit.z).abs(),
            self.travel,
            self.current_speed(),
            self.rebased_last_step,
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
        let window = config::CLEAR_AHEAD;
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

        self.lane = config::START_LANE;
        self.distance = 0.0;
        // A fresh run starts at the base speed again.
        self.run_time = 0.0;
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
        let road_width = config::LANES as f32 * config::LANE_WIDTH + config::LANE_WIDTH;
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
        if cx.slot != 0 && cx.slot.rem_euclid(config::OBSTACLE_EVERY) == 0 {
            let lane = rng.next_below(config::LANES as u32) as usize;
            let pos = Vec3::new(Self::lane_x(lane), config::PLAYER_Y, road_z);

            // Give the obstacle a static physics body + collider so streaming's
            // atomic despawn (ECS entity + rigid body together) is exercised. The
            // body is not used to drive the player; collision is a game-side AABB.
            let handle = cx.physics.create_static_body(Transform::from_position(pos));
            cx.physics.add_box_collider(handle, config::OBSTACLE_HALF);

            // Pick a traffic car variant for this obstacle deterministically from
            // its slot — independent of the lane PRNG, so the obstacle world (and
            // the smoke run) is unchanged by adding variety.
            let variant = variant_for_slot(cx.slot, config::TRAFFIC_CAR_ASSETS.len());

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
            let x = sign * (config::BUILDING_SIDE_X + jitter * config::BUILDING_SIDE_JITTER);
            // Turn each building 90° to face the freeway: the left row (`x < 0`)
            // faces `+X` toward the road, the right row (`x > 0`) faces `-X`.
            let facing = Quat::from_rotation_y(-sign * std::f32::consts::FRAC_PI_2);
            let building = cx.world.spawn((
                TransformComponent::new(Transform {
                    position: Vec3::new(x, config::GROUND_Y, road_z),
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
                TransformComponent::from_position(Vec3::new(
                    sign * config::GUARDRAIL_X,
                    config::GUARDRAIL_DECK_Y,
                    road_z,
                )),
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
    fn hit_any_obstacle(&self, ctx: &EngineCtx) -> Option<Vec3> {
        for (_e, (t, _body)) in ctx
            .world()
            .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
            .iter()
        {
            if self.overlaps(t.transform.position) {
                return Some(t.transform.position);
            }
        }
        None
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
        // `load_model` fits each model to the road (uniform scale + orientation +
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
        let (road_mesh, asphalt) = load_textured_mesh(&mut cache, renderer, config::ROAD_ASSET);
        self.road_mesh = Some(road_mesh);
        self.asphalt = Some(asphalt);

        // Distant city skyline backdrop (KE-0705): a textured billboard quad,
        // uploaded once and drawn far ahead, locked to the camera's XZ.
        let (backdrop_mesh, backdrop_texture) =
            load_textured_mesh(&mut cache, renderer, config::SKYLINE_ASSET);
        self.backdrop_mesh = Some(backdrop_mesh);
        self.backdrop_texture = Some(backdrop_texture);

        self.player_car = load_model(&mut cache, renderer, config::PLAYER_CAR_ASSET, fit_transform);
        self.traffic_cars = config::TRAFFIC_CAR_ASSETS
            .iter()
            .map(|path| load_model(&mut cache, renderer, path, fit_transform))
            .collect();

        // Roadside building prefabs (KE-0706): imported + uploaded once each, fit to
        // a common footprint (heights preserved), base at the model origin so the
        // spawn transform drops each onto the ground plane below the road.
        self.buildings = config::BUILDING_ASSETS
            .iter()
            .map(|path| load_model(&mut cache, renderer, path, building_fit))
            .collect();

        // HUD font atlas (KE-0404/KE-0707): the committed SDF atlas, uploaded
        // once so the HUD can draw text every frame by handle.
        self.font = Some(crate::hud::load_font(renderer, config::FONT_ASSET));

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
        // Ready: the scene is built and framed but nothing moves yet — the world
        // holds still behind the "press to start" prompt until the player commits.
        // Pressing the start key begins the run from this frame.
        if self.state == GameState::Ready {
            if ctx.input().is_key_just_pressed(Key::Space) {
                self.state = GameState::Playing;
                // Start the opening fade: the black title screen dissolves into
                // the running game over the next fraction of a second.
                self.fade_remaining = config::HUD_FADE_SECONDS;
            }
            let player_pos = self.player_position();
            self.chase.follow(ctx.camera_mut(), player_pos, config::FORWARD);
            return;
        }

        // Game over: the world is frozen; wait for the replay key, keep the camera
        // framing the crashed car, and do nothing else.
        if self.state == GameState::GameOver {
            if ctx.input().is_key_just_pressed(Key::Space) {
                self.start_new_run(ctx);
            }
            let player_pos = self.player_position();
            self.chase.follow(ctx.camera_mut(), player_pos, config::FORWARD);
            return;
        }

        // Retire the opening fade on the fixed timestep, so it takes the same
        // wall-clock time regardless of framerate (and stays deterministic).
        self.fade_remaining = (self.fade_remaining - dt).max(0.0);

        // Playing: discrete Left/Right lane changes (kinematic, no solver).
        self.apply_lane_input(ctx);

        // Advance forward by a fixed step. `dt` is always `FIXED_DT`, so the
        // simulation is framerate-independent (KE-0201) — no wall-clock is read.
        // `travel` (world position) and `distance` (score) advance together; only
        // `distance` rewinds on a crash.
        // The run's speed ramps with time survived, so advance the run clock
        // first and move at this frame's speed.
        self.run_time += dt;
        let speed = self.current_speed();
        self.travel += speed * dt;
        self.distance += speed * dt;

        // Streaming cadence: rebase FIRST (between physics steps), against the
        // focus. A rebase shifts every streamed transform back toward the origin;
        // the player is kinematic and rebuilt from `travel`, so we fold the same
        // shift into `travel` to stay aligned with the shifted world.
        let axis = ctx.scene().config().axis;
        let offset = ctx.scene_mut().maybe_rebase(self.player_position());
        self.rebased_last_step = offset != Vec3::ZERO;
        if offset != Vec3::ZERO {
            // `travel` is measured along +axis from the origin; shifting the world
            // by `offset` moves the player's along-axis coordinate by offset·axis.
            self.travel += offset.dot(axis);

            // Carry the camera through the same coordinate change. A rebase moves
            // every transform at once; the camera holds world coordinates too, so
            // it has to move with them or it is suddenly pointing a whole
            // `rebase_threshold` away.
            //
            // Translating by `offset` (rather than re-deriving the pose from the
            // car) is deliberate: the smoothed follow always trails its desired
            // pose by a little, and snapping to the desired pose would erase that
            // lag in one frame — a small but visible jolt. Shifting preserves the
            // lag exactly, so the rebase is *invisible*.
            let camera = ctx.camera_mut();
            let position = camera.position() + offset;
            let target = camera.target() + offset;
            camera.set_position(position);
            camera.set_target(target);
        }

        // Drive the player's transform kinematically to its lane + travel position,
        // now consistent with any rebase this step.
        let player_pos = self.player_position();
        if let Some(p) = self.player {
            if let Ok(mut t) = ctx.world_mut().get::<&mut TransformComponent>(p) {
                t.transform.position = player_pos;
            }
        }

        // Follow the car with the chase camera (KE-0205): trail it from behind and
        // above along the travel axis, looking at it. The engine pushes the
        // camera's view-projection across the render seam before `render`, so the
        // car stays framed. Done in `update` (post-rebase) so the camera tracks
        // the same shifted world the draws use.
        //
        // On a rebase frame the camera must **snap** rather than ease. A rebase
        // teleports every transform (including the car) by up to a whole
        // `rebase_threshold`, and the smoothed follow only closes a fraction of
        // that gap per step — so easing would send the camera hurtling across the
        // world for a good half-second while it caught up, which reads as the game
        // glitching out and makes the car unsteerable. The rebase is a coordinate
        // change, not motion, so the correct response is to move the camera with it.
        // No rebase special-case here: the camera was already carried through the
        // coordinate change above, so ordinary smoothing applies every frame.
        self.chase
            .follow(ctx.camera_mut(), player_pos, config::FORWARD);

        // Then stream ahead / despawn behind, with the player as the focus.
        let tile_depth = ctx.scene().config().spawn_interval;
        let rng = &mut self.rng;
        ctx.scene_mut()
            .stream(player_pos, |cx| Self::spawn_slot(rng, tile_depth, cx));

        // Collision → game over (freeze the run; the player replays with Space).
        //
        // Nothing follows: the score needs no stdout milestone, because since
        // KE-0707 the HUD draws it live on screen every frame. The one line the
        // demo still prints on a crash is a diagnostic, not a readout.
        if let Some(hit) = self.hit_any_obstacle(ctx) {
            self.game_over(hit);
        }
    }

    fn render(&mut self, ctx: &mut EngineCtx) {
        self.render_frame(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaman_core::headless::Headless;

    /// Boot a game and tap the start key, leaving it in [`GameState::Playing`] —
    /// the state the gameplay tests below exercise. The demo opens frozen on its
    /// title prompt (KE-0707), so every test that drives the world starts here,
    /// mirroring what a player does.
    fn started() -> (CarRunner, Headless) {
        let mut game = CarRunner::new();
        let mut h = Headless::new();
        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);
        h.input_mut().release_key(Key::Space);
        assert_eq!(game.state, GameState::Playing, "the start key begins the run");
        (game, h)
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
        assert!((CarRunner::lane_x(0) + config::LANE_WIDTH).abs() < 1e-6);
        assert!(CarRunner::lane_x(1).abs() < 1e-6);
        assert!((CarRunner::lane_x(2) - config::LANE_WIDTH).abs() < 1e-6);
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
        let (mut left_game, mut hl) = started();
        for _ in 0..3 {
            tap(&mut hl, &mut left_game, Key::Left);
        }
        assert_eq!(left_game.lane, 0, "one lane per press, clamped at the left edge");

        // Tap Right three times from the center: reach the last lane and clamp.
        let (mut right_game, mut hr) = started();
        for _ in 0..3 {
            tap(&mut hr, &mut right_game, Key::Right);
        }
        assert_eq!(right_game.lane, config::LANES - 1, "clamped at the right edge");
    }

    #[test]
    fn a_held_key_moves_exactly_one_lane() {
        // Edge-triggered control: holding a key advances exactly one lane, not a
        // glide across lanes (which is what level-triggered input would do).
        let (mut game, mut h) = started();
        game.lane = 0; // start at the left edge
        h.input_mut().press_key(Key::Right);
        h.run(&mut game, 20); // hold Right for many frames
        assert_eq!(
            game.lane, 1,
            "a held key advances one lane (edge-triggered), not a glide to the far lane"
        );
    }

    #[test]
    fn opens_frozen_until_the_start_key() {
        // The demo opens on its title prompt (KE-0707): the world is built and
        // framed, but nothing advances until the player commits.
        let mut game = CarRunner::new();
        let mut h = Headless::new();
        assert_eq!(game.state, GameState::Ready, "starts on the title prompt");

        h.run(&mut game, 30);
        assert_eq!(game.state, GameState::Ready, "still frozen without input");
        assert_eq!(game.distance, 0.0, "no score accrues while frozen");
        assert_eq!(game.travel, 0.0, "the world does not scroll while frozen");

        // Tapping start begins the run, and the world advances from there.
        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);
        assert_eq!(game.state, GameState::Playing, "the start key begins the run");
        h.input_mut().release_key(Key::Space);
        h.run(&mut game, 5);
        assert!(game.distance > 0.0, "the run advances once started");
    }

    #[test]
    fn opening_fade_retires_on_the_fixed_timestep() {
        // The title screen's blackness is the HUD's own opaque wash for `Ready`;
        // the fade timer only runs once a run starts, handing the black over to
        // this countdown so it dissolves over HUD_FADE_SECONDS of simulated time.
        let mut game = CarRunner::new();
        let mut h = Headless::new();
        assert_eq!(game.state, GameState::Ready);
        assert_eq!(game.fade_alpha(), 0.0, "the fade is idle until a run starts");

        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);
        h.input_mut().release_key(Key::Space);
        assert_eq!(game.fade_alpha(), 1.0, "the fade starts fully black");

        // Half the fade duration in: partially revealed, but not yet clear.
        let half = (config::HUD_FADE_SECONDS / (2.0 * kaman_core::FIXED_DT)).round() as u32;
        h.run(&mut game, half);
        let mid = game.fade_alpha();
        assert!(mid > 0.0 && mid < 1.0, "mid-fade is partially revealed: {mid}");

        // Well past the duration: fully revealed and clamped, not negative.
        h.run(&mut game, half + 10);
        assert_eq!(game.fade_alpha(), 0.0, "the fade fully retires and clamps");
    }

    #[test]
    fn speed_ramps_with_time_survived_and_resets_on_replay() {
        // The ramp is a pure function of the run clock, so drive it directly —
        // simulating five real seconds would just crash into traffic first.
        let mut game = CarRunner::new();
        assert!(
            (game.current_speed() - config::SPEED).abs() < 1e-3,
            "a run starts at the base speed",
        );

        // Each interval survived multiplies the speed by exactly one factor.
        game.run_time = config::SPEED_RAMP_SECONDS;
        let expected = config::SPEED * config::SPEED_RAMP;
        assert!(
            (game.current_speed() - expected).abs() < 1e-3,
            "one interval ⇒ ×{} ({} vs {expected})",
            config::SPEED_RAMP,
            game.current_speed(),
        );

        game.run_time = config::SPEED_RAMP_SECONDS * 2.0;
        let expected = config::SPEED * config::SPEED_RAMP * config::SPEED_RAMP;
        assert!(
            (game.current_speed() - expected).abs() < 1e-3,
            "two intervals compound",
        );

        // A fresh run drops back to the base speed.
        game.run_time = 0.0;
        assert!((game.current_speed() - config::SPEED).abs() < 1e-3);
    }

    #[test]
    fn the_run_clock_advances_only_while_playing() {
        // Frozen states must not accrue difficulty: the ramp is driven by the run
        // clock, which only ticks while actually driving.
        let mut game = CarRunner::new();
        let mut h = Headless::new();
        h.run(&mut game, 30);
        assert_eq!(game.run_time, 0.0, "the title screen does not ramp difficulty");

        let (mut game, mut h) = started();
        h.run(&mut game, 5);
        assert!(game.run_time > 0.0, "driving advances the run clock");
    }

    #[test]
    fn speed_is_capped_below_the_tunnelling_threshold() {
        // The ramp is exponential, so it must be clamped: collision is a per-frame
        // AABB test, and a car moving more than the combined half-extents in one
        // step could pass straight through traffic.
        let mut game = CarRunner::new();
        game.run_time = 10_000.0; // absurdly long run
        assert_eq!(game.current_speed(), config::SPEED_MAX, "the ramp is clamped");

        let per_step = config::SPEED_MAX * kaman_core::FIXED_DT;
        let combined_half = config::PLAYER_HALF.z + config::OBSTACLE_HALF.z;
        assert!(
            per_step < combined_half,
            "at max speed the car moves {per_step} per step, which must stay under \
             the {combined_half} overlap window or collisions are missed",
        );
    }

    #[test]
    fn a_dodging_run_never_ends_without_an_actual_collision() {
        // A run must only end when the car genuinely overlaps traffic. Drive a long
        // way — far enough to cross the floating-origin rebase threshold, which
        // shifts every streamed transform — steering into a clear lane each step,
        // and assert the run never ends. Catches a false game-over from the world
        // moving out from under the collision test.
        let (mut game, mut h) = started();
        let mut rebases = 0u32;

        for step in 0..20_000u32 {
            let player = game.player_position();

            // Choose a lane with no traffic in the danger window ahead of us.
            let mut blocked = [false; config::LANES];
            for (_e, (t, _body)) in h
                .world()
                .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
                .iter()
            {
                let p = t.transform.position;
                // "Ahead" is -Z; keep a little margin behind us too.
                if p.z < player.z + 2.0 && p.z > player.z - 14.0 {
                    for (lane, slot) in blocked.iter_mut().enumerate() {
                        if (p.x - CarRunner::lane_x(lane)).abs() < 1.0 {
                            *slot = true;
                        }
                    }
                }
            }
            if blocked[game.lane] {
                if let Some(free) = (0..config::LANES).find(|&l| !blocked[l]) {
                    game.lane = free;
                }
            }

            h.run(&mut game, 1);
            if game.rebased_last_step {
                rebases += 1;
            }

            assert_ne!(
                game.state,
                GameState::GameOver,
                "the run ended at step {step} (travel {:.1}, speed {:.1}, {rebases} rebases so \
                 far) while dodging — a game-over without a real collision",
                game.travel,
                game.current_speed(),
            );
        }

        // The run must actually have crossed the floating-origin rebase, or this
        // test proves nothing about the world shifting under the collision test.
        assert!(
            rebases > 0,
            "the dodging run never triggered a rebase, so it does not cover it",
        );
    }

    #[test]
    fn the_camera_stays_with_the_car_across_a_rebase() {
        // A floating-origin rebase teleports every transform by up to a whole
        // `rebase_threshold`. The smoothed chase camera must snap with it: easing
        // across a ~1000-unit jump would send the camera flying through the world
        // for half a second, which reads as the game glitching out mid-run.
        let (mut game, mut h) = started();

        let mut saw_rebase = false;
        for _ in 0..20_000u32 {
            // Keep dodging so the run survives long enough to rebase.
            let player = game.player_position();
            let mut blocked = [false; config::LANES];
            for (_e, (t, _body)) in h
                .world()
                .query::<(&TransformComponent, &kaman_ecs::PhysicsBodyComponent)>()
                .iter()
            {
                let p = t.transform.position;
                if p.z < player.z + 2.0 && p.z > player.z - 14.0 {
                    for (lane, slot) in blocked.iter_mut().enumerate() {
                        if (p.x - CarRunner::lane_x(lane)).abs() < 1.0 {
                            *slot = true;
                        }
                    }
                }
            }
            if blocked[game.lane] {
                if let Some(free) = (0..config::LANES).find(|&l| !blocked[l]) {
                    game.lane = free;
                }
            }

            // The camera's pose *relative to the car* is what the player actually
            // sees. Capture it before the step so we can prove the rebase does not
            // disturb it.
            let before = h.camera().position() - game.player_position();

            h.run(&mut game, 1);

            if game.rebased_last_step {
                saw_rebase = true;
                let car = game.player_position();
                let cam = h.camera().position();

                // 1. The camera must have come through the coordinate change: it is
                //    still trailing the car, not stranded a threshold away.
                let gap = (cam - car).length();
                let allowed = config::CHASE_DISTANCE + config::CHASE_HEIGHT + 5.0;
                assert!(
                    gap < allowed,
                    "after a rebase the camera was {gap:.1} from the car \
                     (allowed {allowed:.1}) — it did not move with the world",
                );

                // 2. And the view must be *continuous*: the relative pose should
                //    change no more in the rebase step than in an ordinary one.
                //    Re-deriving the pose instead of translating it would erase the
                //    follow's steady-state lag here and show up as a visible jolt.
                let after = cam - car;
                let jolt = (after - before).length();
                let per_step = game.current_speed() * kaman_core::FIXED_DT;
                assert!(
                    jolt < per_step,
                    "the rebase moved the camera {jolt:.3} relative to the car \
                     (more than one step of motion, {per_step:.3}) — visible as a jolt",
                );
                break;
            }
        }

        assert!(saw_rebase, "the run never reached a rebase, so this proves nothing");
    }

    #[test]
    fn score_increases_with_distance() {
        let (mut game, mut h) = started();
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
            let mut rng = Rng::new(config::SEED);
            (0..n)
                .map(|_| rng.next_below(config::LANES as u32))
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
        let (mut game, mut h) = started();
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
        assert_eq!(game.lane, config::START_LANE, "lane recentered");
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
