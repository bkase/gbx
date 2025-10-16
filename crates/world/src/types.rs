//! Core command, intent, and report types shared across the emulator world.
//!
//! These shapes mirror the Wave A contract documented in `docs/architecture.md`
//! so that frontends, schedulers, and services can compile against stable
//! message definitions while higher layers are still under construction.

use smallvec::SmallVec;
use std::{path::PathBuf, sync::Arc};

/// Purpose for a kernel tick invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPurpose {
    /// Time-critical tick that advances the display lane.
    Display,
    /// Background exploration tick that can be deferred under load.
    Exploration,
}

/// Policy describing how the scheduler should handle backpressure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitPolicy {
    /// Command must be submitted immediately; failure is surfaced.
    Must,
    /// Replace or merge with pending work of the same kind.
    Coalesce,
    /// Drop when queues are congested.
    BestEffort,
    /// Never drop; scheduler will retry on subsequent frames.
    Lossless,
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

/// Image span produced by the kernel for presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameSpan {
    /// Frame width in pixels.
    pub width: u16,
    /// Frame height in pixels.
    pub height: u16,
    /// Raw RGBA pixels (row-major).
    pub pixels: Arc<[u8]>,
}

impl FrameSpan {
    /// Creates an empty frame span placeholder.
    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Arc::from([]),
        }
    }
}

impl Default for FrameSpan {
    fn default() -> Self {
        Self::empty()
    }
}

/// Audio buffer produced by the kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSpan {
    /// Interleaved PCM samples.
    pub samples: Arc<[i16]>,
    /// Number of channels in the buffer.
    pub channels: u8,
    /// Sample rate in Hertz.
    pub sample_rate_hz: u32,
}

impl AudioSpan {
    /// Creates an empty audio span placeholder.
    pub fn empty() -> Self {
        Self {
            samples: Arc::from([]),
            channels: 2,
            sample_rate_hz: 48_000,
        }
    }
}

impl Default for AudioSpan {
    fn default() -> Self {
        Self::empty()
    }
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
}

/// Command directed at the kernel service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelCmd {
    /// Advance emulation for a group using the supplied cycle budget.
    Tick {
        /// Kernel group identifier.
        group: u16,
        /// Purpose of this tick, informing scheduling policy decisions.
        purpose: TickPurpose,
        /// Cycle budget granted to this tick.
        budget: u32,
    },
    /// Load ROM bytes into the specified group.
    LoadRom {
        /// Kernel group identifier.
        group: u16,
        /// ROM payload.
        bytes: Arc<[u8]>,
    },
    /// Update joypad inputs for a group.
    SetInputs {
        /// Kernel group identifier.
        group: u16,
        /// Bitmask indicating active lanes.
        lanes_mask: u32,
        /// Raw joypad state.
        joypad: u8,
    },
    /// Terminate a kernel group.
    Terminate {
        /// Kernel group identifier to terminate.
        group: u16,
    },
}

/// Command directed at the filesystem service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsCmd {
    /// Persist a payload to disk.
    Persist {
        /// Target path for the payload.
        path: PathBuf,
        /// Serialized payload bytes.
        bytes: Arc<[u8]>,
    },
}

/// Command directed at the GPU service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuCmd {
    /// Upload a frame span for presentation.
    UploadFrame {
        /// Source lane that produced the frame.
        lane: u16,
        /// Frame contents.
        span: FrameSpan,
    },
}

/// Command directed at the audio service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioCmd {
    /// Submit an audio span for playback.
    Submit {
        /// Audio buffer to submit.
        span: AudioSpan,
    },
}

/// Work command routed through the scheduler during phase A.
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

/// Kernel report variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelRep {
    /// Tick completed with the supplied purpose and lane mask.
    TickDone {
        /// Kernel group identifier.
        group: u16,
        /// Lanes that participated in the tick.
        lanes_mask: u32,
        /// Number of CPU cycles executed during the tick.
        cycles_done: u32,
    },
    /// A frame is ready for a lane.
    LaneFrame {
        /// Kernel group identifier.
        group: u16,
        /// Lane identifier associated with the frame.
        lane: u16,
        /// Contents of the frame.
        span: FrameSpan,
        /// Monotonically increasing frame identifier.
        frame_id: u64,
    },
    /// Audio buffer ready for playback.
    AudioReady {
        /// Kernel group identifier.
        group: u16,
        /// Audio contents produced by the kernel.
        span: AudioSpan,
    },
    /// Thumbnail or debug data dropped due to pressure.
    DroppedThumb {
        /// Kernel group identifier.
        group: u16,
        /// Count of dropped thumbnails.
        count: u32,
    },
}

/// GPU report variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuRep {
    /// A frame reached the swap chain.
    FrameShown {
        /// Presented lane.
        lane: u16,
        /// Frame identifier that was completed.
        frame_id: u64,
    },
}

/// Audio report variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioRep {
    /// Audio buffer played successfully.
    Played {
        /// Number of frames consumed.
        frames: usize,
    },
    /// Audio underrun detected by the backend.
    Underrun,
}

/// Filesystem report variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsRep {
    /// Persistence operation completed.
    Saved {
        /// Path that was persisted.
        path: PathBuf,
        /// Whether the persistence succeeded.
        ok: bool,
    },
}

/// Report emitted by backend services during phase B.
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
    /// AV commands that should be submitted immediately (phase B).
    pub immediate_av: SmallVec<[AvCmd; 8]>,
    /// Intents to enqueue for the next frame (phase A).
    pub deferred_intents: SmallVec<[Intent; 8]>,
}

impl FollowUps {
    /// Creates an empty set of follow-ups.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an immediate AV command to be submitted in phase B.
    pub fn push_immediate_av(&mut self, cmd: AvCmd) {
        self.immediate_av.push(cmd);
    }

    /// Adds a deferred intent for the next phase A pull.
    pub fn push_deferred_intent(&mut self, intent: Intent) {
        self.deferred_intents.push(intent);
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
