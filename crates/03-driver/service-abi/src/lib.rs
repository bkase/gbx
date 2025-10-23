//! Service ABI types shared between services and the scheduler.
//!
//! This crate defines the protocol boundary between the scheduler (layer 05)
//! and service implementations (layer 04), with no app-specific dependencies.

#![allow(missing_docs)]

use serde::Serialize;
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

/// Memory space targeted by inspector debug commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MemSpace {
    /// Video RAM (0x8000-0x9FFF).
    Vram,
    /// Work RAM (0xC000-0xDFFF).
    Wram,
    /// Object attribute memory (sprite table, 0xFE00-0xFE9F).
    Oam,
    /// I/O register window (0xFF00-0xFF7F).
    Io,
}

/// CPU register snapshot used by inspector view models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CpuVM {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    pub halted: bool,
}

/// PPU state snapshot exposed to inspector view models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PpuVM {
    pub ly: u8,
    pub mode: u8,
    pub stat: u8,
    pub lcdc: u8,
    pub scx: u8,
    pub scy: u8,
    pub wy: u8,
    pub wx: u8,
    pub bgp: u8,
    pub frame_ready: bool,
}

/// Timer registers snapshot exposed to inspector view models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TimersVM {
    pub div: u8,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
}

/// Minimal inspector payload emitted with snapshots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectorVMMinimal {
    pub cpu: CpuVM,
    pub ppu: PpuVM,
    pub timers: TimersVM,
    pub io: Vec<u8>,
}

/// Execution step classification for debug stepping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum StepKind {
    /// Exactly one CPU instruction executed.
    Instruction,
    /// Emulation advanced to the next frame boundary.
    Frame,
}

/// Trace information reported after stepping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceVM {
    pub last_pc: u16,
    pub disasm_line: String,
    pub cycles: u32,
}

/// Debug command variants routed to the kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugCmd {
    /// Capture a fresh snapshot of CPU/PPU/timer state.
    Snapshot { group: u16 },
    /// Fetch a memory window from the requested address space.
    MemWindow {
        group: u16,
        space: MemSpace,
        base: u16,
        len: u16,
    },
    /// Step a specific number of CPU instructions.
    StepInstruction { group: u16, count: u32 },
    /// Step exactly one frame worth of cycles.
    StepFrame { group: u16 },
}

impl DebugCmd {
    /// Returns the scheduler policy for this debug command.
    pub fn submit_policy(&self) -> SubmitPolicy {
        match self {
            DebugCmd::StepInstruction { .. } | DebugCmd::StepFrame { .. } => SubmitPolicy::Lossless,
            DebugCmd::Snapshot { .. } => SubmitPolicy::Coalesce,
            DebugCmd::MemWindow { .. } => SubmitPolicy::Lossless,
        }
    }

    /// Returns the expected number of reports produced by the command.
    pub fn expected_reports(&self) -> usize {
        match self {
            DebugCmd::Snapshot { .. } => 1,
            DebugCmd::MemWindow { .. } => 1,
            DebugCmd::StepInstruction { .. } => 1,
            DebugCmd::StepFrame { .. } => 3,
        }
    }

    /// Returns the kernel group the command targets.
    pub fn group(&self) -> u16 {
        match self {
            DebugCmd::Snapshot { group }
            | DebugCmd::MemWindow { group, .. }
            | DebugCmd::StepInstruction { group, .. }
            | DebugCmd::StepFrame { group } => *group,
        }
    }
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
    /// Inspector/debug command routed to the kernel.
    Debug(DebugCmd),
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
    /// Inspector/debug payload emitted by the kernel.
    Debug(DebugRep),
}

/// Debug report emitted by the kernel service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugRep {
    /// Snapshot payload carrying CPU/PPU/timer state.
    Snapshot(InspectorVMMinimal),
    /// Memory window bytes for a specific address space.
    MemWindow {
        space: MemSpace,
        base: u16,
        bytes: Arc<[u8]>,
    },
    /// Result of a stepping command.
    Stepped {
        kind: StepKind,
        cycles: u32,
        pc: u16,
        disasm: Option<String>,
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
