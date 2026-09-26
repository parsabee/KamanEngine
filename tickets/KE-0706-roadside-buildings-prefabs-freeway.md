# KE-0706 — Roadside buildings (prefabs) + elevated freeway

Phase:         7
Priority:      P1
Status:        Done
Integration:   New
Size:          L · A1
Time:          L
Risk:          Med
Depends on:    KE-0701, KE-0703, KE-0203      Blocks: KE-0708
Serves:        KR7.2

## Problem / Motivation
Line the road with a city: **buildings** streamed along both sides, randomly chosen from **5–6
prefab models**, so the world feels populated and varied. And make the road a **freeway** — raised
off the ground — so you're driving above the city.

## Scope & Acceptance
- [x] Author/commit **5–6 building prefab** meshes (`.glb`/`.gltf`) under the demo's `assets/`
      (varied heights/silhouettes), imported via `kaman-assets` (KE-0402/KE-0703 path), loaded once.
      → **8** CC0 Kenney City Kit prefabs ([`BUILDING_ASSETS`]): 2 skyscrapers, 3 large, 2 small,
      1 low. Loaded once each via `load_model(.., building_fit)` through the asset cache.
- [x] Stream buildings on **both sides** of the road as the player advances (reuse `Scene::stream`,
      KE-0203): pick a prefab per slot with the **seeded PRNG** (deterministic), vary spacing/offset,
      and despawn behind — no unbounded growth, no per-frame allocation.
      → one per side per slot in `spawn_slot`, prefab chosen by a **weighted** per-slot hash
      (`building_for_slot`: skyscrapers 8%, large 55%, small/low 37%) with jittered lateral offset,
      each turned 90° to face the freeway; reported via `SpawnCtx::spawned` so they despawn behind.
      Non-colliding decoration (no physics body). Bounded count covered by the existing
      `streaming_stays_bounded_over_a_long_run` test.
- [x] Buildings sit on the **ground plane below** the elevated road (so the freeway is clearly raised);
      the road/lanes/cars move up to the freeway height, buildings stay at ground level.
      → buildings spawn at `GROUND_Y = -10` while the road deck stays at `≈ -0.3`. Added a
      camera-locked **ground terrain** sheet (`terrain_geometry`): a level valley floor under the road
      and both building rows that climbs into rolling hills on the flanks, so the buildings stand on
      ground and the sky no longer shows through the mid-ground. **Guardrails** (`guardrail_geometry`)
      stream along both road edges, so the deck reads as a real elevated freeway.
- [x] Randomization is reproducible (same seed → same city) so the demo is deterministic/testable.
      → prefab choice, side jitter and traffic variant all come from `hash_u64` over the streaming
      slot — deterministic and **independent of the lane PRNG**, so the obstacle world (and the smoke
      run's score of 23) is unchanged.

## Technical notes
- Buildings are streamed content like obstacles but **non-colliding** decoration (don't give them the
  obstacle physics body / collision role).
- Reuse the KE-0703 asset cache so each prefab parses+uploads once and is shared by handle across all
  its instances (KE-0103 discipline, one level up).
- Keep game concepts (building/freeway) in the demo crate only; engine no-game-symbol guards hold.

## Out of scope
- Interior/enterable buildings, building textures beyond base color, traffic on side streets.

## Test gate
`cargo test -p playable-demo` green: deterministic prefab selection from a seed; bounded building
count over a long run. `cargo run -p playable-demo` shows a randomized elevated freeway through a
city; `--smoke` exits 0; clippy + firewall + de-brand clean.

## Doc gate
Document the prefab-streaming approach + the elevated-freeway layout in the demo docs.
