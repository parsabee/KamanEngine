// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Game-side ECS components and the model-placement enum.
//!
//! These types ride alongside the engine-generic components
//! ([`TransformComponent`](kaman_ecs::TransformComponent),
//! [`RenderComponent`](kaman_ecs::RenderComponent),
//! [`PhysicsBodyComponent`](kaman_ecs::PhysicsBodyComponent), …) on the same ECS
//! entities, but they name game concepts (which car model, which building, which
//! decoration) — which is exactly why they live here in the game crate and not in
//! `kaman-ecs`: the engine's no-game-symbol guard would reject them there.

/// A game-side ECS component tagging an obstacle with which traffic car variant it
/// draws (index into [`crate::config::TRAFFIC_CAR_ASSETS`]). Assigned once at
/// spawn from the streaming slot so an obstacle keeps the same car for its whole
/// life.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TrafficVariant(
    /// Index into [`crate::config::TRAFFIC_CAR_ASSETS`].
    pub(crate) usize,
);

/// A game-side ECS component tagging a streamed roadside building with which prefab
/// it draws (index into [`crate::config::BUILDING_ASSETS`]). Assigned once at spawn
/// (deterministic per slot + side) so a building keeps the same prefab for its
/// whole life. A building is **non-colliding** decoration — it carries no physics
/// body.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuildingVariant(
    /// Index into [`crate::config::BUILDING_ASSETS`].
    pub(crate) usize,
);

/// A game-side ECS marker for a streamed guardrail segment (KE-0706), so the
/// renderer draws it with the shared guardrail mesh and excludes it from the road
/// pass. Non-colliding decoration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GuardrailTag;

/// Which imported model a placed entity draws: the player's car, one of the
/// traffic-car variants, or one of the roadside building prefabs.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PlacedModel {
    /// The player's car.
    Player,
    /// A traffic car, by variant index (into
    /// [`crate::game::CarRunner::traffic_cars`]).
    Traffic(usize),
    /// A roadside building, by prefab index (into
    /// [`crate::game::CarRunner::buildings`]).
    Building(usize),
}
