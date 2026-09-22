# car-runner

KamanEngine's first title — a playable **endless runner** — and the host for the
headless **smoke oracle**.

## Play it

```sh
cargo run -p car-runner
```

Opens a Metal window on macOS. You are a box driving forward down a three-lane
road; obstacles (yellow boxes) stream toward you. Dodge them.

A **chase camera** (`kaman_camera::ChaseController`) trails the box from behind
and above and keeps it framed as it drives — so the runner is visually playable,
not viewed from a fixed point. The game positions the engine-owned camera each
`update`; the engine pushes its view-projection across the render seam before the
frame is drawn (KE-0205).

### Controls

| Key            | Action                          |
| -------------- | ------------------------------- |
| `Left` / `A`   | Move one lane left              |
| `Right` / `D`  | Move one lane right             |
| `Space`        | Restart the run (also after a crash) |
| `Escape`       | Quit                            |

Your **score** climbs with distance travelled and is printed to stdout as you
go; a crash prints the final score and best-so-far, then the run restarts.

## The game loop

The engine drives the game at a **fixed timestep** (`kaman_core::FIXED_DT`), so
behavior is framerate-independent. Each fixed `update`:

1. **Restart** on `Space` (clears nearby obstacles, zeroes the score).
2. **Lane input** — `Left/A` / `Right/D` move the box between three discrete
   lanes (clamped at the edges). Movement is **kinematic**: the box's transform
   is set directly, never driven by the physics solver.
3. **Advance** forward one step at a constant speed.
4. **Rebase then stream** — `Scene::maybe_rebase` runs first (floating-origin
   shift, between physics steps), then `Scene::stream` spawns road tiles and
   deterministically-placed obstacles ahead of the box and despawns whatever has
   fallen behind. The box is the streaming focus.
5. **Collision** — a game-side AABB check (same lane + overlap along the travel
   axis) resets the run on a hit. Contact detection is done in the game from
   entity transforms; no physics contact-query API is used.

Obstacle placement is a seeded PRNG (SplitMix64) — no wall-clock time is read —
so a run is fully reproducible.

## The smoke oracle

```sh
cargo run -p car-runner -- --smoke
```

Boots the same game via the engine's headless driver, simulates 120 frames
offscreen against a `NullRenderer`, prints `smoke: 120 frames OK`, and exits 0.
It requires **no GPU/Metal device**, so it runs on headless CI runners. Headless
input is empty, so the box just runs straight down the middle lane.

Part of the [KamanEngine](../../README.md) workspace. Apache-2.0.
