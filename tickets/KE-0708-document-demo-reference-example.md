# KE-0708 — Document the Playable Demo as a reference example

Phase:         7
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A0
Time:          M
Risk:          Low
Depends on:    KE-0702, KE-0703, KE-0704, KE-0705, KE-0706, KE-0707      Blocks: —
Serves:        KR7.3

## Problem / Motivation
The Playable Demo is the **example a developer follows** to build a game on KamanEngine. That only
works if it's cleanly architected and thoroughly documented — the docs are a deliverable, not an
afterthought.

## Scope & Acceptance
- [ ] A demo guide (`games/playable-demo/README.md` and a section in `docs/`) walking through how the
      demo is built **on the engine's public API only**: the `Game`/`EngineCtx` boundary, the
      fixed-timestep loop, streaming, asset loading, the render seam, and the HUD.
- [ ] A "build your own game" how-to: the minimal steps to stand up a new `Game`, load assets, and
      render — pointing at the demo's code as the worked example.
- [ ] Architecture notes: how the demo keeps game concepts out of engine crates (the boundary), and a
      diagram or link into `docs/DESIGN.md`.
- [ ] The demo code itself is exemplary: `#![deny(missing_docs)]`-clean, well-commented, no dead code,
      no reaching past the engine's public API.
- [ ] Controls + how to run documented in the top-level `README`.

## Technical notes
- Cross-link `docs/DESIGN.md` (diagrams), `ARCHITECTURE.md` (constraints), and the rustdoc.
- Do this last in Phase 7 so it documents the finished demo.

## Out of scope
- Video/tutorial content. Documenting unbuilt engine features.

## Test gate
`cargo doc --workspace --no-deps` builds clean (no broken links); `cargo test --workspace` green
(doc examples, if any, compile); the demo's no-game-symbol/firewall/de-brand guards hold.

## Doc gate
The demo guide + how-to exist and are linked from the root `README` and `docs/`; rustdoc clean.
