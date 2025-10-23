//! Core command, intent, and report types shared across the emulator world.
#![allow(missing_docs)]
//!
//! These shapes mirror the Wave A contract documented in `docs/architecture.md`
//! so that frontends, schedulers, and services can compile against stable
//! message definitions while higher layers are still under construction.

use smallvec::SmallVec;
use std::sync::Arc;

// Re-export service ABI types that world uses
pub use service_abi::{
    AudioCmd, AudioRep, AudioSpan, CpuVM, DebugCmd, DebugRep, FrameSpan, FsCmd, FsRep, GpuCmd,
    GpuRep, InspectorVMMinimal, KernelCmd, KernelRep, MemSpace, PpuVM, SlotSpan, StepKind,
    SubmitOutcome, SubmitPolicy, TickPurpose, TimersVM, TraceVM,
};

/// Priority level for intent scheduling (P0 is highest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentPriority {
    /// Highest priority, reserved for latency-critical actions.
    P0,
    /// Medium priority for frame progression and UI-affecting intents.
    P1,
    /// Lowest priority for maintenance or background intents.
    P2,
}

/// Intent emitted by frontends, reducers, or deferred follow-ups.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// Advance the emulation loop by (at most) one frame.
    PumpFrame,
    /// Toggle the emulation pause flag.
    TogglePause,
    /// Adjust the emulation speed multiplier.
    SetSpeed(f32),
    /// Load ROM bytes into a kernel group.
    LoadRom {
        /// Kernel group identifier.
        group: u16,
        /// Raw ROM bytes.
        bytes: Arc<[u8]>,
    },
    /// Select which display lane should be presented.
    SelectDisplayLane(u16),
    /// Request a debug snapshot for the given kernel group.
    DebugSnapshot(u16),
    /// Request a memory window from the kernel.
    DebugMem {
        group: u16,
        space: MemSpace,
        base: u16,
        len: u16,
    },
    /// Step a number of CPU instructions.
    DebugStepInstruction { group: u16, count: u32 },
    /// Step the kernel forward by exactly one frame.
    DebugStepFrame(u16),
}

impl Intent {
    /// Returns the scheduler priority for this intent.
    pub fn priority(&self) -> IntentPriority {
        match self {
            Intent::PumpFrame => IntentPriority::P1,
            Intent::TogglePause => IntentPriority::P0,
            Intent::SetSpeed(_) => IntentPriority::P0,
            Intent::LoadRom { .. } => IntentPriority::P0,
            Intent::SelectDisplayLane(_) => IntentPriority::P1,
            Intent::DebugSnapshot(_) => IntentPriority::P1,
            Intent::DebugMem { .. } => IntentPriority::P1,
            Intent::DebugStepInstruction { .. } => IntentPriority::P0,
            Intent::DebugStepFrame(_) => IntentPriority::P0,
        }
    }
}

/// Work command routed through the scheduler during phase A.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkCmd {
    /// Command that targets the kernel service.
    Kernel(KernelCmd),
    /// Command that targets the filesystem service.
    Fs(FsCmd),
}

impl WorkCmd {
    /// Default submission policy for this command.
    pub fn default_policy(&self) -> SubmitPolicy {
        match self {
            WorkCmd::Kernel(KernelCmd::Tick { purpose, .. }) => match purpose {
                TickPurpose::Display => SubmitPolicy::Coalesce,
                TickPurpose::Exploration => SubmitPolicy::BestEffort,
            },
            WorkCmd::Kernel(KernelCmd::LoadRom { .. }) => SubmitPolicy::Lossless,
            WorkCmd::Kernel(KernelCmd::SetInputs { .. }) => SubmitPolicy::Lossless,
            WorkCmd::Kernel(KernelCmd::Terminate { .. }) => SubmitPolicy::Lossless,
            WorkCmd::Kernel(KernelCmd::Debug(cmd)) => cmd.submit_policy(),
            WorkCmd::Fs(FsCmd::Persist { .. }) => SubmitPolicy::Coalesce,
        }
    }
}

/// Immediate audio/video command produced during report reduction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AvCmd {
    /// GPU-bound command.
    Gpu(GpuCmd),
    /// Audio-bound command.
    Audio(AudioCmd),
}

impl AvCmd {
    /// Default submission policy for this AV command.
    pub fn default_policy(&self, display_lane: u16) -> SubmitPolicy {
        match self {
            AvCmd::Gpu(GpuCmd::UploadFrame { lane, .. }) => {
                if *lane == display_lane {
                    SubmitPolicy::Must
                } else {
                    SubmitPolicy::BestEffort
                }
            }
            AvCmd::Audio(_) => SubmitPolicy::Must,
        }
    }
}

/// Report emitted by backend services during phase B.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// Kernel-originated report.
    Kernel(KernelRep),
    /// GPU-originated report.
    Gpu(GpuRep),
    /// Audio-originated report.
    Audio(AudioRep),
    /// Filesystem-originated report.
    Fs(FsRep),
}

/// Follow-up actions produced while reducing reports.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowUps {
    /// AV commands that should be submitted immediately (phase B).
    pub immediate_av: SmallVec<[AvCmd; 8]>,
    /// Intents to enqueue for the next frame (phase A).
    pub deferred_intents: SmallVec<[(IntentPriority, Intent); 8]>,
}

impl FollowUps {
    /// Creates an empty set of follow-ups.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an immediate AV command to be submitted in phase B.
    pub fn push_immediate_av(&mut self, cmd: AvCmd) {
        self.immediate_av.push(cmd);
    }

    /// Adds a deferred intent for the next phase A pull.
    pub fn push_deferred_intent(&mut self, priority: IntentPriority, intent: Intent) {
        self.deferred_intents.push((priority, intent));
    }
}

impl Default for FollowUps {
    fn default() -> Self {
        Self {
            immediate_av: SmallVec::new(),
            deferred_intents: SmallVec::new(),
        }
    }
}
