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

## Building

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p car-runner -- --smoke   # headless oracle: 120 frames, exits 0
```

The workspace is `crates/kaman-*` (engine) plus `games/car-runner` (first title + smoke
oracle). Layout follows [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §3.

**Toolchain:** `rust-toolchain.toml` pins `1.91.0`. That file is honored by rustup-based
setups (and CI). This dev machine uses a Homebrew rust with no rustup, so the pin is not
enforced locally — the Homebrew toolchain is used instead.

**iOS targets (Phase 3 prerequisite, not installed now):**

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

## Status

**Phase 0 complete** (workspace, CI oracle, `kaman-math`/`kaman-perf`/`kaman-ecs` migrated,
`kaman-render-api` seam). **Phase 1 — Renderer Foundation** is active. See the ticket board
for live status.

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

Free to use, modify, and distribute (including commercially), but you must
retain the copyright and attribution notices per the license.
