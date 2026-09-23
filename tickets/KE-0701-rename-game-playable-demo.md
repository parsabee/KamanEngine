# KE-0701 — Rename the demo game to `playable-demo` ("Playable Demo")

Phase:         7
Priority:      P0
Status:        Done
Integration:   Refactor
Size:          S · A1
Time:          S
Risk:          Low
Depends on:    KE-0204      Blocks: KE-0702, KE-0703, KE-0704, KE-0705, KE-0706, KE-0707, KE-0708
Serves:        KR7.3

## Problem / Motivation
The example game is the **reference a user follows** to build on KamanEngine, so it should be named
for what it is: the **Playable Demo**. Rename the `car-runner` crate/binary to `playable-demo` and
update every reference before the rest of Phase 7 builds on it (fewer references to churn later).

## Scope & Acceptance
- [x] Move `games/car-runner` → `games/playable-demo`; crate + `[[bin]]` name `playable-demo`.
- [x] Update the root `Cargo.toml` workspace members, `docs/ARCHITECTURE.md` §3 layout, `README`,
      and the CI smoke step (`cargo run -p playable-demo -- --smoke`).
- [x] Update the `--smoke` oracle references and any `-p car-runner` invocations across the repo.
- [x] Keep the game engine-public-API-only (the boundary is unchanged); no engine crate renamed.
- [x] A short in-repo display name "Playable Demo" (window title if trivial, README, crate description).

## Technical notes
- Pure rename/move; no gameplay change. Keep it as its own commit for a clean diff (INTEGRATION §2.5).
- The no-game-symbols guards on engine crates must still pass (game concepts stay in the demo crate).

## Out of scope
- Any gameplay/content change (later Phase 7 tickets).

## Test gate
`cargo build/test --workspace` green; `cargo run -p playable-demo -- --smoke` prints the smoke
contract and exits 0; clippy clean; CI updated and green.

## Doc gate
`README`/`ARCHITECTURE` reference `playable-demo`; crate `README` names it the Playable Demo.
