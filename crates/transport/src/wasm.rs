//! Layout descriptors exposed when targeting `wasm32`.
//!
//! These lightweight structs capture byte offsets and lengths for regions
//! inside the shared linear memory so that host glue (JS/TS) can materialise
//! typed array views without copying.

/// Byte-range descriptor within the shared linear memory.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// Offset in bytes from the start of the shared memory.
    pub offset: u32,
    /// Length in bytes for the region.
    pub length: u32,
}

/// Layout metadata for a message ring.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MsgRingLayout {
    /// Header region containing the atomic cursors and metadata.
    pub header: Region,
    /// Data region that stores archived payloads.
    pub data: Region,
    /// Capacity of the data region in bytes.
    pub capacity_bytes: u32,
}

/// Layout metadata for a coalescing mailbox.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MailboxLayout {
    /// Header containing cursors and envelope metadata.
    pub header: Region,
    /// Data region storing the payload bytes.
    pub data: Region,
}

/// Layout metadata for an index ring (free/ready pools).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IndexRingLayout {
    /// Header region holding the atomic cursors.
    pub header: Region,
    /// Entries region holding `u32` slot indices.
    pub entries: Region,
    /// Total number of entries managed by the ring.
    pub capacity: u32,
}

/// Layout metadata for a slot pool (frame/audio).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SlotPoolLayout {
    /// Region covering all slot payload bytes.
    pub slots: Region,
    /// Size of each slot in bytes.
    pub slot_size: u32,
    /// Number of slots managed by the pool.
    pub slot_count: u32,
    /// Layout of the free index ring.
    pub free: IndexRingLayout,
    /// Layout of the ready index ring.
    pub ready: IndexRingLayout,
}
