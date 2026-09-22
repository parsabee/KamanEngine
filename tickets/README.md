# KamanEngine — Ticket System & Backlog

All tickets are **new to KamanEngine**. IDs encode the phase: `KE-0PNN`, where `P` is the
phase digit (Phase 0 → `KE-00NN`, Phase 1 → `KE-01NN`, …). Phases and their OKRs live in
[../docs/ROADMAP.md](../docs/ROADMAP.md); the implementation/test discipline is
[../docs/INTEGRATION.md](../docs/INTEGRATION.md).

Only the **active phase's** tickets are written in full. Later-phase tickets are listed in
the index and fleshed out when that phase begins (per the phase loop).

## Ticket header schema

```
# KE-0PNN — Title
Phase:         0..6
Priority:      P0 | P1 | P2 | P3
Status:        Todo | In-Progress | In-Review | Blocked | Done
Integration:   Reuse-as-is | Refactor | New
Size:          <LOC-band> · <A-level>      e.g.  M · A2
Time:          S | M | L | XL
Risk:          Low | Med | High | Critical
Depends on:    KE-…      Blocks: KE-…
Serves:        KR<phase>.<n>               (the OKR key result this advances)
Test gate:     <tests that must be green to close>
Doc gate:      <rustdoc + invariants + arch/README note required to close>
## Problem / Scope & Acceptance / Technical notes / Out of scope
```

A ticket reaches **Done** only when its Test gate + Doc gate are green **and** the macOS
oracle still builds and runs.

## Sizing — two axes

**LOC band** (net engine impl churn; tests counted separately):

| Band | LOC |
|---|---|
| XS | < 50 |
| S | 50–200 |
| M | 200–600 |
| L | 600–1500 |
| XL | > 1500 |

**Architecture-change level** (blast radius across module/contract boundaries):

| Level | Meaning | Rule |
|---|---|---|
| A0 | Local/additive — one module, no API change | safe, parallelizable |
| A1 | Internal refactor — module internals; callers unaffected | guard with characterization tests |
| A2 | Contract change — public API/trait/layout others depend on | update dependents in same PR |
| A3 | Foundational — introduces/moves a boundary the whole system plugs into | do first, alone; WIP-limit 1 |

**Risk** is derived: `f(A-level, test-coverage gap, hardware dependence)`. GPU/iOS tickets
are bumped one tier (not fully unit-testable).

## Definitions

- **Priority:** P0 blocks the phase OKR · P1 needed for the phase · P2 polish/release · P3 later.
- **Integration class:** Reuse-as-is / Refactor / New — sets the coverage target (INTEGRATION §2.4).
- **Status flow:** `Todo → In-Progress → In-Review → Done`; `Blocked` carries the blocking ID.
- **WIP limit:** at most one A2/A3 ticket In-Progress at a time.

---

## Backlog index (board)

### Phase 0 — Foundation & Migration Harness  *(complete)*
| # | Title | Pri | Int | Size | Status | Serves |
|---|---|---|---|---|---|---|
| KE-0001 | Workspace + repo bootstrap + CI oracle | P0 | New | L · A3 | Done | KR0.1/0.2 |
| KE-0003 | Migrate math → `kaman-math` | P0 | Reuse | XS · A0 | Done | KR0.3 |
| KE-0004 | Migrate perf → `kaman-perf` | P1 | Reuse | S · A0 | Done | KR0.3 |
| KE-0005 | Migrate ecs → `kaman-ecs` | P0 | Reuse | S · A1 | Done | KR0.3 |
| KE-0006 | `kaman-render-api` seam skeleton | P0 | New | S · A3 | Done | KR0.4 |
| KE-0007 | De-brand: remove `ProjectRigor` identifiers | P1 | Refactor | S · A1 | Done | KR0.5 |

### Phase 1 — Renderer Foundation  *(complete — KE-0107 gated on full Xcode)*
| # | Title | Pri | Int | Size | Status | Depends | Serves |
|---|---|---|---|---|---|---|---|
| KE-0101 | Engine/game boundary: `Game` trait + `EngineCtx` | P0 | Refactor | L · A3 | Done | KE-0005 | KR1.1 |
| KE-0102 | Migrate renderer → `kaman-render` behind seam | P0 | Refactor | L · A2 | Done | KE-0006, KE-0101 | KR1.3/1.5 |
| KE-0103 | Persistent mesh buffers + handle registry | P0 | Refactor | M · A2 | Done | KE-0102 | KR1.2 |
| KE-0104 | Uniform ring + argument buffers | P0 | Refactor | M · A2 | Done | KE-0102 | KR1.2 |
| KE-0105 | Triple-buffered frames-in-flight + pacing | P0 | Refactor | S · A1 | Done | KE-0102, KE-0104 | KR1.2/1.5 |
| KE-0106 | Feature-gate raytracer out of default/iOS | P0 | Refactor | XS · A0 | Done | KE-0102 | KR1.4 |
| KE-0107 | Precompiled `.metallib` shaders | P0 | Refactor | S · A1 | In-Progress | KE-0102 | KR1.4 |

**Phase 1 order:** KE-0101 (A3, alone) → KE-0102 (A2) → then KE-0103/0104 (A2, one at a
time per WIP limit) → KE-0105; KE-0106/0107 (A0/A1) land any time after KE-0102.

### Phase 2 — Gameplay Core  *(complete)*
| # | Title | Pri | Int | Size | Status | Depends | Serves |
|---|---|---|---|---|---|---|---|
| KE-0201 | Fixed-timestep loop + `Game::update` hook | P0 | Refactor | M · A2 | Done | KE-0101 | KR2.1 |
| KE-0202 | rapier wrapper + removal API + use-after-free guard | P0 | Refactor | S · A0 | Done | KE-0005 | KR2.2 |
| KE-0203 | World streaming: spawn/despawn + origin rebase | P0 | New | L · A2 | Done | KE-0201, KE-0202 | KR2.3 |
| KE-0204 | `car-runner` prototype (box car) | P1 | New | M · A0 | Done | KE-0201, KE-0202, KE-0203 | KR2.4 |
| KE-0205 | Migrate camera → `kaman-camera` + chase controller | P1 | Refactor | L · A2 | Done | KE-0102, KE-0204 | KR2.4 |

**Phase 2 order:** KE-0201 (A2) and KE-0202 (A0) can go in either order (0202 is independent);
then KE-0203 (A2) needs both; KE-0204 (the playable prototype + "is it fun?" gate) is last.
All of Phase 2 is macOS/CLT-friendly (pure Rust + rapier) — no Xcode needed.

### Phase 3 — iOS Bring-up *(planned)*
| # | Title | Pri | Int | Size |
|---|---|---|---|---|
| KE-0301 | Platform abstraction (`#[cfg]` surface/input) | P0 | Refactor | M · A3 |
| KE-0302 | iOS app target + bundle + staticlib C-ABI | P0 | New | M · A2 |
| KE-0303 | CAMetalLayer on UIView + CADisplayLink loop | P0 | New | M · A2 |
| KE-0304 | Input abstraction + touch/tilt | P1 | Refactor | S · A2 |
| KE-0305 | TBDR memoryless depth / MSAA | P0 | Refactor | S · A1 |

### Phase 4 — Look & Feel + Content *(planned)*
| # | Title | Pri | Int | Size |
|---|---|---|---|---|
| KE-0401 | Modern-look rendering stack | P1 | Refactor | L · A1 |
| KE-0402 | Static glTF import | P1 | New | M · A1 |
| KE-0403 | Textures + ASTC + mipmaps | P1 | New | M · A1 |
| KE-0404 | 2D HUD / SDF text overlay | P1 | New | M · A1 |
| KE-0405 | Audio (kira) | P1 | New | M · A0 |

### Phase 5 — KamanScript *(planned; may start after Phase 2 API freeze)*
| # | Title | Pri | Int | Size |
|---|---|---|---|---|
| KE-0501 | Spec freeze — 20-construct language | P0 | New | S · A0 |
| KE-0502 | Lexer (logos) + recursive-descent parser + AST | P0 | New | L · A1 |
| KE-0503 | Tree-walking interpreter + ECS host bindings | P0 | New | L · A2 |
| KE-0504 | Hot-reload + port runner logic to `.kaman` | P1 | New | M · A1 |

### Phase 6 — Release *(planned)*
| # | Title | Pri | Int | Size |
|---|---|---|---|---|
| KE-0601 | Signing / bundling / TestFlight | P2 | New | S · A0 |
| KE-0602 | Device profiling: GPU capture + thermal/memory | P2 | Refactor | S · A0 |
| KE-0603 | App Store validation + crash-free soak | P2 | New | S · A0 |
