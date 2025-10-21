//! Layout descriptors exposed when targeting `wasm32`.
//!
//! These lightweight structs capture byte offsets and lengths for regions
//! inside the shared linear memory so that host glue (JS/TS) can materialise
//! typed array views without copying.

use rkyv::{Archive, Deserialize, Serialize};

/// Trait for converting layout types (native or archived) into native layouts.
/// This allows `from_wasm_layout` to accept both regular and archived layouts.
pub trait IntoNativeLayout {
    /// The native layout type this converts to.
    type Native;
    /// Converts this layout (native or archived) into its native form.
    fn into_native(self) -> Self::Native;
}

/// Byte-range descriptor within the shared linear memory.
#[repr(C)]
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
pub struct Region {
    /// Offset in bytes from the start of the shared memory.
    pub offset: u32,
    /// Length in bytes for the region.
    pub length: u32,
}

impl IntoNativeLayout for Region {
    type Native = Region;
    fn into_native(self) -> Self::Native {
        self
    }
}

impl IntoNativeLayout for &ArchivedRegion {
    type Native = Region;
    fn into_native(self) -> Self::Native {
        Region {
            offset: self.offset.to_native(),
            length: self.length.to_native(),
        }
    }
}

/// Layout metadata for a message ring.
#[repr(C)]
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
pub struct MsgRingLayout {
    /// Header region containing the atomic cursors and metadata.
    pub header: Region,
    /// Data region that stores archived payloads.
    pub data: Region,
    /// Capacity of the data region in bytes.
    pub capacity_bytes: u32,
}

impl IntoNativeLayout for MsgRingLayout {
    type Native = MsgRingLayout;
    fn into_native(self) -> Self::Native {
        self
    }
}

impl IntoNativeLayout for &ArchivedMsgRingLayout {
    type Native = MsgRingLayout;
    fn into_native(self) -> Self::Native {
        MsgRingLayout {
            header: (&self.header).into_native(),
            data: (&self.data).into_native(),
            capacity_bytes: self.capacity_bytes.to_native(),
        }
    }
}

/// Layout metadata for a coalescing mailbox.
#[repr(C)]
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
pub struct MailboxLayout {
    /// Header containing cursors and envelope metadata.
    pub header: Region,
    /// Data region storing the payload bytes.
    pub data: Region,
}

/// Layout metadata for an index ring (free/ready pools).
#[repr(C)]
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
pub struct IndexRingLayout {
    /// Header region holding the atomic cursors.
    pub header: Region,
    /// Entries region holding `u32` slot indices.
    pub entries: Region,
    /// Total number of entries managed by the ring.
    pub capacity: u32,
}

impl IntoNativeLayout for IndexRingLayout {
    type Native = IndexRingLayout;
    fn into_native(self) -> Self::Native {
        self
    }
}

impl IntoNativeLayout for &ArchivedIndexRingLayout {
    type Native = IndexRingLayout;
    fn into_native(self) -> Self::Native {
        IndexRingLayout {
            header: (&self.header).into_native(),
            entries: (&self.entries).into_native(),
            capacity: self.capacity.to_native(),
        }
    }
}

/// Layout metadata for a slot pool (frame/audio).
#[repr(C)]
#[derive(Clone, Copy, Debug, Archive, Deserialize, Serialize)]
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

impl IntoNativeLayout for SlotPoolLayout {
    type Native = SlotPoolLayout;
    fn into_native(self) -> Self::Native {
        self
    }
}

impl IntoNativeLayout for &ArchivedSlotPoolLayout {
    type Native = SlotPoolLayout;
    fn into_native(self) -> Self::Native {
        SlotPoolLayout {
            slots: (&self.slots).into_native(),
            slot_size: self.slot_size.to_native(),
            slot_count: self.slot_count.to_native(),
            free: (&self.free).into_native(),
            ready: (&self.ready).into_native(),
        }
    }
}
