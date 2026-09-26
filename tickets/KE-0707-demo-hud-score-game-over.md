# KE-0707 — Demo HUD: score + game-over / replay overlay

Phase:         7
Priority:      P0
Status:        Done
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
- [x] Live **score** drawn as HUD text during `Playing` (top corner, safe-area aware).
      → `SCORE n` at `inset + HUD_MARGIN` in the top-left safe corner. Pinned headlessly by
      `playing_past_the_fade_draws_the_score_and_no_wash`.
- [x] A **game-over overlay** on crash: "Game Over", final score, and a "Press <key> to replay" prompt.
      → A partial dim (so the crashed scene stays readable behind) under a centered four-line banner:
      `GAME OVER` / `SCORE n` / `BEST n` / `PRESS SPACE TO REPLAY`. Pinned by
      `game_over_dims_the_scene_and_adds_a_banner`. The demo also gained a `Ready` title screen
      (`KAMAN RUNNER` / `PRESS SPACE TO START`) with a `HUD_FADE_SECONDS` opening fade.
- [x] Text is crisp at the window size (SDF via KE-0404); the overlay draws over the 3D scene last.
      → SDF glyphs at three sizes; `flush_overlay` runs at the end of `submit()`, after all 3D draws.
      The wash is recorded *first* so text composites over it — asserted by `assert_wash_first`.
- [x] The HUD reads game state from the demo (KE-0702) — it renders, it does not own game logic.
      → `hud.rs` holds no game logic at all: `draw_hud` is a `&self` method that reads `state`,
      `score()`, `best_score()` and `fade_alpha()` and emits quads. It mutates nothing.

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
