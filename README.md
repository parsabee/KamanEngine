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

KamanEngine is produced by **migrating and refactoring** an earlier prototype
(`ProjectRigor`) into this repo, module by module, under a test-and-document-as-you-go
discipline. Nothing is copied without a characterization test and rustdoc. The migration
runs as a **phase-gated pipeline** — each phase defines work, writes tickets, implements
them, runs SQA, and must hit its OKRs before the next phase starts.

- Roadmap & phase OKRs: [docs/ROADMAP.md](docs/ROADMAP.md)
- Integration & testing strategy: [docs/INTEGRATION.md](docs/INTEGRATION.md)
- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Ticket system & backlog: [tickets/README.md](tickets/README.md)

## Status

**Phase 0 — Foundation & Migration Harness.** See the ticket board for live status.

## License

MIT — see [LICENSE](LICENSE).
