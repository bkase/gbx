//! Slot span descriptors for zero-copy bulk data (frames, audio).
//!
//! Services pass slot indices over reply rings to advertise ready
//! frame/audio payloads without copying the underlying buffer contents.

/// Describes a contiguous range of slots within a pool.
///
/// Typically serialized into reply messages to inform the scheduler
/// which slot indices are ready for consumption.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotSpan {
    /// Starting slot index within the pool.
    pub start_idx: u32,
    /// Number of contiguous slots in this span.
    pub count: u32,
}

impl SlotSpan {
    /// Creates a new span covering a single slot.
    pub fn single(slot_idx: u32) -> Self {
        Self {
            start_idx: slot_idx,
            count: 1,
        }
    }

    /// Creates a new span covering multiple contiguous slots.
    pub fn new(start_idx: u32, count: u32) -> Self {
        Self { start_idx, count }
    }

    /// Returns true if this span contains the given slot index.
    pub fn contains(&self, slot_idx: u32) -> bool {
        slot_idx >= self.start_idx && slot_idx < self.start_idx + self.count
    }

    /// Returns the exclusive end index (one past the last slot).
    pub fn end_idx(&self) -> u32 {
        self.start_idx + self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_slot_span() {
        let span = SlotSpan::single(5);
        assert_eq!(span.start_idx, 5);
        assert_eq!(span.count, 1);
        assert_eq!(span.end_idx(), 6);
        assert!(span.contains(5));
        assert!(!span.contains(4));
        assert!(!span.contains(6));
    }

    #[test]
    fn multi_slot_span() {
        let span = SlotSpan::new(10, 5);
        assert_eq!(span.start_idx, 10);
        assert_eq!(span.count, 5);
        assert_eq!(span.end_idx(), 15);
        assert!(span.contains(10));
        assert!(span.contains(14));
        assert!(!span.contains(9));
        assert!(!span.contains(15));
    }
}
