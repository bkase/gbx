//! Service ABI types shared between services and the scheduler.
//!
//! This crate defines the protocol boundary between the scheduler (layer 05)
//! and service implementations (layer 04), with no app-specific dependencies.

#![allow(missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

// Re-export core service types from transport-fabric
pub use transport_fabric::{Service, SubmitOutcome};

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

/// Describes a contiguous range of slots within a transport pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotSpan {
    /// Starting slot index within the pool.
    pub start_idx: u32,
    /// Number of contiguous slots in this span.
    pub count: u32,
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
    /// Optional slot span referencing transport-managed memory.
    pub slot_span: Option<SlotSpan>,
}

impl FrameSpan {
    /// Creates an empty frame span placeholder.
    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Arc::from([]),
            slot_span: None,
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
    /// Optional slot span referencing transport-managed memory.
    pub slot_span: Option<SlotSpan>,
}

impl AudioSpan {
    /// Creates an empty audio span placeholder.
    pub fn empty() -> Self {
        Self {
            samples: Arc::from([]),
            channels: 2,
            sample_rate_hz: 48_000,
            slot_span: None,
        }
    }
}

impl Default for AudioSpan {
    fn default() -> Self {
        Self::empty()
    }
}

/// Purpose for a kernel tick invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPurpose {
    /// Time-critical tick that advances the display lane.
    Display,
    /// Background exploration tick that can be deferred under load.
    Exploration,
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
    /// ROM loading completed for a group.
    RomLoaded {
        /// Kernel group identifier.
        group: u16,
        /// Size of the loaded ROM in bytes.
        bytes_len: usize,
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

/// Command directed at the audio service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioCmd {
    /// Submit an audio span for playback.
    Submit {
        /// Audio buffer to submit.
        span: AudioSpan,
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

// Service handle type aliases for convenience
// Note: Service trait is re-exported from transport-fabric above

/// Handle to the kernel service implementation.
pub type KernelServiceHandle = Arc<dyn Service<Cmd = KernelCmd, Rep = KernelRep> + Send + Sync>;
/// Handle to the filesystem service implementation.
pub type FsServiceHandle = Arc<dyn Service<Cmd = FsCmd, Rep = FsRep> + Send + Sync>;
/// Handle to the GPU service implementation.
pub type GpuServiceHandle = Arc<dyn Service<Cmd = GpuCmd, Rep = GpuRep> + Send + Sync>;
/// Handle to the audio service implementation.
pub type AudioServiceHandle = Arc<dyn Service<Cmd = AudioCmd, Rep = AudioRep> + Send + Sync>;
