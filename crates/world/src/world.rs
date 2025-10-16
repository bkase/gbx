//! Minimal world state container used by reducers and tests.

use crate::types::{
    AudioCmd, AudioRep, AudioSpan, AvCmd, FollowUps, Intent, KernelCmd, KernelRep, Report,
    TickPurpose, WorkCmd,
};

/// Aggregated performance counters (placeholder for future metrics).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorldPerf {
    /// Monotonic frame identifier for the display lane.
    pub last_frame_id: u64,
    /// Accumulated audio underruns observed.
    pub audio_underruns: u64,
}

/// Health flags tracked across frames (placeholder for future recovery logic).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorldHealth {
    /// Whether the GPU backend signalled sustained backpressure.
    pub gpu_blocked: bool,
    /// Whether any service reported persistent backpressure.
    pub service_pressure: bool,
}

/// Minimal emulator world used in Wave A scaffolding.
#[derive(Debug, Clone, PartialEq)]
pub struct World {
    /// Whether the emulator loop is paused.
    pub paused: bool,
    /// Speed multiplier applied to display ticks.
    pub speed: f32,
    /// Which lane is currently presented to the user.
    pub display_lane: u16,
    /// Whether the scheduler should enqueue autopump intents.
    pub auto_pump: bool,
    /// Placeholder performance counters.
    pub perf: WorldPerf,
    /// Placeholder health flags.
    pub health: WorldHealth,
}

impl World {
    /// Creates a new world using the default initializer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the default work command for a cadence tick.
    pub fn display_tick(&self, group: u16, budget: u32) -> WorkCmd {
        WorkCmd::Kernel(KernelCmd::Tick {
            group,
            purpose: TickPurpose::Display,
            budget,
        })
    }

    /// Produces an empty set of follow-ups for convenience in tests.
    pub fn empty_follow_ups(&self) -> FollowUps {
        FollowUps::new()
    }

    /// Helper used by tests to pretend a frame was presented.
    pub fn record_present(&mut self, frame_id: u64) {
        self.perf.last_frame_id = frame_id;
    }

    /// Helper used by tests to track an audio underrun event.
    pub fn record_audio_underrun(&mut self) {
        self.perf.audio_underruns = self.perf.audio_underruns.saturating_add(1);
    }

    /// Applies a minimal report reducer hook, used only by doctests.
    pub fn reduce_report_stub(&mut self, report: Report) -> FollowUps {
        match report {
            Report::Kernel(KernelRep::LaneFrame { frame_id, .. }) => {
                self.record_present(frame_id);
            }
            Report::Audio(AudioRep::Underrun) => self.record_audio_underrun(),
            Report::Audio(AudioRep::Played { .. }) => {}
            Report::Kernel(_) | Report::Gpu(_) | Report::Fs(_) => {}
        }
        FollowUps::new()
    }

    /// Applies a minimal intent reducer hook, used only by doctests.
    pub fn reduce_intent_stub(&self, intent: Intent) -> Option<AvCmd> {
        match intent {
            Intent::PumpFrame => Some(AvCmd::Audio(AudioCmd::Submit {
                span: AudioSpan::default(),
            })),
            _ => None,
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self {
            paused: false,
            speed: 1.0,
            display_lane: 0,
            auto_pump: true,
            perf: WorldPerf::default(),
            health: WorldHealth::default(),
        }
    }
}
