# KamanEngine Roadmap

The engine is built as a **phase-gated pipeline**. Each phase runs the same loop, and a
phase does not start until the previous phase's OKRs are green.

## The phase loop

```
1. Define work + phase OKRs
2. Write tickets   (new IDs, sized on two axes, with test + doc gates)
3. Implement tickets   (per-module pipeline — see INTEGRATION.md §2.3)
4. SQA testing   (run every gate: unit, macOS oracle, device smoke)
5. OKR review   (did we hit the Key Results?)
6. Gate:  pass → next phase   |   miss → write remediation tickets, repeat 3–5
```

"Functioning engine" is reached at the end of **Phase 3** (a real game running on both
targets). Phases 4–6 turn it into a shippable product.

---

## Phase 0 — Foundation & Migration Harness

**Objective:** Stand up KamanEngine as its own repo with a green, test-guarded workspace and
the lowest-risk modules migrated.

- **KR0.1** `cargo build --workspace` + `cargo test --workspace` green in CI on every commit.
- **KR0.2** macOS headless smoke oracle boots a scene, renders 120 frames, exits 0 — in CI.
- **KR0.3** `kaman-math`, `kaman-perf`, `kaman-ecs` migrated as crates, each covered by its own unit tests; `#![deny(missing_docs)]` compiles.
- **KR0.4** `kaman-render-api` seam landed with a `NullRenderer` test double and no `metal` dependency above the seam.
- **KR0.5** Zero `ProjectRigor`/`projectrigor` identifiers remain in migrated code.

## Phase 1 — Renderer Foundation (mobile-safe Metal)

**Objective:** A Metal renderer with zero per-frame allocations behind a clean engine/game
seam, with provably identical output.

- **KR1.1** Engine/game boundary landed — `Game` trait + `EngineCtx`; no game type in engine crates.
- **KR1.2** Zero `new_buffer*` calls in the per-frame path (allocations instrument); persistent mesh buffers + uniform ring + 3 frames-in-flight.
- **KR1.3** Golden pixel-hash of the reference scene stable across the refactor (or updated with written justification).
- **KR1.4** Raytracer feature-gated out of default/iOS build; shaders load from precompiled `.metallib` (no runtime compile).
- **KR1.5** 60 fps on macOS M-series; GPU frame time ≤ 4 ms on the reference scene.

## Phase 2 — Gameplay Core (prove it's fun, macOS)

**Objective:** A playable endless-runner loop on macOS driving the engine's public API.

- **KR2.1** Fixed-timestep update/render split correct at 60 & 120 Hz; `Game::update(ctx, dt)` hook.
- **KR2.2** rapier wrapper gains body/collider removal; use-after-free test green; no stale-handle deref.
- **KR2.3** World streaming (spawn-ahead / despawn-behind + origin rebase); no memory growth over 10 min play.
- **KR2.4** Runner prototype (car = box) playable; **"is it fun?" gate reviewed.**

## Phase 3 — iOS Bring-up  *(functioning engine reached here)*

**Objective:** The runner runs on a physical iPhone.

- **KR3.1** Platform abstraction; `#[cfg]` macOS/iOS surface; no AppKit-only panics.
- **KR3.2** iOS app target + signed bundle; Rust staticlib via C-ABI; CAMetalLayer + CADisplayLink loop.
- **KR3.3** Touch input on the shared `InputEvent` path; TBDR memoryless depth on device.
- **KR3.4** ≥ 30 fps on an iPhone 12-class device; no crash in a 5-minute session.

## Phase 4 — Look & Feel + Content Pipeline

**Objective:** The game looks modern and runs on real assets, with HUD and audio.

- **KR4.1** Modern-look stack: sRGB+tonemap, fog+gradient sky, memoryless MSAA, one shadow, bloom.
- **KR4.2** Static glTF import + base-color/normal/roughness textures + ASTC; runner uses real meshes.
- **KR4.3** 2D HUD/SDF text overlay (safe-area aware) + audio (SFX/music) on device.

## Phase 5 — KamanScript

**Objective:** Game-specific behavior authored in the custom language, not compiled Rust.
(May start once the ECS public API is stable — end of Phase 2 — and run parallel to Phase 4.)

- **KR5.1** KamanScript v1 (frozen 20-construct spec): logos lexer + recursive-descent parser + tree-walking interpreter with line/col errors.
- **KR5.2** ECS host bindings (spawn/despawn/get/set/query, input, time, physics, audio); example-script corpus asserted end-to-end.
- **KR5.3** Hot-reload on macOS; runner gameplay ported to `.kaman`; engine crates contain only bootstrap.

## Phase 6 — Release

**Objective:** On TestFlight, shippable to the App Store, stable on target hardware.

- **KR6.1** Signing/provisioning, archive→ipa, App Store assets + privacy manifest; on TestFlight.
- **KR6.2** Device GPU capture + thermal/memory within budget on the min-spec device.
- **KR6.3** Passes App Store automated validation; crash-free across 100 test sessions.

## Phase 7 — Playable Demo  *(pulled forward — highest priority)*

**Objective:** A complete, good-looking, playable vertical slice on macOS — the **reference example**
a developer follows to build a game on KamanEngine. Prioritized ahead of Phases 3–6: it drives
engine features by real need and proves the public API is pleasant to build on. More is added as the
engine grows.

- **KR7.1** Playable slice: a 3-lane elevated **freeway**; **Left/Right** are the only gameplay keys
  and snap the car one lane (edge-triggered); score tracked; crash → **game over** reporting the
  score with a **replay** option.
- **KR7.2** Real content: authored complex **car models** (player + traffic), an **asphalt** road
  texture, a distant **city backdrop**, and roadside **buildings** randomly placed from 5–6 prefabs.
- **KR7.3** The game is renamed **"Playable Demo"** and is **documented as a reference example** —
  clean, engine-public-API-only, with an architecture + how-to guide a user can follow.

---

## Critical path

`P0 → P1 → P2 → P3` is strictly sequential (each gates the next). `P5 (KamanScript)` may run
parallel to `P4` once the Phase 2 ECS API is frozen. `P6` requires `P3`+`P4`.
