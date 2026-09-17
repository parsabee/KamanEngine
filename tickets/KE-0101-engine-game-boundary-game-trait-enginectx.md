# KE-0101 — Engine/game boundary: `Game` trait + `EngineCtx`

Phase:         1
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          L · A3
Time:          L
Risk:          High
Depends on:    KE-0005      Blocks: KE-0102, Phase 2 gameplay (KE-0201, KE-0204)
Serves:        KR1.1

## Problem / Motivation
The prototype's `app.rs` owns the event loop *and* the scene/game logic in one place. To keep
game concepts (car, road, score) out of the engine crates (ARCHITECTURE §3), the engine must
own the loop and call into the game through a narrow seam: a `Game` trait the game implements,
and an `EngineCtx` handle the engine passes in. This is the A3 that Phase 1 hangs off — land it
first and alone (WIP-limit 1).

## Scope & Acceptance
- [ ] Define a `Game` trait in `kaman-core` with lifecycle hooks: `init(&mut self, ctx: &mut EngineCtx)`,
      `update(&mut self, ctx: &mut EngineCtx, dt: f32)`, and `render(&mut self, ctx: &mut EngineCtx)`
      (render hook may be a no-op in Phase 1; the fixed-timestep split is refined in KE-0201).
- [ ] Define `EngineCtx` exposing engine services the game is allowed to touch: the ECS `World`
      (`kaman-ecs`), a `&mut dyn RenderDevice` / frame recorder seam (`kaman-render-api`), input
      snapshot, and frame timing (`kaman-perf`). No Metal types, no game types.
- [ ] Engine owns the loop: `kaman-core` drives `winit` (macOS) and calls `init` once then
      `update`/`render` per frame, passing `EngineCtx`. The prototype's `app.rs` loop logic moves
      into `kaman-core` **as a move-commit first, refactor second** (INTEGRATION §2.3).
- [ ] `games/car-runner` implements `Game` and contains the only game-aware code; the `--smoke`
      oracle boots a `Game` impl and runs 120 frames headlessly.
- [ ] A compile-time/test guard asserts no game-named symbols leaked into `kaman-core` (extend the
      KE-0005 pattern).

## Technical notes
- `EngineCtx` is a borrow-checker chokepoint: prefer passing `&mut EngineCtx` with field accessors
  over handing out long-lived `&mut` to sub-systems, so the game can't alias engine state.
- Keep the render access behind the `kaman-render-api` traits (KE-0006) — the game never sees Metal.
- This crate is `kaman-core`; the platform `#[cfg]` split lands later (KE-0301), so keep macOS-only
  paths isolated behind small functions to ease that split.

## Out of scope
- Fixed-timestep accumulator semantics (KE-0201). Platform abstraction / iOS (KE-0301).
- The real Metal renderer behind the seam (KE-0102) — Phase 1's smoke still clears a color.

## Test gate
`cargo test --workspace` green; the no-game-symbols guard for `kaman-core` green; `--smoke`
boots a `Game` impl and exits 0; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; every `Game` hook and `EngineCtx` accessor documents its contract and
call-ordering invariants; `kaman-core` README + ARCHITECTURE §3 describe the boundary; a doc
example shows a minimal `Game` impl.
