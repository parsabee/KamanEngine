//! Injectable time source.
//!
//! [`PerfTracker`](crate::PerfTracker) reads elapsed time through a [`Clock`]
//! rather than calling [`std::time::Instant::now`] directly. Production code
//! uses [`SystemClock`] (a thin wrapper over `Instant`); tests use
//! [`MockClock`], whose "now" only advances when explicitly told to, making the
//! timing and rolling-average math fully deterministic.

use std::time::Duration;

/// A monotonic source of elapsed time.
///
/// Implementations return an ever-increasing instant as a [`Duration`] measured
/// from an arbitrary but fixed origin. Only *differences* between values
/// returned by the same clock are meaningful.
pub trait Clock {
    /// Current instant, expressed as elapsed time since this clock's origin.
    ///
    /// Successive calls must never return a smaller value than a previous call
    /// (the clock is monotonic).
    fn now(&self) -> Duration;
}

/// Real wall-clock time backed by [`std::time::Instant`].
///
/// The origin is the moment the clock is constructed.
#[derive(Debug, Clone)]
pub struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    /// Create a clock whose origin is now.
    pub fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A deterministic clock whose time only moves when [`MockClock::advance`] is
/// called.
///
/// Intended for tests: construct one, hand it to a
/// [`PerfTracker`](crate::PerfTracker::with_clock), and drive time manually so
/// that frame durations are exact and reproducible.
///
/// The current instant is held in a [`Cell`](std::cell::Cell) so that
/// [`Clock::now`] can take `&self` while `advance` mutates through a shared
/// reference (the tracker owns the clock by value, and tests keep a handle via
/// [`MockClock::clone`], which shares the same underlying cell).
#[derive(Debug, Clone)]
pub struct MockClock {
    now: std::rc::Rc<std::cell::Cell<Duration>>,
}

impl MockClock {
    /// Create a mock clock starting at zero.
    pub fn new() -> Self {
        Self {
            now: std::rc::Rc::new(std::cell::Cell::new(Duration::ZERO)),
        }
    }

    /// Advance the clock by `delta`.
    pub fn advance(&self, delta: Duration) {
        self.now.set(self.now.get() + delta);
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MockClock {
    fn now(&self) -> Duration {
        self.now.get()
    }
}
