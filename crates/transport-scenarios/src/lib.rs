#![allow(missing_docs)]

mod checks;
mod config;
mod engine;
mod handle;
mod stats;

pub use checks::{verify_backpressure, verify_burst, verify_flood, CheckResult, DrainReport};
pub use config::{ScenarioKind, ScenarioType, TestConfig};
pub use engine::FrameScenarioEngine;
pub use handle::FabricHandle;
pub use stats::{ArcStatsSink, PtrStatsSink, ScenarioStats, StatsSink};

/// Convenience function to compute event payload bytes.
#[inline]
pub fn event_payload(frame_id: u32, slot_idx: u32) -> [u8; 8] {
    let mut payload = [0u8; 8];
    payload[..4].copy_from_slice(&frame_id.to_le_bytes());
    payload[4..].copy_from_slice(&slot_idx.to_le_bytes());
    payload
}

/// Envelope tag/version used by default frame events.
pub const EVENT_TAG: u8 = 0x13;
pub const EVENT_VER: u8 = 1;

/// Utility to update stats counters with wrapping arithmetic.
#[inline]
fn wrapping_add(base: u32, delta: u32) -> u32 {
    base.wrapping_add(delta)
}
