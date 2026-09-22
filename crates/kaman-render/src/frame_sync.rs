// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! Frames-in-flight pacing primitive (KE-0105).
//!
//! A small **counting semaphore** used to bound how far the CPU may run ahead of
//! the GPU. The backend initializes it to [`MAX_FRAMES_IN_FLIGHT`] permits;
//! `begin_frame` [`acquire`](FrameSemaphore::acquire)s one permit (blocking if
//! all three frames are already queued) and the `MTLCommandBuffer` completion
//! handler [`release`](FrameSemaphore::release)s one when its frame finishes on
//! the GPU. This is the standard, mobile-safe triple-buffering model.
//!
//! [`MAX_FRAMES_IN_FLIGHT`]: crate::backend::MAX_FRAMES_IN_FLIGHT
//!
//! # Why not a raw `dispatch_semaphore`?
//!
//! A `dispatch_semaphore` would work, but this self-contained
//! [`Condvar`]-backed counter has a guaranteed API, no extra dependency, blocks
//! the CPU (**never a busy-wait/spin**), and — crucially — its
//! [`release`](FrameSemaphore::release) path is **allocation-free**, which is the
//! discipline required of the completion handler that runs on a Metal-owned
//! thread (KE-0105 acceptance).
//!
//! # Invariant enforced
//!
//! The permit count is exactly "GPU frames not yet completed" subtracted from
//! [`MAX_FRAMES_IN_FLIGHT`]. Because a frame's CPU writes happen *after* its
//! [`acquire`] returns, and the ring region for frame `F` is reused only by
//! frame `F + MAX_FRAMES_IN_FLIGHT`, the CPU can only reach that later frame once
//! frame `F`'s completion handler has [`release`]d — so **no in-flight ring slot
//! is ever CPU-written while the GPU is still reading it**.

use std::sync::{Condvar, Mutex};

/// A blocking counting semaphore with a fixed number of permits.
///
/// `acquire` waits (parking the thread on a [`Condvar`], never spinning) until a
/// permit is available and takes one; `release` returns a permit and wakes one
/// waiter. Cloneable-by-`Arc` at the call site so a command-buffer completion
/// handler can hold a handle and `release` from a Metal-owned thread.
pub struct FrameSemaphore {
    /// Permits currently available (0..=`max`).
    permits: Mutex<u32>,
    /// Signalled by `release` to wake a blocked `acquire`.
    available: Condvar,
}

impl FrameSemaphore {
    /// Create a semaphore pre-loaded with `permits` permits.
    #[must_use]
    pub fn new(permits: u32) -> Self {
        Self {
            permits: Mutex::new(permits),
            available: Condvar::new(),
        }
    }

    /// Take one permit, blocking the calling thread until one is available.
    ///
    /// Blocks on a [`Condvar`] (the OS parks the thread) — there is **no
    /// busy-wait**. Called by `begin_frame`, so the CPU can never have more than
    /// the initial permit count of frames queued at once.
    pub fn acquire(&self) {
        let mut permits = self.permits.lock().expect("frame semaphore poisoned");
        while *permits == 0 {
            permits = self
                .available
                .wait(permits)
                .expect("frame semaphore poisoned");
        }
        *permits -= 1;
    }

    /// Return one permit and wake a blocked [`acquire`](Self::acquire).
    ///
    /// **Allocation-free**: it only locks a mutex, increments a counter, and
    /// notifies the condvar. Invoked from the `MTLCommandBuffer` completion
    /// handler, which runs on a Metal-owned thread and must not allocate.
    pub fn release(&self) {
        let mut permits = self.permits.lock().expect("frame semaphore poisoned");
        *permits += 1;
        self.available.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn acquire_takes_permits_up_to_the_limit_then_release_frees_one() {
        let sem = Arc::new(FrameSemaphore::new(3));
        // Three acquires succeed without blocking.
        sem.acquire();
        sem.acquire();
        sem.acquire();

        // A fourth acquire must block until a release happens. Prove it by
        // releasing from another thread and joining.
        let s = Arc::clone(&sem);
        let handle = std::thread::spawn(move || {
            s.acquire(); // blocks until the release below
        });

        // Give the spawned thread a moment to park, then release.
        std::thread::sleep(std::time::Duration::from_millis(20));
        sem.release();
        handle.join().expect("acquirer thread should unblock");
    }
}
