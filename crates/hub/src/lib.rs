//! Game Boy emulator service hub orchestration layer.
//!
//! This crate provides the core orchestration types for coordinating between
//! the UI layer (intents) and backend services (kernel, filesystem, GPU, audio).
//! It implements a policy-based submission system with priority queues and
//! report aggregation.

use anyhow::{anyhow, Result};
use smallvec::SmallVec;
use std::sync::Arc;

/// Default budget for processing intents per tick.
pub const DEFAULT_INTENT_BUDGET: usize = 3;

/// Default budget for draining reports from services per tick.
pub const DEFAULT_REPORT_BUDGET: usize = 32;

/// Policy for how commands should be submitted to service queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitPolicy {
    /// Command must be submitted; block if queue is full.
    Must,
    /// Coalesce with or replace pending similar commands.
    Coalesce,
    /// Submit if space available, otherwise drop.
    BestEffort,
    /// Command must eventually be processed; never drop.
    Lossless,
}

/// Result of attempting to submit a command to a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Command was accepted into the queue.
    Accepted,
    /// Command was merged with an existing queued command.
    Coalesced,
    /// Command was dropped due to capacity or policy.
    Dropped,
    /// Queue is full and policy doesn't allow blocking or dropping.
    WouldBlock,
    /// Service has been shut down.
    Closed,
}

/// Purpose of a kernel tick, affecting scheduling and policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPurpose {
    /// Tick for rendering to display (time-sensitive).
    Display,
    /// Tick for exploration/background work (can be deferred).
    Exploration,
}

/// Priority level for intent processing (P0 = highest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentPriority {
    /// Highest priority (e.g., ROM loading, pause).
    P0,
    /// Medium priority (e.g., frame pump, display lane selection).
    P1,
    /// Lowest priority (reserved for future use).
    P2,
}

impl IntentPriority {
    /// Returns the numeric index for this priority (0 = highest).
    pub fn index(self) -> usize {
        match self {
            IntentPriority::P0 => 0,
            IntentPriority::P1 => 1,
            IntentPriority::P2 => 2,
        }
    }
}

/// User-facing intent that may trigger one or more backend commands.
#[derive(Clone, Debug)]
pub enum Intent {
    /// Request the emulator to advance by one frame.
    PumpFrame,
    /// Toggle emulation pause state.
    TogglePause,
    /// Set emulation speed multiplier.
    SetSpeed(f32),
    /// Load a ROM from raw bytes.
    LoadRom {
        /// Raw ROM file bytes.
        bytes: Arc<[u8]>,
    },
    /// Select which display lane to present.
    SelectDisplayLane(u16),
}

impl Intent {
    /// Returns the processing priority for this intent.
    pub fn priority(&self) -> IntentPriority {
        match self {
            Intent::PumpFrame => IntentPriority::P1,
            Intent::TogglePause => IntentPriority::P0,
            Intent::SetSpeed(_) => IntentPriority::P0,
            Intent::LoadRom { .. } => IntentPriority::P0,
            Intent::SelectDisplayLane(_) => IntentPriority::P1,
        }
    }
}

/// Command sent to the emulator kernel service.
#[derive(Clone, Debug)]
pub enum KernelCmd {
    /// Execute emulation tick with given purpose and cycle budget.
    Tick {
        /// Purpose of this tick (display or exploration).
        purpose: TickPurpose,
        /// Instruction budget for this tick.
        budget: u32,
    },
    /// Load a ROM into the emulator.
    LoadRom {
        /// Raw ROM file bytes.
        bytes: Arc<[u8]>,
    },
}

/// Command sent to the filesystem service.
#[derive(Clone, Debug)]
pub enum FsCmd {
    /// Persist data to storage.
    Persist {
        /// Storage key identifier.
        key: String,
        /// Data payload to persist.
        payload: Arc<[u8]>,
    },
}

/// Work command targeting kernel or filesystem services.
#[derive(Clone, Debug)]
pub enum WorkCmd {
    /// Command for the kernel service.
    Kernel(KernelCmd),
    /// Command for the filesystem service.
    Fs(FsCmd),
}

impl WorkCmd {
    /// Returns the default submission policy for this command.
    pub fn default_policy(&self) -> SubmitPolicy {
        match self {
            WorkCmd::Kernel(KernelCmd::Tick { purpose, .. }) => match purpose {
                TickPurpose::Display => SubmitPolicy::Coalesce,
                TickPurpose::Exploration => SubmitPolicy::BestEffort,
            },
            WorkCmd::Kernel(KernelCmd::LoadRom { .. }) => SubmitPolicy::Lossless,
            WorkCmd::Fs(FsCmd::Persist { key, .. }) => {
                if key == "manual-save" {
                    SubmitPolicy::Lossless
                } else {
                    SubmitPolicy::Coalesce
                }
            }
        }
    }
}

/// Command sent to the GPU service.
#[derive(Clone, Debug)]
pub enum GpuCmd {
    /// Upload a frame to be presented.
    UploadFrame {
        /// Display lane identifier.
        lane: u16,
        /// Unique frame identifier.
        frame_id: u64,
    },
}

/// Command sent to the audio service.
#[derive(Clone, Debug)]
pub enum AudioCmd {
    /// Submit audio sample frames for playback.
    SubmitSamples {
        /// Number of sample frames to submit.
        frames: usize,
    },
}

/// Audio/video command targeting GPU or audio services.
#[derive(Clone, Debug)]
pub enum AvCmd {
    /// Command for the GPU service.
    Gpu(GpuCmd),
    /// Command for the audio service.
    Audio(AudioCmd),
}

impl AvCmd {
    /// Returns the default submission policy for this command.
    pub fn default_policy(&self) -> SubmitPolicy {
        match self {
            AvCmd::Gpu(_) => SubmitPolicy::Must,
            AvCmd::Audio(_) => SubmitPolicy::Must,
        }
    }
}

/// Report from the kernel service.
#[derive(Clone, Debug)]
pub enum KernelRep {
    /// Tick completed successfully.
    TickDone {
        /// Purpose of the completed tick.
        purpose: TickPurpose,
        /// Instruction budget that was used.
        budget: u32,
    },
    /// A frame is ready on a display lane.
    LaneFrame {
        /// Display lane identifier.
        lane: u16,
        /// Unique frame identifier.
        frame_id: u64,
    },
    /// ROM loading completed.
    RomLoaded {
        /// Size of the loaded ROM in bytes.
        bytes_len: usize,
    },
}

/// Report from the filesystem service.
#[derive(Clone, Debug)]
pub enum FsRep {
    /// Data persistence operation completed.
    Saved {
        /// Storage key that was saved.
        key: String,
        /// Whether the save succeeded.
        ok: bool,
    },
}

/// Report from the GPU service.
#[derive(Clone, Debug)]
pub enum GpuRep {
    /// Frame was presented to the display.
    FramePresented {
        /// Display lane identifier.
        lane: u16,
        /// Unique frame identifier.
        frame_id: u64,
    },
}

/// Report from the audio service.
#[derive(Clone, Debug)]
pub enum AudioRep {
    /// Audio frames were played successfully.
    Played {
        /// Number of sample frames played.
        frames: usize,
    },
    /// Audio buffer underrun occurred.
    Underrun,
}

/// Report from any service in the hub.
#[derive(Clone, Debug)]
pub enum Report {
    /// Report from the kernel service.
    Kernel(KernelRep),
    /// Report from the filesystem service.
    Fs(FsRep),
    /// Report from the GPU service.
    Gpu(GpuRep),
    /// Report from the audio service.
    Audio(AudioRep),
}

/// Follow-up actions triggered by processing a report.
#[derive(Clone, Debug)]
pub struct FollowUps {
    /// AV commands to submit immediately.
    pub immediate_av: SmallVec<[AvCmd; 8]>,
    /// Intents to defer for later processing.
    pub deferred_intents: SmallVec<[(IntentPriority, Intent); 8]>,
}

impl FollowUps {
    /// Creates an empty set of follow-ups.
    pub fn new() -> Self {
        Self {
            immediate_av: SmallVec::new(),
            deferred_intents: SmallVec::new(),
        }
    }

    /// Adds an immediate AV command to execute.
    pub fn push_av(&mut self, cmd: AvCmd) {
        self.immediate_av.push(cmd);
    }

    /// Adds a deferred intent to process later.
    pub fn push_deferred(&mut self, priority: IntentPriority, intent: Intent) {
        self.deferred_intents.push((priority, intent));
    }
}

impl Default for FollowUps {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for converting intents into work commands.
pub trait IntentReducer {
    /// Reduces an intent into zero or more work commands.
    fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]>;
}

/// Trait for processing reports and generating follow-up actions.
pub trait ReportReducer {
    /// Processes a report and returns follow-up actions.
    fn reduce_report(&mut self, report: Report) -> FollowUps;
}

/// Trait for backend services that accept commands and produce reports.
pub trait Service: Send + Sync {
    /// Command type accepted by this service.
    type Command: Send + 'static;
    /// Report type produced by this service.
    type Report: Send + 'static;

    /// Attempts to submit a command with the given policy.
    fn try_submit(&self, cmd: Self::Command, policy: SubmitPolicy) -> SubmitOutcome;
    /// Attempts to poll a report from this service.
    fn try_poll_report(&self) -> Option<Self::Report>;
}

/// Type alias for a kernel service handle.
pub type KernelServiceHandle = Arc<dyn Service<Command = KernelCmd, Report = KernelRep>>;
/// Type alias for a filesystem service handle.
pub type FsServiceHandle = Arc<dyn Service<Command = FsCmd, Report = FsRep>>;
/// Type alias for a GPU service handle.
pub type GpuServiceHandle = Arc<dyn Service<Command = GpuCmd, Report = GpuRep>>;
/// Type alias for an audio service handle.
pub type AudioServiceHandle = Arc<dyn Service<Command = AudioCmd, Report = AudioRep>>;

/// Centralized hub coordinating all backend services.
#[derive(Clone)]
pub struct ServicesHub {
    kernel: KernelServiceHandle,
    fs: FsServiceHandle,
    gpu: GpuServiceHandle,
    audio: AudioServiceHandle,
}

impl ServicesHub {
    /// Creates a new builder for constructing a services hub.
    pub fn builder() -> ServicesHubBuilder {
        ServicesHubBuilder::new()
    }

    /// Attempts to submit a work command to the appropriate service.
    pub fn try_submit_work(&self, cmd: WorkCmd) -> SubmitOutcome {
        let policy = cmd.default_policy();
        match cmd {
            WorkCmd::Kernel(inner) => self.kernel.try_submit(inner, policy),
            WorkCmd::Fs(inner) => self.fs.try_submit(inner, policy),
        }
    }

    /// Attempts to submit an AV command to the appropriate service.
    pub fn try_submit_av(&self, cmd: AvCmd) -> SubmitOutcome {
        let policy = cmd.default_policy();
        match cmd {
            AvCmd::Gpu(inner) => self.gpu.try_submit(inner, policy),
            AvCmd::Audio(inner) => self.audio.try_submit(inner, policy),
        }
    }

    /// Drains reports from all services up to the given budget.
    pub fn drain_reports(&self, budget: usize) -> Vec<Report> {
        if budget == 0 {
            return Vec::new();
        }

        let mut remaining = budget;
        let mut out = Vec::with_capacity(budget);
        let mut progressed = true;

        while remaining > 0 && progressed {
            progressed = false;

            if remaining > 0 {
                if let Some(rep) = self.kernel.try_poll_report() {
                    out.push(Report::Kernel(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.fs.try_poll_report() {
                    out.push(Report::Fs(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.gpu.try_poll_report() {
                    out.push(Report::Gpu(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.audio.try_poll_report() {
                    out.push(Report::Audio(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }
        }

        out
    }
}

/// Builder for constructing a ServicesHub with all required services.
pub struct ServicesHubBuilder {
    kernel: Option<KernelServiceHandle>,
    fs: Option<FsServiceHandle>,
    gpu: Option<GpuServiceHandle>,
    audio: Option<AudioServiceHandle>,
}

impl ServicesHubBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self {
            kernel: None,
            fs: None,
            gpu: None,
            audio: None,
        }
    }

    /// Sets the kernel service handle.
    pub fn kernel(mut self, svc: KernelServiceHandle) -> Self {
        self.kernel = Some(svc);
        self
    }

    /// Sets the filesystem service handle.
    pub fn fs(mut self, svc: FsServiceHandle) -> Self {
        self.fs = Some(svc);
        self
    }

    /// Sets the GPU service handle.
    pub fn gpu(mut self, svc: GpuServiceHandle) -> Self {
        self.gpu = Some(svc);
        self
    }

    /// Sets the audio service handle.
    pub fn audio(mut self, svc: AudioServiceHandle) -> Self {
        self.audio = Some(svc);
        self
    }

    /// Builds the ServicesHub, returning an error if any service is missing.
    pub fn build(self) -> Result<ServicesHub> {
        Ok(ServicesHub {
            kernel: self
                .kernel
                .ok_or_else(|| anyhow!("missing kernel service"))?,
            fs: self.fs.ok_or_else(|| anyhow!("missing fs service"))?,
            gpu: self.gpu.ok_or_else(|| anyhow!("missing gpu service"))?,
            audio: self.audio.ok_or_else(|| anyhow!("missing audio service"))?,
        })
    }
}

impl Default for ServicesHubBuilder {
    fn default() -> Self {
        Self::new()
    }
}
