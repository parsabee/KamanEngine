# KE-0707 — Demo HUD: score + game-over / replay overlay

Phase:         7
Priority:      P0
Status:        Todo
Integration:   New
Size:          M · A1
Time:          M
Risk:          Med
Depends on:    KE-0701, KE-0404      Blocks: KE-0702, KE-0708
Serves:        KR7.1

## Problem / Motivation
The player needs on-screen feedback: a live **score** readout while playing, and a **game-over**
overlay reporting the final score with a **replay** prompt. This is the demo's use of the engine's
2D HUD/text feature (KE-0404).

## Scope & Acceptance
- [ ] Live **score** drawn as HUD text during `Playing` (top corner, safe-area aware).
- [ ] A **game-over overlay** on crash: "Game Over", final score, and a "Press <key> to replay" prompt.
- [ ] Text is crisp at the window size (SDF via KE-0404); the overlay draws over the 3D scene last.
- [ ] The HUD reads game state from the demo (KE-0702) — it renders, it does not own game logic.

## Technical notes
- Depends on the engine HUD/text capability (KE-0404). If KE-0404 lands first, this is pure game code
  using it; keep the demo engine-public-API-only.
- Keep HUD draws allocation-free per frame (glyph quads from a persistent atlas, KE-0404).

## Out of scope
- The text-rendering engine feature itself (KE-0404). Menus/settings screens. Localization.

## Test gate
`cargo run -p playable-demo` shows the live score and a game-over/replay overlay; `cargo test
--workspace` green; `--smoke` exits 0 (HUD path headless-safe or skipped); clippy clean.

## Doc gate
Document how the demo composes the HUD from the engine text API (the "game draws UI via the engine"
pattern) in the demo docs.
