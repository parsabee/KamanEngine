# KE-0102 — Migrate renderer → `kaman-render` behind the seam

Phase:         1
Priority:      P0
Status:        Todo
Integration:   Refactor
Size:          L · A2
Time:          XL
Risk:          Critical
Depends on:    KE-0006, KE-0101      Blocks: KE-0103, KE-0104, KE-0105, KE-0106, KE-0107
Serves:        KR1.3, KR1.5

## Problem / Motivation
The prototype's `renderer.rs` (~1422 LOC) talks raw Metal and is called directly by the app.
Migrate the **rasterization** path into `crates/kaman-render` as the raw-Metal implementation of
the `kaman-render-api` traits (KE-0006), so nothing above the seam sees `metal`. This is the
highest-risk migration in the project; it lands behind the seam and is guarded by a render
pixel-hash, and every subsequent buffer/frames-in-flight change (KE-0103–0105) builds on it.

## Scope & Acceptance
- [ ] **Move commit:** relocate `renderer.rs` rasterization path into `crates/kaman-render`
      unchanged (raytracer comes along but is gated out in KE-0106); wire `metal`, `kaman-math`,
      `kaman-render-api`, `winit`/layer glue. No logic change. Oracle still green.
- [ ] **Refactor commit:** implement `RenderDevice` + `FrameRecorder` for the Metal backend;
      the app/game now drives rendering through the seam types, not Metal directly.
- [ ] `metal` appears **only** in `kaman-render` (and platform glue) — the KE-0006 CI firewall
      still passes for every crate above the seam.
- [ ] **Render pixel-hash guard:** add a deterministic offscreen render of a fixed reference
      scene → hash the pixel buffer → assert against a committed hash. Bless the baseline from the
      **migrated `kaman-render`** first-correct frame (not the prototype), then hold it stable
      across KE-0103/0104/0105. Provide a `BLESS=1` (or `xtask bless`) path with a required
      justification note. (This is the KR1.3 net; it replaces the dropped KE-0002 render scaffold.)
- [ ] Metal API Validation on in the CI debug/offscreen run.

## Technical notes
- Per-frame allocations exist today (`renderer.rs:458` allocates a uniform buffer every frame;
  mesh buffers via `new_buffer_with_data`). **Do not fix them here** — KE-0102 preserves behavior;
  KE-0103/0104 remove the allocations. Keeping this ticket behavior-preserving is what makes the
  pixel-hash meaningful across the later changes.
- Shaders still compile at runtime here (`new_library_with_source(include_str!(...))`); the
  `.metallib` precompile is KE-0107.
- Refactor (Int-class): raise unit/where-testable coverage on the new public surface to ≥ 80%
  (INTEGRATION §2.4); GPU paths covered by the pixel-hash + Metal validation (§2.7).

## Out of scope
- Persistent mesh buffers (KE-0103), uniform ring/argument buffers (KE-0104), frames-in-flight
  (KE-0105), raytracer gating (KE-0106), `.metallib` (KE-0107).

## Test gate
`cargo test --workspace` green incl. the render pixel-hash; KE-0006 metal-firewall green for all
above-seam crates; macOS oracle green; clippy clean.

## Doc gate
`#![deny(missing_docs)]`; `kaman-render` README explains the backend and how it implements the
seam; the pixel-hash capture/re-bless workflow documented; ARCHITECTURE §2 updated to note the
backend now exists behind the seam.
