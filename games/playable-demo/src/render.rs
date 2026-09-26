// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The render pass: [`CarRunner`]'s half of the
//! [`Game::render`](kaman_core::Game::render) hook.
//!
//! Draws are grouped by pipeline, in this order, so each pipeline switch and
//! texture bind happens once per group rather than once per entity:
//!
//! 1. The distant skyline **backdrop** (textured pipeline).
//! 2. The **road tiles**, then every **textured** model part — car/building
//!    primitives whose material carries a base-color texture (textured
//!    pipeline).
//! 3. The **ground terrain**, then the **guardrails**, then every **untextured**
//!    model part — primitives with no base-color texture, drawn with the
//!    material color the importer packed as their vertex color (Phong
//!    pipeline).
//!
//! All meshes/textures/pipelines are created once in
//! [`Game::init`](kaman_core::Game::init) (KE-0103) and only referenced here by
//! handle; this module never allocates a GPU resource.

use kaman_core::EngineCtx;
use kaman_ecs::{PhysicsBodyComponent, RenderComponent, TransformComponent};
use kaman_math::glam::{Quat, Vec3};
use kaman_math::Transform;
use kaman_render_api::MaterialParams;

use crate::assets::{compose, pick_model, CarPart};
use crate::components::{BuildingVariant, GuardrailTag, PlacedModel, TrafficVariant};
use crate::config::{BACKDROP_DIST, BACKDROP_H, BACKDROP_W, BACKDROP_Y};
use crate::game::CarRunner;

impl CarRunner {
    /// Record one frame: gather this frame's draw transforms from the ECS world,
    /// then issue the grouped draws described at the module level.
    pub(crate) fn render_frame(&mut self, ctx: &mut EngineCtx) {
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
        // 4. The HUD (KE-0707). Recorded last, but the engine flushes the overlay
        //    in its own orthographic pass at `submit`, after every 3D draw — so it
        //    composites on top of the scene regardless of record order.
        self.draw_hud(renderer);

        renderer.submit();
    }

    /// The parts of the model a placement draws: the player's car, a traffic-car
    /// variant, or a building prefab (each clamped to its loaded set).
    fn model_parts(&self, model: PlacedModel) -> &[CarPart] {
        match model {
            PlacedModel::Player => &self.player_car,
            PlacedModel::Traffic(i) => pick_model(&self.traffic_cars, i),
            PlacedModel::Building(i) => pick_model(&self.buildings, i),
        }
    }

    /// The world transform for the ground terrain this frame (KE-0706): the sheet is
    /// baked in player-relative `Z`, so translating it to the player's `Z` keeps it
    /// camera-locked — the ground always fills the view and a stream/rebase never
    /// slides it.
    fn terrain_transform(&self) -> Transform {
        Transform::from_position(Vec3::new(0.0, 0.0, self.player_position().z))
    }

    /// The world transform for the skyline backdrop billboard this frame (KE-0705):
    /// a wide, tall quad placed [`BACKDROP_DIST`] units ahead of the player along
    /// the travel axis and **locked to the player's XZ** (centered on `x = 0`) so a
    /// world stream/rebase never shifts it — it reads as a fixed far skyline. Sits
    /// at [`BACKDROP_DIST`] < the camera far plane (100) so it is not clipped.
    fn backdrop_transform(&self) -> Transform {
        let player = self.player_position();
        Transform {
            position: Vec3::new(0.0, BACKDROP_Y, player.z - BACKDROP_DIST),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(BACKDROP_W, BACKDROP_H, 1.0),
        }
    }
}
