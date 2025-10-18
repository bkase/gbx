//! Single-slot, coalescing mailbox backed by shared memory.
//!
//! A mailbox stores the most recent payload written by a producer. Subsequent
//! writes overwrite the previous payload and report a `Coalesced` outcome while
//! remaining strictly non-blocking. The consumer always observes the latest
//! payload and acknowledgements reset the coalescing state.

use crate::msg_ring::Envelope;
use crate::region::{SharedRegion, Zeroed};
use crate::{TransportError, TransportResult};
use core::mem;
#[cfg(feature = "loom")]
use loom::sync::atomic::{AtomicU32, Ordering};
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{AtomicU32, Ordering};

const MAILBOX_ALIGNMENT: usize = 8;

#[repr(C, align(8))]
struct MailboxHeader {
    payload_capacity: u32,
    write_seq: AtomicU32,
    read_seq: AtomicU32,
    payload_len: AtomicU32,
    envelope_packed: AtomicU32,
    reserved: u32,
}

impl MailboxHeader {
    fn new(payload_capacity: u32, packed_env: u32) -> Self {
        Self {
            payload_capacity,
            write_seq: AtomicU32::new(0),
            read_seq: AtomicU32::new(0),
            payload_len: AtomicU32::new(0),
            envelope_packed: AtomicU32::new(packed_env),
            reserved: 0,
        }
    }
}

/// Outcome reported when writing into the mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxSend {
    /// Payload replaced an empty slot or a previously observed message.
    Accepted,
    /// Payload overwrote a message that had not yet been consumed.
    Coalesced,
}

/// Borrowed view of the latest payload stored in the mailbox.
///
/// The payload slice references the underlying shared region and is valid until
/// the next call mutating the mailbox.
#[derive(Debug, PartialEq, Eq)]
pub struct MailboxRecord<'a> {
    /// Envelope describing the payload tag/version/flags.
    pub envelope: Envelope,
    /// Archived payload bytes written by the producer.
    pub payload: &'a [u8],
}

/// Coalescing mailbox that retains only the newest payload.
pub struct Mailbox {
    region: SharedRegion<Zeroed>,
    default_envelope: Envelope,
}

impl Mailbox {
    /// Creates a mailbox capable of storing payloads up to `payload_capacity` bytes.
    pub fn new(payload_capacity: usize, default_envelope: Envelope) -> TransportResult<Self> {
        let aligned_capacity = align_capacity(payload_capacity);
        if aligned_capacity > u32::MAX as usize {
            return Err(TransportError::InvalidCapacity {
                requested: payload_capacity,
                minimum: MAILBOX_ALIGNMENT,
            });
        }

        let header_size = mem::size_of::<MailboxHeader>();
        let total = header_size.checked_add(aligned_capacity).ok_or({
            TransportError::InvalidCapacity {
                requested: payload_capacity,
                minimum: MAILBOX_ALIGNMENT,
            }
        })?;
        let mut region =
            SharedRegion::<Zeroed>::new_aligned_zeroed(total, MAILBOX_ALIGNMENT.max(64))?;

        let header = region.prefix_mut::<MailboxHeader>();
        *header = MailboxHeader::new(aligned_capacity as u32, pack_envelope(default_envelope));

        Ok(Self {
            region,
            default_envelope,
        })
    }

    #[cfg(target_arch = "wasm32")]
    /// Attaches to an existing mailbox living in shared linear memory.
    ///
    /// # Safety
    /// The caller must guarantee that the layout describes a region owned by the runtime and
    /// that the backing memory outlives the returned mailbox handle.
    pub unsafe fn from_wasm_layout(
        layout: crate::wasm::MailboxLayout,
        default_envelope: Envelope,
    ) -> Self {
        let header_offset = layout.header.offset as usize;
        let header_len = layout.header.length as usize;
        let data_offset = layout.data.offset as usize;
        let data_len = layout.data.length as usize;
        debug_assert!(
            data_offset == header_offset + header_len,
            "mailbox layout must place payload after header"
        );
        let total = header_len
            .checked_add(data_len)
            .expect("mailbox layout overflow");
        let alignment = MAILBOX_ALIGNMENT.max(64);
        let mut region =
            SharedRegion::<Zeroed>::from_linear_memory(total, alignment, layout.header.offset);
        let header = region.prefix_mut::<MailboxHeader>();
        header.payload_capacity = data_len as u32;
        header
            .envelope_packed
            .store(pack_envelope(default_envelope), Ordering::Relaxed);
        Self {
            region,
            default_envelope,
        }
    }

    /// Maximum payload size accepted by the mailbox.
    pub fn capacity(&self) -> usize {
        self.region.prefix::<MailboxHeader>().payload_capacity as usize
    }

    /// Writes a payload into the mailbox without blocking.
    ///
    /// Returns [`MailboxSend::Coalesced`] when the previous message is overwritten
    /// before the consumer observed it.
    pub fn try_send(
        &mut self,
        payload: &[u8],
        envelope: Option<Envelope>,
    ) -> Result<MailboxSend, TransportError> {
        if payload.len() > self.capacity() {
            return Err(TransportError::InvalidCapacity {
                requested: payload.len(),
                minimum: self.capacity(),
            });
        }

        let env = envelope.unwrap_or(self.default_envelope);

        {
            let data = self.data_slice_mut();
            copy_payload(data, payload);
        }

        let header = self.region.prefix_mut::<MailboxHeader>();
        let seq = header.write_seq.load(Ordering::Relaxed);
        let next_seq = seq.wrapping_add(1);

        header
            .payload_len
            .store(payload.len() as u32, Ordering::Release);
        header
            .envelope_packed
            .store(pack_envelope(env), Ordering::Release);
        header.write_seq.store(next_seq, Ordering::Release);

        let prev_read = header.read_seq.load(Ordering::Acquire);
        if next_seq.wrapping_sub(prev_read) > 1 {
            Ok(MailboxSend::Coalesced)
        } else {
            Ok(MailboxSend::Accepted)
        }
    }

    /// Returns the latest payload, clearing the coalescing window.
    pub fn take_latest(&mut self) -> Option<MailboxRecord<'_>> {
        let header = self.region.prefix_mut::<MailboxHeader>();
        let write_seq = header.write_seq.load(Ordering::Acquire);
        let read_seq = header.read_seq.load(Ordering::Relaxed);
        if write_seq == read_seq {
            return None;
        }

        let len = header.payload_len.load(Ordering::Acquire) as usize;
        let envelope_bits = header.envelope_packed.load(Ordering::Relaxed);
        header.read_seq.store(write_seq, Ordering::Release);
        let _ = header;

        let payload = {
            let data = self.data_slice();
            &data[..len]
        };
        let envelope = unpack_envelope(envelope_bits);
        Some(MailboxRecord { envelope, payload })
    }

    #[cfg(target_arch = "wasm32")]
    /// Describes the mailbox layout for host-side WebAssembly glue.
    pub fn wasm_layout(&self) -> crate::wasm::MailboxLayout {
        use core::convert::TryFrom;
        let region = self.region.wasm_region();
        let header_len = u32::try_from(mem::size_of::<MailboxHeader>()).expect("header fits");
        crate::wasm::MailboxLayout {
            header: crate::wasm::Region {
                offset: region.offset,
                length: header_len,
            },
            data: crate::wasm::Region {
                offset: region.offset + header_len,
                length: region
                    .length
                    .checked_sub(header_len)
                    .expect("mailbox region must exceed header"),
            },
        }
    }

    fn data_slice(&self) -> &[u8] {
        let header_size = mem::size_of::<MailboxHeader>();
        &self.region.as_slice()[header_size..]
    }

    fn data_slice_mut(&mut self) -> &mut [u8] {
        let header_size = mem::size_of::<MailboxHeader>();
        &mut self.region.as_mut_slice()[header_size..]
    }
}

fn align_capacity(capacity: usize) -> usize {
    if capacity == 0 {
        return MAILBOX_ALIGNMENT;
    }
    let rem = capacity % MAILBOX_ALIGNMENT;
    if rem == 0 {
        capacity
    } else {
        capacity + (MAILBOX_ALIGNMENT - rem)
    }
}

fn pack_envelope(env: Envelope) -> u32 {
    ((env.tag as u32) << 24) | ((env.ver as u32) << 16) | env.flags as u32
}

fn unpack_envelope(bits: u32) -> Envelope {
    Envelope {
        tag: ((bits >> 24) & 0xFF) as u8,
        ver: ((bits >> 16) & 0xFF) as u8,
        flags: (bits & 0xFFFF) as u16,
    }
}

fn copy_payload(dst: &mut [u8], payload: &[u8]) {
    let len = payload.len();
    if len == 0 {
        return;
    }
    // SAFETY: `dst` and `payload` are derived from disjoint regions, and we cap writes by
    // `len`, which has already been bounds checked.
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), dst.as_mut_ptr(), len);
    }
}
