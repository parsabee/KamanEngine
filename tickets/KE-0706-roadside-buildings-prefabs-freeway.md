# KE-0706 — Roadside buildings (prefabs) + elevated freeway

Phase:         7
Priority:      P1
Status:        Todo
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
- [ ] Author/commit **5–6 building prefab** meshes (`.glb`/`.gltf`) under the demo's `assets/`
      (varied heights/silhouettes), imported via `kaman-assets` (KE-0402/KE-0703 path), loaded once.
- [ ] Stream buildings on **both sides** of the road as the player advances (reuse `Scene::stream`,
      KE-0203): pick a prefab per slot with the **seeded PRNG** (deterministic), vary spacing/offset,
      and despawn behind — no unbounded growth, no per-frame allocation.
- [ ] Buildings sit on the **ground plane below** the elevated road (so the freeway is clearly raised);
      the road/lanes/cars move up to the freeway height, buildings stay at ground level.
- [ ] Randomization is reproducible (same seed → same city) so the demo is deterministic/testable.

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
