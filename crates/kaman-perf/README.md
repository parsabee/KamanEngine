# kaman-perf

CPU-side profiling and frame timing for KamanEngine.

## Responsibility

`kaman-perf` measures per-frame CPU work and maintains rolling averages over the
last `WINDOW_SIZE` (60) frames: frame time, per-subsystem breakdown
(physics / spatial-query / render), FPS, and buffer allocation/upload/free
accounting. It has no dependencies on the renderer or GPU — GPU-side timing
(command-buffer completion handlers) is a later phase (KE-0602) and is
explicitly out of scope here.

## API tour

The core type is [`PerfTracker`]. It reads elapsed time through an injectable
[`Clock`], defaulting to the real [`SystemClock`].

```rust
use kaman_perf::PerfTracker;

let mut perf = PerfTracker::new();

perf.begin_frame();

perf.mark_start();
// ... run physics ...
perf.mark_physics();

perf.mark_start();
// ... run spatial queries ...
perf.mark_query();

perf.mark_start();
// ... record the frame ...
perf.mark_render();

perf.record_allocation(64 * 1024); // a GPU buffer upload
perf.set_triangle_count(1_000);

perf.end_frame();

// Read back rolling averages:
let fps = perf.avg_fps();
let snapshot = perf.snapshot(); // whole-window immutable summary
let _ = (fps, snapshot);
```

Read-back methods:

- `current_fps` / `avg_fps` — FPS from the last frame / the window average.
- `avg_frame_time`, `min_max_frame_time` — frame timing and stutter bounds.
- `avg_subsystems` — average `(physics, query, render)` durations.
- `avg_allocations` — average `(allocations, bytes)` per frame.
- `snapshot` — a `PerfSnapshot` capturing the whole window plus cumulative
  memory totals in one immutable struct.
- `print_summary` — formatted stdout dump (see the feature flag below).

### Deterministic timing in tests

Inject a `MockClock` via `PerfTracker::with_clock` and drive time by hand so the
timing/rolling-average math is exact and reproducible — no wall-clock sleeps:

```rust
use std::time::Duration;
use kaman_perf::{PerfTracker, MockClock};

let clock = MockClock::new();
let mut perf = PerfTracker::with_clock(clock.clone());

perf.begin_frame();
clock.advance(Duration::from_millis(10)); // exactly 10ms
perf.end_frame();

assert!((perf.current_fps() - 100.0).abs() < 1e-9); // exactly 100 FPS
```

## `perf-hud` feature flag

Off by default. Timing and averaging are **always** compiled; only the
human-readable stdout output is gated:

- **Disabled (default):** `print_summary` is a no-op and dropping a tracker
  prints nothing. This keeps the macOS smoke oracle's output clean.
- **Enabled (`--features perf-hud`):** `print_summary` prints a formatted
  metrics table, and a summary is printed automatically when a `PerfTracker` is
  dropped.

```sh
cargo build -p kaman-perf                    # tracking only, no output
cargo build -p kaman-perf --features perf-hud # + stdout HUD / drop summary
```

[`PerfTracker`]: https://docs.rs/kaman-perf
[`Clock`]: https://docs.rs/kaman-perf
[`SystemClock`]: https://docs.rs/kaman-perf
