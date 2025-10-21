//! Cross-platform atomic wait/notify shims used by the transport primitives.
//!
//! Web workers park on wasm linear-memory atomics via `memory_atomic_wait32`
//! while native targets rely on the `atomic-wait` crate (futex-backed where
//! available). Loom tests stub these operations so deterministic schedulers
//! keep working.

#[cfg(feature = "loom")]
use loom::sync::atomic::{AtomicU32, Ordering};
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{AtomicU32, Ordering};

/// Result of attempting to wait on an atomic location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitResult {
    /// The value matched and the caller was woken by a notify.
    Ok,
    /// The value no longer matched when the wait was attempted.
    NotEqual,
    /// The wait timed out before a notify was observed.
    TimedOut,
}

#[cfg(feature = "loom")]
mod imp {
    use super::{AtomicU32, WaitResult};

    #[inline]
    pub(crate) fn wait_u32(atomic: &AtomicU32, expected: u32) -> WaitResult {
        let _ = (atomic, expected);
        WaitResult::NotEqual
    }

    #[inline]
    pub(crate) fn wake_one(atomic: &AtomicU32) -> u32 {
        let _ = atomic;
        0
    }

    #[inline]
    pub(crate) fn wake_all(atomic: &AtomicU32) -> u32 {
        let _ = atomic;
        0
    }
}

#[cfg(all(not(feature = "loom"), target_arch = "wasm32"))]
mod imp {
    use super::{AtomicU32, WaitResult};
    use core::arch::wasm32::{memory_atomic_notify, memory_atomic_wait32};

    #[inline]
    pub(crate) fn wait_u32(atomic: &AtomicU32, expected: u32) -> WaitResult {
        // SAFETY: The atomic resides in the shared linear memory backing the transport rings.
        let result = unsafe {
            memory_atomic_wait32(atomic as *const _ as *mut i32, expected as i32, -1_i64)
        };
        const WAIT_OK: i32 = 0;
        const WAIT_NOT_EQUAL: i32 = 1;
        const WAIT_TIMED_OUT: i32 = 2;
        match result {
            WAIT_OK => WaitResult::Ok,
            WAIT_NOT_EQUAL => WaitResult::NotEqual,
            WAIT_TIMED_OUT => WaitResult::TimedOut,
            _ => WaitResult::NotEqual,
        }
    }

    #[inline]
    pub(crate) fn wake_one(atomic: &AtomicU32) -> u32 {
        // SAFETY: Pointer addresses the same shared linear memory used for waits.
        unsafe { memory_atomic_notify(atomic as *const _ as *mut i32, 1) }
    }

    #[inline]
    pub(crate) fn wake_all(atomic: &AtomicU32) -> u32 {
        // SAFETY: Pointer addresses the same shared linear memory used for waits.
        unsafe { memory_atomic_notify(atomic as *const _ as *mut i32, u32::MAX) }
    }
}

#[cfg(all(not(feature = "loom"), not(target_arch = "wasm32")))]
mod imp {
    use super::{AtomicU32, WaitResult};

    #[inline]
    pub(crate) fn wait_u32(atomic: &AtomicU32, expected: u32) -> WaitResult {
        atomic_wait::wait(atomic, expected);
        WaitResult::Ok
    }

    #[inline]
    pub(crate) fn wake_one(atomic: &AtomicU32) -> u32 {
        atomic_wait::wake_one(atomic as *const AtomicU32);
        1
    }

    #[inline]
    pub(crate) fn wake_all(atomic: &AtomicU32) -> u32 {
        atomic_wait::wake_all(atomic as *const AtomicU32);
        0
    }
}

/// Blocks the current caller until the atomic differs from `expected` or a wakeup occurs.
#[inline]
pub fn wait_u32(atomic: &AtomicU32, expected: u32) -> WaitResult {
    imp::wait_u32(atomic, expected)
}

/// Wakes at most one waiter parked on `atomic`.
#[inline]
pub fn wake_one(atomic: &AtomicU32) -> u32 {
    imp::wake_one(atomic)
}

/// Wakes all waiters parked on `atomic`.
#[inline]
pub fn wake_all(atomic: &AtomicU32) -> u32 {
    imp::wake_all(atomic)
}

/// Convenience helper that captures the current value and waits for a change.
#[inline]
pub fn wait_for_change(atomic: &AtomicU32, order: Ordering) -> WaitResult {
    let expected = atomic.load(order);
    wait_u32(atomic, expected)
}
