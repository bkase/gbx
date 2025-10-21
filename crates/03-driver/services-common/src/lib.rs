#![deny(missing_docs)]
//! Shared helpers for queue-backed mock services.
//!
//! The GPU, audio, filesystem, and kernel mocks behave similarly when their
//! report queues reach capacity. This crate centralises the common flow so the
//! concrete services focus on policies and report construction.

use hub::{SubmitOutcome, SubmitPolicy};
use smallvec::SmallVec;
use std::cell::UnsafeCell;
use std::collections::VecDeque;

/// Single-threaded queue wrapper used by mock services.
///
/// The scheduler operates on a single thread, so we can rely on SPSC-style
/// access while still exposing a type that is `Send + Sync` for trait object
/// ergonomics.
pub struct LocalQueue<T> {
    inner: UnsafeCell<VecDeque<T>>,
}

impl<T> LocalQueue<T> {
    /// Creates a new queue with the requested capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: UnsafeCell::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Grants temporary mutable access to the underlying deque.
    #[inline]
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut VecDeque<T>) -> R) -> R {
        // SAFETY: The queue is only mutated by the scheduler thread. Service
        // trait objects require `Sync`, so we guarantee no concurrent access.
        let deque = unsafe { &mut *self.inner.get() };
        f(deque)
    }
}

// SAFETY: `LocalQueue` enforces single-threaded mutation; callers can move the
// queue across threads because the scheduler never shares mutable references
// concurrently. Elements are `Send`, so moving them between threads is sound.
unsafe impl<T: Send> Send for LocalQueue<T> {}
// SAFETY: Although the underlying storage uses interior mutability, the
// scheduler guarantees single-threaded access. Declaring `Sync` allows the
// queue to live behind `Arc<dyn Service + Send + Sync>` without introducing
// races.
unsafe impl<T: Send> Sync for LocalQueue<T> {}

/// Attempts to push reports into a bounded queue following the provided policy.
///
/// Returns the resulting [`SubmitOutcome`], matching the semantics expected by
/// the scheduler.
pub fn try_submit_queue<Rep, F>(
    queue: &LocalQueue<Rep>,
    capacity: usize,
    policy: SubmitPolicy,
    needed: usize,
    materialise: F,
) -> SubmitOutcome
where
    Rep: Send + 'static,
    F: FnOnce() -> SmallVec<[Rep; 8]>,
{
    queue.with_mut(|inner| {
        match policy {
            SubmitPolicy::BestEffort => {
                if inner.len() + needed > capacity {
                    return SubmitOutcome::Dropped;
                }
            }
            SubmitPolicy::Coalesce => {
                let mut coalesced = false;
                while inner.len() + needed > capacity && !inner.is_empty() {
                    inner.pop_front();
                    coalesced = true;
                }

                if inner.len() + needed > capacity {
                    return SubmitOutcome::WouldBlock;
                }

                let reports = materialise();
                inner.extend(reports);
                return if coalesced {
                    SubmitOutcome::Coalesced
                } else {
                    SubmitOutcome::Accepted
                };
            }
            SubmitPolicy::Must | SubmitPolicy::Lossless => {
                if inner.len() + needed > capacity {
                    return SubmitOutcome::WouldBlock;
                }
            }
        }

        let reports = materialise();
        inner.extend(reports);
        SubmitOutcome::Accepted
    })
}

/// Drains up to `max` reports from the queue.
pub fn drain_queue<Rep>(queue: &LocalQueue<Rep>, max: usize) -> SmallVec<[Rep; 8]>
where
    Rep: Send + 'static,
{
    if max == 0 {
        return SmallVec::new();
    }

    queue.with_mut(|inner| {
        let mut out = SmallVec::<[Rep; 8]>::new();
        let limit = max.min(inner.len());
        for _ in 0..limit {
            if let Some(rep) = inner.pop_front() {
                out.push(rep);
            }
        }
        out
    })
}
