//! Transport-visible message schema frozen for rkyv serialization.
//!
//! `rkyv` expands `Archive` derives into generated archived structs that inherit `#[allow(missing_docs)]`.
//! To avoid sprinkling the allowance on every generated type mirror, we permit missing docs at
//! the module level and keep human-authored documentation on the source types concise.
#![allow(missing_docs)]
//!
//! Types in this module define the stable, archived representation of commands
//! and reports exchanged over the transport rings. Any backward-incompatible
//! change must bump the schema version and regenerate the golden fixtures under
//! `crates/tests/golden`.

use rkyv::{Archive, Serialize};
use std::string::String;
use std::vec::Vec;

/// Schema version for transport-visible messages.
pub const SCHEMA_VERSION_V1: u8 = 1;

/// Envelope tag for kernel commands.
pub const TAG_KERNEL_CMD: u8 = 0x01;
/// Envelope tag for filesystem commands.
pub const TAG_FS_CMD: u8 = 0x02;
/// Envelope tag for GPU commands.
pub const TAG_GPU_CMD: u8 = 0x03;
/// Envelope tag for audio commands.
pub const TAG_AUDIO_CMD: u8 = 0x04;

/// Envelope tag for kernel reports.
pub const TAG_KERNEL_REP: u8 = 0x11;
/// Envelope tag for filesystem reports.
pub const TAG_FS_REP: u8 = 0x12;
/// Envelope tag for GPU reports.
pub const TAG_GPU_REP: u8 = 0x13;
/// Envelope tag for audio reports.
pub const TAG_AUDIO_REP: u8 = 0x14;

/// Identifier of a display lane.
pub type LaneId = u16;
/// Unique identifier for a frame.
pub type FrameId = u64;

/// Purpose of a kernel tick affecting scheduling and policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `TickPurposeV1`."
    ),
    bytecheck()
)]
pub enum TickPurposeV1 {
    /// Tick for rendering to display (time-sensitive).
    Display,
    /// Tick for exploration/background work (can be deferred).
    Exploration,
}

/// Kernel tick command payload.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `KernelTickCmdV1`."
    ),
    bytecheck()
)]
pub struct KernelTickCmdV1 {
    /// Purpose of this tick (display or exploration).
    pub purpose: TickPurposeV1,
    /// Instruction budget for this tick.
    pub budget: u32,
}

/// Kernel command to load a ROM into the emulator.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `KernelLoadRomCmdV1`."
    ),
    bytecheck()
)]
pub struct KernelLoadRomCmdV1 {
    /// Raw ROM bytes.
    pub bytes: Vec<u8>,
}

/// Kernel command to update input state.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `KernelSetInputsCmdV1`."
    ),
    bytecheck()
)]
pub struct KernelSetInputsCmdV1 {
    /// Kernel group identifier.
    pub group: u16,
    /// Bitmask of active lanes.
    pub lanes_mask: u32,
    /// Raw joypad state.
    pub joypad: u8,
}

/// Kernel command to terminate a group.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `KernelTerminateCmdV1`."
    ),
    bytecheck()
)]
pub struct KernelTerminateCmdV1 {
    /// Kernel group identifier to terminate.
    pub group: u16,
}

/// Command sent to the kernel service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `KernelCmdV1`."),
    bytecheck()
)]
pub enum KernelCmdV1 {
    /// Execute an emulation tick.
    Tick(KernelTickCmdV1),
    /// Load a ROM into the emulator.
    LoadRom(KernelLoadRomCmdV1),
    /// Update joypad inputs.
    SetInputs(KernelSetInputsCmdV1),
    /// Terminate a kernel group.
    Terminate(KernelTerminateCmdV1),
}

/// Filesystem command to persist data.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(
        allow(missing_docs),
        doc = "Archived representation of `FsPersistCmdV1`."
    ),
    bytecheck()
)]
pub struct FsPersistCmdV1 {
    /// Storage key identifier.
    pub key: String,
    /// Payload bytes to persist.
    pub payload: Vec<u8>,
}

/// Command sent to the filesystem service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `FsCmdV1`."),
    bytecheck()
)]
pub enum FsCmdV1 {
    /// Persist data to storage.
    Persist(FsPersistCmdV1),
}

/// Command sent to the GPU service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `GpuCmdV1`."),
    attr(doc(hidden)),
    bytecheck()
)]
pub enum GpuCmdV1 {
    /// Upload a frame slot for presentation.
    UploadFrame {
        /// Display lane identifier.
        lane: LaneId,
        /// Unique frame identifier.
        frame_id: FrameId,
    },
}

/// Command sent to the audio service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `AudioCmdV1`."),
    bytecheck()
)]
pub enum AudioCmdV1 {
    /// Submit audio sample frames for playback.
    SubmitSamples {
        /// Number of sample frames to submit.
        frames: u32,
    },
}

/// Report generated by the kernel service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `KernelRepV1`."),
    bytecheck()
)]
pub enum KernelRepV1 {
    /// Tick completed successfully.
    TickDone {
        /// Purpose of the completed tick.
        purpose: TickPurposeV1,
        /// Instruction budget that was used.
        budget: u32,
    },
    /// A frame is ready on a display lane.
    LaneFrame {
        /// Display lane identifier.
        lane: LaneId,
        /// Unique frame identifier.
        frame_id: FrameId,
        /// Slot span carrying the frame payload.
        span: SlotSpanV1,
    },
    /// ROM loading completed with the number of bytes.
    RomLoaded {
        /// Size of the loaded ROM in bytes.
        bytes_len: u32,
    },
    /// Audio buffer ready for playback.
    AudioReady {
        /// Kernel group identifier.
        group: u16,
        /// Slot span carrying the audio payload.
        span: SlotSpanV1,
    },
}

/// Slot span descriptor used by kernel reports.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `SlotSpanV1`."),
    bytecheck()
)]
pub struct SlotSpanV1 {
    /// Starting slot index.
    pub start_idx: u32,
    /// Number of contiguous slots.
    pub count: u32,
}

/// Report generated by the filesystem service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `FsRepV1`."),
    bytecheck()
)]
pub enum FsRepV1 {
    /// Data persistence operation completed.
    Saved {
        /// Storage key that was saved.
        key: String,
        /// Whether the save succeeded.
        ok: bool,
    },
}

/// Report generated by the GPU service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `GpuRepV1`."),
    bytecheck()
)]
pub enum GpuRepV1 {
    /// Frame was presented to the display.
    FramePresented {
        /// Display lane identifier.
        lane: LaneId,
        /// Unique frame identifier.
        frame_id: FrameId,
    },
}

/// Report generated by the audio service.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize)]
#[rkyv(
    attr(allow(missing_docs), doc = "Archived representation of `AudioRepV1`."),
    bytecheck()
)]
pub enum AudioRepV1 {
    /// Audio frames were played successfully.
    Played {
        /// Number of sample frames played.
        frames: u32,
    },
    /// Audio buffer underrun occurred.
    Underrun,
}
