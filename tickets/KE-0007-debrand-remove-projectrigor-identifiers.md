# KE-0007 — De-brand: remove `ProjectRigor` identifiers

Phase:         0
Priority:      P1
Status:        Done
Integration:   Refactor
Size:          S · A1
Time:          S
Risk:          Low
Depends on:    KE-0003, KE-0004, KE-0005      Blocks: KR0.5 sign-off
Serves:        KR0.5

## Problem / Motivation
Migrated modules still carry the prototype's crate name, doc references, and license headers.
KamanEngine is its own product with its own license; no `ProjectRigor`/`projectrigor`
identifiers should remain in shipped code. Provenance is kept in docs only.

## Scope & Acceptance
- [x] Replace crate/module identifiers, `projectrigor::` paths, and doc references with the
      `kaman-*` equivalents across all migrated crates.
- [x] Update per-file license headers to the KamanEngine MIT header (2026, Parsa Bagheri).
- [x] A CI grep check fails the build if `ProjectRigor`/`projectrigor` appears anywhere except
      `docs/` provenance notes.
- [x] Add a short "Provenance" note in `ARCHITECTURE.md` (origin prototype) — the only allowed mention.

## Technical notes
- Run after the leaf migrations so renames touch already-moved code; keep as its own commit for
  a clean diff.

## Out of scope
- Renderer/scene/camera renaming (those crates migrate in later phases; de-brand each as it lands).

## Test gate
Grep check green (no stray identifiers); `cargo build/test --workspace` + oracle green.

## Doc gate
License headers consistent; `ARCHITECTURE.md` provenance note present.
