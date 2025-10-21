//! Health tracking helpers for service backpressure and recovery flows.
//!
//! The application scheduler surfaces coarse health flags so the UI can present
//! actionable feedback and the runtime can adjust follow-up work. GPU stall
//! recovery is coordinated via a short countdown window where best-effort work
//! is throttled until we observe a successful `Must` submission.

/// Latch-style health indicators exported to the UI layer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HealthFlags {
    /// True when the GPU backend has returned `WouldBlock` for critical uploads.
    pub gpu_blocked: bool,
    /// True when ancillary services report sustained backpressure.
    pub service_pressure: bool,
    /// True when any service closed unexpectedly and the app should halt.
    pub fatal: bool,
}

/// Aggregates health flags with stall-relief bookkeeping.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    /// Snapshot of coarse health flags.
    pub flags: HealthFlags,
    /// Remaining frames to throttle best-effort GPU work during recovery.
    pub stall_relief_frames: u8,
}

impl Health {
    /// Starts or extends a stall relief window and marks the GPU as blocked.
    pub fn begin_stall_relief(&mut self, frames: u8) {
        self.flags.gpu_blocked = true;
        if frames > self.stall_relief_frames {
            self.stall_relief_frames = frames;
        }
    }

    /// Decrements the stall relief window by one frame if active.
    pub fn decay_one_frame(&mut self) {
        if self.stall_relief_frames > 0 {
            self.stall_relief_frames -= 1;
        }
    }

    /// Clears the GPU stall flag after a successful submission and decays relief.
    pub fn clear_on_success(&mut self) {
        self.flags.gpu_blocked = false;
        self.decay_one_frame();
    }
}
