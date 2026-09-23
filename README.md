# KamanEngine

A Rust game engine for **Apple platforms** (macOS + iPhone), rendering with **raw Metal**.
Its first title is an **infinite car runner** (static meshes, no animation rigs, arcade
physics). The engine also ships a custom high-level scripting language, **KamanScript**,
for authoring gameplay.

## Design commitments

- **Apple-only, raw Metal.** macOS and iOS both use Metal natively, so raw Metal is *one*
  code path, not two. A `kaman-render-api` trait seam keeps the door open for a future
  portable backend without touching engine logic. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
- **Cargo workspace** of focused `kaman-*` crates + a `games/car-runner` consumer that only
  ever touches the engine's public API.
- **hecs** (ECS) · **rapier3d** (physics, v1) · **glam** (math) · **kira** (audio, later).
  Physics uses rapier now; a custom arcade-physics/spatial-query layer replaces it later.

## How this project is built

KamanEngine is produced by **migrating and refactoring** an earlier prototype into this
repo, module by module, under a test-and-document-as-you-go discipline. Nothing is migrated
without unit tests and rustdoc. The migration runs as a **phase-gated pipeline** — each
phase defines work, writes tickets, implements them, runs SQA, and must hit its OKRs before
the next phase starts. Provenance of the prototype is recorded in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

- Roadmap & phase OKRs: [docs/ROADMAP.md](docs/ROADMAP.md)
- Integration & testing strategy: [docs/INTEGRATION.md](docs/INTEGRATION.md)
- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Ticket system & backlog: [tickets/README.md](tickets/README.md)

## Prerequisites

KamanEngine is **Apple-only** (macOS + iOS, raw Metal). Verify your toolchain before building:

```sh
./scripts/preflight.sh          # deps needed to build the engine today
./scripts/preflight.sh --ios    # also require the Phase 3 (iOS) + .metallib toolchain
```

Two tiers of dependencies:

| Tier | Tools | Needed for |
|---|---|---|
| **Required now** | macOS · Rust ≥ 1.91 (`cargo`/`rustc`, MSRV enforced by cargo) · Apple `clang` + macOS SDK (Command Line Tools) | Build/run the engine on macOS |
| **iOS / KE-0107** | Full **Xcode** (`metal`/`metallib` shader compiler) · `rustup` + `aarch64-apple-ios`(`-sim`) targets | iOS bring-up (Phase 3) and precompiled `.metallib` shaders |

Command Line Tools alone (`xcode-select --install`) covers the engine today — shaders currently
compile at runtime. Full Xcode becomes required at Phase 3; install it then (App Store), which
also unblocks KE-0107.

## Building

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p car-runner -- --smoke   # headless oracle: 120 frames, exits 0
```

The workspace is `crates/kaman-*` (engine) plus `games/car-runner` (first title + smoke
oracle). Layout follows [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §3.

**Toolchain:** `rust-toolchain.toml` pins `1.91.0` (with `rustfmt`, `clippy`, `rust-analyzer`).
Honored by rustup (the recommended setup) and CI. The MSRV is also enforced by cargo via
`rust-version`.

## Documentation

**📖 Live API docs: https://parsabee.github.io/KamanEngine/** — built from rustdoc and published
to GitHub Pages on every push to `main` (see `.github/workflows/docs.yml`).

Every engine crate compiles under `#![deny(missing_docs)]`, so the public surface is fully
documented inline. Build the docs locally with:

```sh
cargo doc --workspace --no-deps --open   # build + open the API docs in a browser
```

Architecture docs (prose + diagrams):
- [docs/DESIGN.md](docs/DESIGN.md) — **diagrams**: component, UML class, and sequence diagrams
  (Mermaid) of the layers, the render seam, the frame loop, streaming, and asset load.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — intentional constraints + the render seam.
- [docs/ROADMAP.md](docs/ROADMAP.md) — phases + OKRs · [docs/INTEGRATION.md](docs/INTEGRATION.md) — test/migration discipline.

The rustdoc covers the code; CI keeps it free of broken intra-doc links.

**iOS targets (Phase 3 prerequisite, not installed now):**

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

## Status

**Phases 0–2 complete.** Phase 0 (workspace, CI oracle, math/perf/ecs migrated, render seam);
Phase 1 (raw-Metal renderer behind the seam, persistent buffers + uniform ring + triple-
buffered frames-in-flight; `.metallib` precompile gated on full Xcode); Phase 2 (fixed-timestep
loop, physics + removal API, world streaming, and a playable box-car runner). **Phase 3 — iOS
bring-up** is next (needs full Xcode). See the ticket board for live status.

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

Free to use, modify, and distribute (including commercially), but you must
retain the copyright and attribution notices per the license.
