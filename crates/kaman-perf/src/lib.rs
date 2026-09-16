// Copyright (c) 2025 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! CPU-side performance monitoring and frame timing for KamanEngine.
//!
//! The core type is [`PerfTracker`]: call [`begin_frame`](PerfTracker::begin_frame)
//! and [`end_frame`](PerfTracker::end_frame) around each frame, bracket
//! subsystems with [`mark_start`](PerfTracker::mark_start) /
//! [`mark_physics`](PerfTracker::mark_physics) / etc., and read back rolling
//! averages via [`current_fps`](PerfTracker::current_fps),
//! [`avg_fps`](PerfTracker::avg_fps), or a whole-window [`PerfSnapshot`] from
//! [`snapshot`](PerfTracker::snapshot).
//!
//! # Time source
//!
//! Timing is read through an injectable [`Clock`] rather than a hardcoded
//! [`std::time::Instant`]. Production code uses the default [`SystemClock`];
//! tests inject a [`MockClock`] so the rolling-average math is deterministic.
//! See the [`clock`] module.
//!
//! # `perf-hud` feature
//!
//! Timing and averaging are *always* compiled. Only human-readable stdout
//! output — [`PerfTracker::print_summary`] and the summary printed on drop — is
//! gated behind the off-by-default `perf-hud` cargo feature, so the macOS smoke
//! oracle keeps clean output. Without the feature, `print_summary` is a no-op
//! and dropping a tracker prints nothing.

#![deny(missing_docs)]

pub mod clock;

use std::collections::VecDeque;
use std::time::Duration;

pub use clock::{Clock, MockClock, SystemClock};

/// Number of most-recent frames kept in the rolling-average window.
pub const WINDOW_SIZE: usize = 60;

/// Performance metrics captured for a single frame.
#[derive(Debug, Clone, Copy, Default)]
struct FrameMetrics {
    total_time: Duration,
    physics_time: Duration,
    query_time: Duration,
    render_time: Duration,
    buffer_allocs: u32,
    bytes_uploaded: u64,
}

/// An immutable summary of the tracker's current rolling window and cumulative
/// totals.
///
/// Produced by [`PerfTracker::snapshot`]. All durations are averages over the
/// current window (up to [`WINDOW_SIZE`] frames) except `current_fps`, which is
/// derived from the most recent frame, and the `total_*` / `current_memory`
/// fields, which are cumulative since the tracker was created.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfSnapshot {
    /// Number of frames currently in the rolling window.
    pub frame_count: usize,
    /// FPS derived from the most recent frame's total time.
    pub current_fps: f64,
    /// FPS derived from the average frame time over the window.
    pub avg_fps: f64,
    /// Average total frame time over the window.
    pub avg_frame_time: Duration,
    /// Shortest total frame time in the window (stutter detection).
    pub min_frame_time: Duration,
    /// Longest total frame time in the window (stutter detection).
    pub max_frame_time: Duration,
    /// Average physics-subsystem time over the window.
    pub avg_physics_time: Duration,
    /// Average spatial-query-subsystem time over the window.
    pub avg_query_time: Duration,
    /// Average render-subsystem time over the window.
    pub avg_render_time: Duration,
    /// Average number of buffer allocations per frame over the window.
    pub avg_allocations: f64,
    /// Average bytes uploaded per frame over the window.
    pub avg_bytes_uploaded: f64,
    /// Triangle count reported for the most recent frame.
    pub triangle_count: u32,
    /// Cumulative buffer allocations since tracker creation.
    pub total_allocations: u64,
    /// Cumulative bytes uploaded since tracker creation.
    pub total_bytes_uploaded: u64,
    /// Cumulative buffer frees since tracker creation.
    pub total_frees: u64,
    /// Cumulative bytes freed since tracker creation.
    pub total_bytes_freed: u64,
    /// Live allocated-minus-freed byte count.
    pub current_memory_bytes: u64,
}

/// Tracks CPU-side performance over a rolling window of frames.
///
/// Generic over the [`Clock`] used to measure elapsed time. Use
/// [`PerfTracker::new`] for the real [`SystemClock`], or
/// [`PerfTracker::with_clock`] to inject a [`MockClock`] in tests.
pub struct PerfTracker<C: Clock = SystemClock> {
    // Time source.
    clock: C,

    // Frame history (rolling window).
    frames: VecDeque<FrameMetrics>,

    // Current frame being tracked.
    current: FrameMetrics,
    frame_start: Option<Duration>,
    subsystem_start: Option<Duration>,

    // External state (passed in).
    triangle_count: u32,

    // Cumulative totals since tracker creation.
    total_allocations: u64,
    total_bytes_uploaded: u64,
    total_frees: u64,
    total_bytes_freed: u64,

    // Live memory tracking (current allocated - freed).
    current_memory_bytes: u64,
}

impl PerfTracker<SystemClock> {
    /// Create a tracker driven by real wall-clock time ([`SystemClock`]).
    pub fn new() -> Self {
        Self::with_clock(SystemClock::new())
    }
}

impl Default for PerfTracker<SystemClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Clock> PerfTracker<C> {
    /// Create a tracker driven by the supplied [`Clock`].
    ///
    /// Injecting a [`MockClock`] makes the timing math deterministic for tests.
    pub fn with_clock(clock: C) -> Self {
        Self {
            clock,
            frames: VecDeque::with_capacity(WINDOW_SIZE),
            current: FrameMetrics::default(),
            frame_start: None,
            subsystem_start: None,
            triangle_count: 0,
            total_allocations: 0,
            total_bytes_uploaded: 0,
            total_frees: 0,
            total_bytes_freed: 0,
            current_memory_bytes: 0,
        }
    }

    /// Call at the start of each frame. Resets per-frame metrics and starts the
    /// frame timer.
    pub fn begin_frame(&mut self) {
        self.frame_start = Some(self.clock.now());
        self.current = FrameMetrics::default();
    }

    /// Mark the start of a subsystem span (physics, query, or render).
    pub fn mark_start(&mut self) {
        self.subsystem_start = Some(self.clock.now());
    }

    /// Record elapsed time since [`mark_start`](Self::mark_start) as physics time.
    pub fn mark_physics(&mut self) {
        if let Some(start) = self.subsystem_start.take() {
            self.current.physics_time = self.clock.now().saturating_sub(start);
        }
    }

    /// Record elapsed time since [`mark_start`](Self::mark_start) as query time.
    pub fn mark_query(&mut self) {
        if let Some(start) = self.subsystem_start.take() {
            self.current.query_time = self.clock.now().saturating_sub(start);
        }
    }

    /// Record elapsed time since [`mark_start`](Self::mark_start) as render time.
    pub fn mark_render(&mut self) {
        if let Some(start) = self.subsystem_start.take() {
            self.current.render_time = self.clock.now().saturating_sub(start);
        }
    }

    /// Record a buffer allocation of `bytes` bytes for the current frame.
    pub fn record_allocation(&mut self, bytes: u64) {
        self.current.buffer_allocs += 1;
        self.current.bytes_uploaded += bytes;
        self.total_allocations += 1;
        self.total_bytes_uploaded += bytes;
        self.current_memory_bytes += bytes;
    }

    /// Record a buffer free of `bytes` bytes.
    pub fn record_free(&mut self, bytes: u64) {
        self.total_frees += 1;
        self.total_bytes_freed += bytes;
        self.current_memory_bytes = self.current_memory_bytes.saturating_sub(bytes);
    }

    /// Set the triangle count reported for the current frame.
    pub fn set_triangle_count(&mut self, count: u32) {
        self.triangle_count = count;
    }

    /// Call at the end of each frame. Stops the frame timer and pushes the
    /// frame into the rolling window, evicting the oldest if full.
    pub fn end_frame(&mut self) {
        if let Some(start) = self.frame_start.take() {
            self.current.total_time = self.clock.now().saturating_sub(start);
        }

        self.frames.push_back(self.current);
        if self.frames.len() > WINDOW_SIZE {
            self.frames.pop_front();
        }
    }

    /// FPS derived from the most recent frame's total time (0.0 if none).
    pub fn current_fps(&self) -> f64 {
        if let Some(last) = self.frames.back() {
            if last.total_time.as_secs_f64() > 0.0 {
                return 1.0 / last.total_time.as_secs_f64();
            }
        }
        0.0
    }

    /// FPS derived from the average frame time over the window (0.0 if empty).
    pub fn avg_fps(&self) -> f64 {
        if self.frames.is_empty() {
            return 0.0;
        }
        let avg_time = self.avg_frame_time();
        if avg_time.as_secs_f64() > 0.0 {
            1.0 / avg_time.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Average total frame time over the window ([`Duration::ZERO`] if empty).
    pub fn avg_frame_time(&self) -> Duration {
        if self.frames.is_empty() {
            return Duration::ZERO;
        }
        let total: Duration = self.frames.iter().map(|f| f.total_time).sum();
        total / self.frames.len() as u32
    }

    /// Shortest and longest total frame time in the window (for detecting
    /// stutter). Returns `(ZERO, ZERO)` if the window is empty.
    pub fn min_max_frame_time(&self) -> (Duration, Duration) {
        if self.frames.is_empty() {
            return (Duration::ZERO, Duration::ZERO);
        }
        let min = self.frames.iter().map(|f| f.total_time).min().unwrap();
        let max = self.frames.iter().map(|f| f.total_time).max().unwrap();
        (min, max)
    }

    /// Average `(physics, query, render)` subsystem times over the window.
    /// Returns all-`ZERO` if the window is empty.
    pub fn avg_subsystems(&self) -> (Duration, Duration, Duration) {
        if self.frames.is_empty() {
            return (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        }
        let physics: Duration = self.frames.iter().map(|f| f.physics_time).sum();
        let query: Duration = self.frames.iter().map(|f| f.query_time).sum();
        let render: Duration = self.frames.iter().map(|f| f.render_time).sum();
        let n = self.frames.len() as u32;
        (physics / n, query / n, render / n)
    }

    /// Average `(allocations_per_frame, bytes_per_frame)` over the window.
    /// Returns `(0.0, 0.0)` if the window is empty.
    pub fn avg_allocations(&self) -> (f64, f64) {
        if self.frames.is_empty() {
            return (0.0, 0.0);
        }
        let total_allocs: u32 = self.frames.iter().map(|f| f.buffer_allocs).sum();
        let total_bytes: u64 = self.frames.iter().map(|f| f.bytes_uploaded).sum();
        let n = self.frames.len() as f64;
        (total_allocs as f64 / n, total_bytes as f64 / n)
    }

    /// Capture the full current state as an immutable [`PerfSnapshot`].
    pub fn snapshot(&self) -> PerfSnapshot {
        let (min_frame_time, max_frame_time) = self.min_max_frame_time();
        let (avg_physics_time, avg_query_time, avg_render_time) = self.avg_subsystems();
        let (avg_allocations, avg_bytes_uploaded) = self.avg_allocations();
        PerfSnapshot {
            frame_count: self.frames.len(),
            current_fps: self.current_fps(),
            avg_fps: self.avg_fps(),
            avg_frame_time: self.avg_frame_time(),
            min_frame_time,
            max_frame_time,
            avg_physics_time,
            avg_query_time,
            avg_render_time,
            avg_allocations,
            avg_bytes_uploaded,
            triangle_count: self.triangle_count,
            total_allocations: self.total_allocations,
            total_bytes_uploaded: self.total_bytes_uploaded,
            total_frees: self.total_frees,
            total_bytes_freed: self.total_bytes_freed,
            current_memory_bytes: self.current_memory_bytes,
        }
    }

    /// Print a formatted performance summary to stdout.
    ///
    /// Only produces output when the crate is built with the `perf-hud`
    /// feature; otherwise it is a no-op so the smoke oracle stays clean.
    #[cfg(feature = "perf-hud")]
    pub fn print_summary(&self) {
        let (min_ft, max_ft) = self.min_max_frame_time();
        let (phys, query, render) = self.avg_subsystems();
        let (allocs, bytes) = self.avg_allocations();

        println!("\n╔═══════════════════════════════════════════════════════╗");
        println!("║           PERFORMANCE METRICS                         ║");
        println!("╚═══════════════════════════════════════════════════════╝");

        // Show current FPS only if it's reasonable (not more than 2x average).
        let avg_fps = self.avg_fps();
        let current_fps = self.current_fps();
        if current_fps > 0.0 && current_fps <= avg_fps * 2.0 {
            println!("  FPS:          {:<6.1} (current)", current_fps);
            println!(
                "                {:<6.1} (avg over {} frames)",
                avg_fps,
                self.frames.len()
            );
        } else {
            println!(
                "  FPS:          {:<6.1} (avg over {} frames)",
                avg_fps,
                self.frames.len()
            );
        }

        println!(" ───────────────────────────────────────────────────────");
        println!(
            "  Frame Time:   {:<6.2} ms (avg)",
            self.avg_frame_time().as_secs_f64() * 1000.0
        );
        println!("                {:<6.2} ms (min)", min_ft.as_secs_f64() * 1000.0);
        println!("                {:<6.2} ms (max)", max_ft.as_secs_f64() * 1000.0);

        println!(" ───────────────────────────────────────────────────────");
        println!("  Breakdown:");
        println!("    Physics:    {:<6.2} ms", phys.as_secs_f64() * 1000.0);
        println!("    Query:      {:<6.2} ms", query.as_secs_f64() * 1000.0);
        println!("    Render:     {:<6.2} ms", render.as_secs_f64() * 1000.0);

        println!(" ───────────────────────────────────────────────────────");
        println!("  GPU (per frame):");
        println!("    Allocations: {:<6.1} per frame", allocs);
        println!("    Upload:      {:<6.1} KB per frame", bytes / 1024.0);
        println!("    Triangles:   {:<6}", self.triangle_count);
        println!(
            "    Current Mem: {:<6.1} MB",
            self.current_memory_bytes as f64 / (1024.0 * 1024.0)
        );

        println!(" ───────────────────────────────────────────────────────");
        println!("  GPU (cumulative):");
        println!("    Total Allocs: {:<6}", self.total_allocations);
        println!(
            "    Total Upload: {:<6.1} MB",
            self.total_bytes_uploaded as f64 / (1024.0 * 1024.0)
        );
        println!("    Total Frees:  {:<6}", self.total_frees);
        println!(
            "    Total Freed:  {:<6.1} MB",
            self.total_bytes_freed as f64 / (1024.0 * 1024.0)
        );
        println!(
            "    Net Memory:   {:<6.1} MB",
            (self.total_bytes_uploaded as i64 - self.total_bytes_freed as i64) as f64
                / (1024.0 * 1024.0)
        );
        println!(
            "    Current Alloc: {:<6.1} MB\n",
            self.current_memory_bytes as f64 / (1024.0 * 1024.0)
        );
    }

    /// No-op summary when the `perf-hud` feature is disabled.
    #[cfg(not(feature = "perf-hud"))]
    #[inline]
    pub fn print_summary(&self) {}
}

#[cfg(feature = "perf-hud")]
impl<C: Clock> Drop for PerfTracker<C> {
    fn drop(&mut self) {
        println!("\n=== Performance Summary (on drop) ===");
        self.print_summary();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Run a frame on a mock clock: advance physics/query/render spans by the
    /// given millisecond counts and record the supplied `(count, bytes)`
    /// allocations.
    fn simulate_frame(
        tracker: &mut PerfTracker<MockClock>,
        clock: &MockClock,
        physics_ms: u64,
        query_ms: u64,
        render_ms: u64,
        allocations: &[(u64, u64)],
    ) {
        tracker.begin_frame();

        tracker.mark_start();
        clock.advance(MS * physics_ms as u32);
        tracker.mark_physics();

        tracker.mark_start();
        clock.advance(MS * query_ms as u32);
        tracker.mark_query();

        tracker.mark_start();
        clock.advance(MS * render_ms as u32);
        tracker.mark_render();

        for &(count, bytes) in allocations {
            for _ in 0..count {
                tracker.record_allocation(bytes);
            }
        }

        tracker.end_frame();
    }

    fn new_pair() -> (PerfTracker<MockClock>, MockClock) {
        let clock = MockClock::new();
        let tracker = PerfTracker::with_clock(clock.clone());
        (tracker, clock)
    }

    #[test]
    fn single_frame_records_total_time() {
        let (mut tracker, clock) = new_pair();
        tracker.begin_frame();
        clock.advance(MS * 10);
        tracker.end_frame();

        assert!(tracker.current_fps() > 0.0);
        assert_eq!(tracker.avg_frame_time(), MS * 10);
        // 10ms frame => exactly 100 FPS.
        assert!((tracker.current_fps() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn rolling_window_caps_at_window_size() {
        let (mut tracker, clock) = new_pair();
        for _ in 0..100 {
            tracker.begin_frame();
            clock.advance(MS);
            tracker.end_frame();
        }
        assert_eq!(tracker.frames.len(), WINDOW_SIZE);
    }

    #[test]
    fn subsystem_timing_is_exact() {
        let (mut tracker, clock) = new_pair();
        tracker.begin_frame();

        tracker.mark_start();
        clock.advance(MS * 5);
        tracker.mark_physics();

        tracker.mark_start();
        clock.advance(MS * 3);
        tracker.mark_query();

        tracker.end_frame();

        let (phys, query, render) = tracker.avg_subsystems();
        assert_eq!(phys, MS * 5);
        assert_eq!(query, MS * 3);
        assert_eq!(render, Duration::ZERO);
    }

    #[test]
    fn allocation_tracking_single_frame() {
        let (mut tracker, _clock) = new_pair();
        tracker.begin_frame();
        tracker.record_allocation(1024);
        tracker.record_allocation(2048);
        tracker.end_frame();

        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 2.0);
        assert_eq!(bytes, 3072.0);
    }

    #[test]
    fn allocation_average_across_frames() {
        let (mut tracker, _clock) = new_pair();

        tracker.begin_frame();
        tracker.record_allocation(48 * 1024);
        tracker.record_allocation(128);
        tracker.record_allocation(4);
        tracker.end_frame();

        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 3.0);
        assert_eq!(bytes, (48 * 1024 + 128 + 4) as f64);

        tracker.begin_frame();
        tracker.end_frame();

        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 1.5);
        assert_eq!(bytes, ((48 * 1024 + 132) / 2) as f64);

        tracker.begin_frame();
        tracker.end_frame();

        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 1.0);
        assert_eq!(bytes, ((48 * 1024 + 132) / 3) as f64);
    }

    #[test]
    fn allocations_decay_after_warmup() {
        let (mut tracker, _clock) = new_pair();

        tracker.begin_frame();
        tracker.record_allocation(1024);
        tracker.record_allocation(2048);
        tracker.record_allocation(512);
        tracker.end_frame();

        for _ in 0..59 {
            tracker.begin_frame();
            tracker.end_frame();
        }

        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 3.0 / 60.0);
        assert_eq!(bytes, (1024 + 2048 + 512) as f64 / 60.0);
    }

    #[test]
    fn continuous_allocations_stay_constant() {
        let (mut tracker, _clock) = new_pair();
        for _ in 0..60 {
            tracker.begin_frame();
            tracker.record_allocation(48 * 1024);
            tracker.record_allocation(128);
            tracker.record_allocation(4);
            tracker.end_frame();
        }
        let (allocs, bytes) = tracker.avg_allocations();
        assert_eq!(allocs, 3.0);
        assert_eq!(bytes, (48 * 1024 + 132) as f64);
    }

    #[test]
    fn fps_is_exact_at_60() {
        let (mut tracker, clock) = new_pair();
        // 16ms + 1 + 0 = a fixed frame; use exactly ~16.667ms via micros.
        for _ in 0..10 {
            tracker.begin_frame();
            clock.advance(Duration::from_micros(16_667));
            tracker.end_frame();
        }
        let fps = tracker.avg_fps();
        assert!((fps - 59.999).abs() < 0.1, "expected ~60 FPS, got {fps}");
    }

    #[test]
    fn frame_time_min_max_is_exact() {
        let (mut tracker, clock) = new_pair();
        simulate_frame(&mut tracker, &clock, 1, 0, 4, &[]); // 5ms
        simulate_frame(&mut tracker, &clock, 2, 0, 8, &[]); // 10ms
        simulate_frame(&mut tracker, &clock, 1, 0, 2, &[]); // 3ms

        let (min, max) = tracker.min_max_frame_time();
        assert_eq!(min, MS * 3);
        assert_eq!(max, MS * 10);
    }

    #[test]
    fn subsystem_breakdown_is_exact() {
        let (mut tracker, clock) = new_pair();
        for _ in 0..5 {
            simulate_frame(&mut tracker, &clock, 2, 3, 5, &[]);
        }
        let (physics, query, render) = tracker.avg_subsystems();
        assert_eq!(physics, MS * 2);
        assert_eq!(query, MS * 3);
        assert_eq!(render, MS * 5);
    }

    #[test]
    fn allocation_byte_averaging() {
        let (mut tracker, clock) = new_pair();
        simulate_frame(&mut tracker, &clock, 0, 0, 1, &[(2, 1024)]);
        simulate_frame(&mut tracker, &clock, 0, 0, 1, &[(3, 512)]);
        simulate_frame(&mut tracker, &clock, 0, 0, 1, &[(1, 4096)]);

        let (avg_allocs, avg_bytes) = tracker.avg_allocations();
        assert_eq!(avg_allocs, 2.0); // (2+3+1)/3
        assert_eq!(avg_bytes, 2560.0); // (2048+1536+4096)/3
    }

    #[test]
    fn triangle_count_is_retained() {
        let (mut tracker, _clock) = new_pair();
        tracker.begin_frame();
        tracker.set_triangle_count(1234);
        tracker.end_frame();
        assert_eq!(tracker.triangle_count, 1234);

        tracker.begin_frame();
        tracker.set_triangle_count(5678);
        tracker.end_frame();
        assert_eq!(tracker.triangle_count, 5678);
    }

    #[test]
    fn empty_tracker_is_zero_safe() {
        let (tracker, _clock) = new_pair();
        assert_eq!(tracker.current_fps(), 0.0);
        assert_eq!(tracker.avg_fps(), 0.0);
        assert_eq!(tracker.avg_frame_time(), Duration::ZERO);
        assert_eq!(tracker.min_max_frame_time(), (Duration::ZERO, Duration::ZERO));
        assert_eq!(
            tracker.avg_subsystems(),
            (Duration::ZERO, Duration::ZERO, Duration::ZERO)
        );
        assert_eq!(tracker.avg_allocations(), (0.0, 0.0));
    }

    #[test]
    fn window_overflow_keeps_latest_triangle_count() {
        let (mut tracker, clock) = new_pair();
        for i in 0..100 {
            tracker.begin_frame();
            tracker.set_triangle_count(i as u32);
            clock.advance(MS);
            tracker.end_frame();
        }
        assert_eq!(tracker.frames.len(), WINDOW_SIZE);
        assert_eq!(tracker.triangle_count, 99);
    }

    #[test]
    fn record_free_reduces_live_memory() {
        let (mut tracker, _clock) = new_pair();
        tracker.record_allocation(4096);
        tracker.record_allocation(1024);
        tracker.record_free(4096);
        let snap = tracker.snapshot();
        assert_eq!(snap.current_memory_bytes, 1024);
        assert_eq!(snap.total_allocations, 2);
        assert_eq!(snap.total_frees, 1);
        assert_eq!(snap.total_bytes_freed, 4096);
    }

    #[test]
    fn record_free_saturates_at_zero() {
        let (mut tracker, _clock) = new_pair();
        tracker.record_allocation(100);
        tracker.record_free(500);
        assert_eq!(tracker.snapshot().current_memory_bytes, 0);
    }

    #[test]
    fn snapshot_reflects_window() {
        let (mut tracker, clock) = new_pair();
        for _ in 0..60 {
            tracker.begin_frame();
            tracker.mark_start();
            clock.advance(Duration::from_micros(500));
            tracker.mark_physics();
            tracker.mark_start();
            clock.advance(Duration::from_micros(100));
            tracker.mark_query();
            tracker.mark_start();
            clock.advance(Duration::from_micros(7400));
            tracker.mark_render();
            tracker.record_allocation(48 * 1024);
            tracker.record_allocation(8 * 1024);
            tracker.record_allocation(8 * 1024);
            tracker.set_triangle_count(1000);
            tracker.end_frame();
        }

        let snap = tracker.snapshot();
        assert_eq!(snap.frame_count, 60);
        // 8ms frame => 125 FPS, exact.
        assert!((snap.avg_fps - 125.0).abs() < 1e-6, "got {}", snap.avg_fps);
        assert_eq!(snap.avg_physics_time, Duration::from_micros(500));
        assert_eq!(snap.avg_query_time, Duration::from_micros(100));
        assert_eq!(snap.avg_render_time, Duration::from_micros(7400));
        assert_eq!(snap.avg_allocations, 3.0);
        assert_eq!(snap.avg_bytes_uploaded, 64.0 * 1024.0);
        assert_eq!(snap.triangle_count, 1000);
    }
}
