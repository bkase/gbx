//! Error handling helpers for the transport crate.
//!
//! The transport layer intentionally keeps its error surface small: capacity
//! validation and allocation failures. Higher-level rings translate these into
//! optional results rather than propagating errors at runtime.

use std::fmt;

/// Convenience result alias for fallible transport operations.
pub type TransportResult<T, E = TransportError> = Result<T, E>;

#[derive(Debug)]
/// Errors surfaced by low-level transport helpers.
pub enum TransportError {
    /// Requested ring capacity or buffer size is below the minimum or not properly aligned.
    InvalidCapacity { requested: usize, minimum: usize },
    /// Allocation of a shared region failed for the given size/alignment pair.
    AllocationFailed { size: usize, alignment: usize },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::InvalidCapacity { requested, minimum } => {
                write!(
                    f,
                    "ring capacity {requested} must be at least {minimum} bytes and 8-byte aligned"
                )
            }
            TransportError::AllocationFailed { size, alignment } => {
                write!(
                    f,
                    "failed to allocate shared region of {size} bytes aligned to {alignment}"
                )
            }
        }
    }
}

impl std::error::Error for TransportError {}
