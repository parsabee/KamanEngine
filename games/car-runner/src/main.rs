// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! `car-runner` — KamanEngine's first title: a playable endless runner, and the
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

use kaman_assets::{import_gltf, MeshAsset};
use kaman_camera::ChaseController;
use kaman_core::input::Key;
use kaman_core::{EngineCtx, Game};
use kaman_ecs::hecs::Entity;
use kaman_ecs::{DynamicTag, RenderComponent, StaticTag, TransformComponent};
use kaman_math::glam::Vec3;
use kaman_math::Transform;
use kaman_render_api::{
    MaterialParams, MeshData, MeshHandle, PipelineDescriptor, PipelineHandle, TextureData,
    TextureHandle, VertexLayout,
};
use kaman_scene::Scene;

/// Number of frames the smoke oracle simulates before exiting.
const SMOKE_FRAMES: u32 = 120;

/// Command-line arguments for `car-runner`.
#[derive(Parser, Debug)]
#[command(name = "car-runner", about = "KamanEngine car-runner + headless smoke oracle")]
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

/// Boot the `car-runner` [`Game`] and drive `frames` frames headlessly, then report success.
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

/// The car-runner game state.
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
    /// The untextured box mesh (default `[pos,normal,color]`), uploaded once in
    /// `init` and referenced by every color-driven box (road, obstacles).
    box_mesh: Option<MeshHandle>,
    /// The **textured** player mesh imported from the committed glTF asset
    /// (`[pos,normal,uv]`), drawn on the textured pipeline with its base-color
    /// texture (KE-0403).
    player_mesh: Option<MeshHandle>,
    /// The player mesh's base-color texture, uploaded once in `init` (KE-0403).
    player_texture: Option<TextureHandle>,
    /// The player's base-color factor from its glTF material (multiplied with the
    /// sampled texture).
    player_base_color: [f32; 4],
    /// The untextured Phong pipeline for the color-driven boxes, created in `init`.
    pipeline: Option<PipelineHandle>,
    /// The textured pipeline for the player mesh (base-color sampling), created in
    /// `init` (KE-0403).
    textured_pipeline: Option<PipelineHandle>,
    /// The chase camera controller that keeps the player framed (KE-0205). It
    /// trails the box from behind and above along the travel axis, with light
    /// smoothing so the follow eases rather than snapping.
    chase: ChaseController,
}

impl CarRunner {
    /// Forward speed of the player along the streaming axis, in units/second.
    const SPEED: f32 = 14.0;
    /// Number of discrete lanes.
    const LANES: usize = 3;
    /// Distance between adjacent lane centers along `X`, in world units.
    const LANE_WIDTH: f32 = 3.0;
    /// The player box's resting height (its transform's `Y`).
    const PLAYER_Y: f32 = 0.6;
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
            box_mesh: None,
            player_mesh: None,
            player_texture: None,
            player_base_color: [1.0, 1.0, 1.0, 1.0],
            pipeline: None,
            textured_pipeline: None,
            chase: ChaseController::new(Self::CHASE_DISTANCE, Self::CHASE_HEIGHT)
                .with_look_at_height(Self::CHASE_LOOK_AT_HEIGHT)
                .with_smoothing(Self::CHASE_SMOOTHING),
        }
    }

    /// The score derived from the current run's distance (1 point per world unit).
    fn score(&self) -> u64 {
        self.distance.max(0.0) as u64
    }

    /// Apply lane-switch input for this fixed step (kinematic, level-triggered).
    ///
    /// Left/A move one lane toward 0, Right/D one lane toward `LANES-1`, clamped at
    /// the edges. Both directions at once cancel. Level input means holding the key
    /// glides across lanes; that is fine for a first prototype (edge detection is
    /// KE-0304).
    fn apply_lane_input(&mut self, ctx: &EngineCtx) {
        let input = ctx.input();
        let left = input.is_key_down(Key::Left) || input.is_key_down(Key::A);
        let right = input.is_key_down(Key::Right) || input.is_key_down(Key::D);
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

    /// Restart the run after a crash (or on demand): report the score, zero it,
    /// recenter the lane, and clear the obstacles around the player so it doesn't
    /// instantly re-collide. The world keeps scrolling — `travel` is monotonic, so
    /// the scene's (private) spawn frontier stays valid and the road ahead is
    /// unbroken. Fully deterministic: no PRNG reseed, no wall-clock read.
    fn reset(&mut self, ctx: &mut EngineCtx) {
        self.best = self.best.max(self.distance);
        println!(
            "crash! score {} (distance {:.1}) — best {:.1}. restarting.",
            self.score(),
            self.distance,
            self.best
        );

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

            let obstacle = cx.world.spawn((
                TransformComponent::from_position(pos),
                kaman_ecs::PhysicsBodyComponent::new(handle),
                RenderComponent::cube([0.95, 0.8, 0.1]),
                StaticTag,
            ));
            cx.spawned(obstacle);
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

        // Load the player mesh from the committed **textured** glTF asset
        // (KE-0403): its `[pos,normal,uv]` geometry + decoded base-color texture
        // are uploaded once (KE-0103) and drawn on the textured pipeline so the
        // player renders with a real texture. The color-driven boxes (road,
        // obstacles) keep the untextured `[pos,normal,color]` Phong path, so both
        // pipelines are created here.
        let mesh: MeshAsset = load_player_mesh();
        let renderer = ctx.renderer();

        // Untextured Phong pipeline + a plain color box mesh for road/obstacles.
        self.pipeline = Some(renderer.create_pipeline(&PipelineDescriptor {
            vertex_shader: "vertex_main".into(),
            fragment_shader: "fragment_main".into(),
            vertex_layout: color_box_layout(),
        }));
        let (box_vertices, box_indices) = color_box_geometry();
        self.box_mesh = Some(renderer.create_mesh(&MeshData {
            vertices: &box_vertices,
            indices: &box_indices,
            layout: color_box_layout(),
        }));

        // Textured pipeline + the imported player mesh + its base-color texture.
        self.textured_pipeline = Some(renderer.create_pipeline(&PipelineDescriptor {
            vertex_shader: "textured_vertex_main".into(),
            fragment_shader: "textured_fragment_main".into(),
            vertex_layout: mesh.layout.clone(),
        }));
        self.player_mesh = Some(renderer.create_mesh(&MeshData {
            vertices: &mesh.vertices,
            indices: &mesh.indices,
            layout: mesh.layout.clone(),
        }));
        if let Some(base) = &mesh.base_color {
            self.player_base_color = mesh.base_color_factor;
            self.player_texture = Some(renderer.create_texture(&TextureData {
                width: base.width,
                height: base.height,
                rgba8: &base.rgba8,
            }));
        }

        // Prime the road ahead so the first frame is not empty.
        let focus = self.player_position();
        let tile_depth = ctx.scene().config().spawn_interval;
        let rng = &mut self.rng;
        ctx.scene_mut()
            .stream(focus, |cx| Self::spawn_slot(rng, tile_depth, cx));
    }

    fn update(&mut self, ctx: &mut EngineCtx, dt: f32) {
        // Restart on demand (Space): also the "start over" affordance after a
        // crash, and always deterministic.
        if ctx.input().is_key_down(Key::Space) {
            self.reset(ctx);
        }

        // Kinematic lane movement from input (no solver involvement).
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

        // Collision → reset the run.
        if self.hit_any_obstacle(ctx) {
            self.reset(ctx);
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
        // Read this frame's transforms + colors. The player is drawn on the
        // textured pipeline (its base-color texture); every other box (road,
        // obstacles) is drawn on the untextured color pipeline (KE-0403). Both
        // meshes are persistent (KE-0103: no per-frame mesh upload).
        let player_entity = self.player;
        let box_mesh = self.box_mesh.expect("box mesh created in init");
        let draws: Vec<(Entity, kaman_math::Transform, [f32; 3])> = ctx
            .world()
            .query::<(&TransformComponent, &RenderComponent)>()
            .iter()
            .map(|(e, (t, r))| (e, t.transform, r.color))
            .collect();

        let pipeline = self.pipeline.expect("pipeline created in init");
        let textured_pipeline = self.textured_pipeline.expect("textured pipeline in init");
        let player_mesh = self.player_mesh.expect("player mesh created in init");
        let player_texture = self.player_texture;
        let player_base_color = self.player_base_color;

        let renderer = ctx.renderer();
        renderer.begin_frame();

        // Color-driven boxes on the untextured pipeline.
        renderer.set_pipeline(pipeline);
        for (entity, transform, color) in &draws {
            if Some(*entity) == player_entity {
                continue;
            }
            let material = MaterialParams {
                base_color: [color[0], color[1], color[2], 1.0],
                ..MaterialParams::default()
            };
            renderer.draw_mesh(box_mesh, transform, &material);
        }

        // The player on the textured pipeline with its base-color texture bound.
        if let (Some(player_entity), Some(texture)) = (player_entity, player_texture) {
            if let Some((_, transform, _)) = draws.iter().find(|(e, _, _)| *e == player_entity) {
                renderer.set_pipeline(textured_pipeline);
                renderer.bind_texture(texture);
                let material = MaterialParams {
                    base_color: player_base_color,
                    ..MaterialParams::default()
                };
                renderer.draw_mesh(player_mesh, transform, &material);
            }
        }

        renderer.submit();
    }
}

/// Filesystem path to the committed player mesh asset (`assets/cube.gltf`),
/// resolved relative to this crate so it loads regardless of the working
/// directory.
const PLAYER_MESH_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/cube.gltf");

/// The untextured `[pos,normal,color]` layout (36-byte stride) the color boxes
/// (road, obstacles) are drawn with — the same layout the untextured Phong
/// pipeline binds.
fn color_box_layout() -> VertexLayout {
    kaman_assets::render_vertex_layout()
}

/// A unit cube packed onto the `[pos,normal,color]` layout with a white vertex
/// color (per-draw `MaterialParams` carries each box's actual color; the packed
/// color is unused by the shader beyond modulation, so white keeps it neutral).
///
/// Used for the color-driven road/obstacle boxes so they stay on the untextured
/// pipeline while the player uses its imported textured mesh (KE-0403).
fn color_box_geometry() -> (Vec<u8>, Vec<u32>) {
    // 8-corner cube with per-vertex normals pointing outward from center; a
    // simple, deterministic box that renders identically to the prior imported
    // (untextured) cube for these entities.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]]),
    ];
    let color = [1.0f32, 1.0, 1.0];
    let mut bytes = Vec::new();
    let mut indices = Vec::new();
    for (n, verts) in faces {
        let base = (bytes.len() / 36) as u32;
        for v in verts {
            for f in v {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in &n {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
            for f in &color {
                bytes.extend_from_slice(&f.to_ne_bytes());
            }
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (bytes, indices)
}

/// Import the shared player mesh from the committed glTF asset (KE-0402/KE-0403).
///
/// The single primitive is the whole mesh; on the (unexpected) event of a
/// missing/empty file the game panics loudly at init rather than draw nothing —
/// the asset is committed to the repo, so a failure here is a build/packaging
/// bug, not a runtime condition.
fn load_player_mesh() -> MeshAsset {
    let scene = import_gltf(PLAYER_MESH_PATH)
        .unwrap_or_else(|e| panic!("failed to import player mesh {PLAYER_MESH_PATH}: {e}"));
    scene
        .meshes
        .into_iter()
        .next()
        .expect("player mesh asset has at least one mesh")
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

        // Hold Left from the center for many steps: reach lane 0 and never underflow.
        let mut left_game = CarRunner::new();
        let mut hl = Headless::new();
        hl.input_mut().press_key(Key::Left);
        hl.run(&mut left_game, 10);
        assert_eq!(left_game.lane, 0, "clamped at the left edge");

        // Hold Right from the center: reach the last lane and never overflow.
        let mut right_game = CarRunner::new();
        let mut hr = Headless::new();
        hr.input_mut().press_key(Key::D); // the `D` alias also moves right
        hr.run(&mut right_game, 10);
        assert_eq!(right_game.lane, CarRunner::LANES - 1, "clamped at the right edge");
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

    #[test]
    fn collision_resets_the_run() {
        // Drive many frames: the deterministic stream guarantees the middle-lane
        // runner eventually meets an obstacle, which resets distance to ~0.
        let mut game = CarRunner::new();
        let mut h = kaman_core::headless::Headless::new();

        let mut saw_reset = false;
        let mut prev = 0.0f32;
        for _ in 0..600 {
            h.run(&mut game, 1);
            // A reset is a large backward jump in distance.
            if game.distance + 1.0 < prev {
                saw_reset = true;
                break;
            }
            prev = game.distance;
        }
        assert!(saw_reset, "the runner eventually crashes and the run resets");
    }

    #[test]
    fn restart_zeroes_score_but_keeps_world_scrolling() {
        // Run a while, move off-center, then press Space (restart). Score resets
        // to zero and the lane recenters, but the monotonic world position keeps
        // advancing so the road never breaks.
        let mut game = CarRunner::new();
        let mut h = kaman_core::headless::Headless::new();
        h.input_mut().press_key(Key::Left);
        h.run(&mut game, 40);
        let travel_before = game.travel;
        assert!(game.distance > 0.0);
        assert_ne!(game.lane, CarRunner::START_LANE, "moved off center");

        // Release the lane key, press Space, step once to trigger the restart.
        h.input_mut().release_key(Key::Left);
        h.input_mut().press_key(Key::Space);
        h.run(&mut game, 1);

        assert!(game.distance < CarRunner::SPEED * 0.05, "score distance reset");
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
