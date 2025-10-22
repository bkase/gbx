//! Service trait and outcome types for transport-fabric.

use smallvec::SmallVec;

/// Non-blocking service trait implemented by backend adapters.
pub trait Service {
    /// Command type accepted by the service.
    type Cmd: Send + 'static;
    /// Report type produced by the service.
    type Rep: Send + 'static;

    /// Attempts to submit a command without blocking. Defaults to `Accepted`.
    fn try_submit(&self, _cmd: &Self::Cmd) -> SubmitOutcome {
        SubmitOutcome::Accepted
    }

    /// Drains up to `max` reports without blocking. Defaults to empty.
    fn drain(&self, _max: usize) -> SmallVec<[Self::Rep; 8]> {
        SmallVec::new()
    }
}

/// Outcome returned when attempting to submit a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Command entered the queue untouched.
    Accepted,
    /// Command replaced or merged with a pending entry.
    Coalesced,
    /// Command was intentionally dropped per policy.
    Dropped,
    /// Service could not accept without blocking.
    WouldBlock,
    /// Service is closed or unhealthy.
    Closed,
}
