//! Single-producer/single-consumer message ring implementation.
//!
//! Layout (per transport spec §2.1):
//!
//! ```text
//! +----------------------+--------------------------------------------+
//! | Header (32 bytes)    | Data region (capacity bytes, 8B aligned)   |
//! +----------------------+--------------------------------------------+
//!                          Record:
//!                          [u32 total_len][u8 tag][u8 ver][u16 flags]
//!                          [rkyv archived payload ...][pad → 8 bytes]
//!                          Sentinel (wrap): total_len == 0xFFFF_FFFF
//! ```
//!
//! Producers reserve space via [`ProducerGrant`], write archived payloads
//! directly, then commit to move the head pointer. Consumers peek and pop
//! records by reading envelopes and payload slices without additional copies.

use crate::region::{RegionInit, SharedRegion};
use crate::{TransportError, TransportResult};
#[cfg(feature = "loom")]
use loom::sync::atomic::{AtomicU32, Ordering};
use std::cell::Cell;
use std::mem::size_of;
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{AtomicU32, Ordering};

const ALIGN: usize = 8;
const ENVELOPE_LEN: usize = 8;
const SENTINEL: u32 = u32::MAX;
const SENTINEL_BYTES: usize = 4;
const HEADER_SIZE: usize = size_of::<MsgRingHeader>();
const MIN_CAPACITY: usize = 64;

#[cfg(debug_assertions)]
const MSG_RING_MAGIC: u64 = 0x4D53_4752_494E_4755;
#[cfg(not(debug_assertions))]
const MSG_RING_MAGIC: u64 = 0;

#[repr(C, align(8))]
struct MsgRingHeader {
    capacity_bytes: u32,
    head_bytes: AtomicU32,
    tail_bytes: AtomicU32,
    flags_or_pad: u32,
    magic: u64,
    reserved: u64,
}

impl MsgRingHeader {
    fn new(capacity_bytes: u32) -> Self {
        Self {
            capacity_bytes,
            head_bytes: AtomicU32::new(0),
            tail_bytes: AtomicU32::new(0),
            flags_or_pad: 0,
            magic: MSG_RING_MAGIC,
            reserved: 0,
        }
    }
}

/// Metadata stored alongside each payload inside the ring.
///
/// The envelope allows consumers to identify the serialized type (`tag`),
/// enforce schema compatibility (`ver`), and carry lightweight bitflags while
/// keeping the payload naturally 8-byte aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// Application-defined discriminant used to select the rkyv schema.
    pub tag: u8,
    /// Schema epoch associated with this payload.
    pub ver: u8,
    /// Reserved bitflags that travel with the payload.
    pub flags: u16,
}

impl Envelope {
    /// Constructs an envelope with the given tag and schema version.
    pub const fn new(tag: u8, ver: u8) -> Self {
        Self { tag, ver, flags: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecordMeta {
    envelope: Envelope,
    total_len: u32,
}

/// Borrowed view of a record returned by `consumer_peek`.
#[derive(Debug, PartialEq, Eq)]
pub struct Record<'a> {
    /// Envelope describing the payload's tag and version.
    pub envelope: Envelope,
    /// Archived payload bytes ready for deserialisation.
    pub payload: &'a [u8],
}

/// Reservation object handed to producers once capacity has been secured.
pub struct ProducerGrant<'a> {
    ring: &'a mut MsgRing,
    offset: usize,
    payload_capacity: usize,
    record_len: usize,
    new_head: usize,
    envelope: Envelope,
    committed: bool,
}

impl<'a> ProducerGrant<'a> {
    /// Returns the writable slice reserved for the payload.
    pub fn payload(&mut self) -> &mut [u8] {
        let start = self.offset + ENVELOPE_LEN;
        let end = start + self.payload_capacity;
        &mut self.ring.data_slice_mut()[start..end]
    }

    /// Maximum number of bytes that can be written into this grant.
    pub fn capacity(&self) -> usize {
        self.payload_capacity
    }

    /// Overrides the envelope that will be committed with this payload.
    pub fn set_envelope(&mut self, envelope: Envelope) {
        self.envelope = envelope;
    }

    /// Finalises the reservation, advances the producer head, and returns the
    /// number of payload bytes recorded.
    pub fn commit(mut self, written: usize) -> usize {
        assert!(!self.committed, "producer grant committed twice");
        assert!(
            written <= self.payload_capacity,
            "payload length {written} exceeds reserved capacity {}",
            self.payload_capacity
        );

        self.ring.finish_producer(
            self.offset,
            self.envelope,
            written,
            self.record_len,
            self.new_head,
        );
        self.committed = true;
        written
    }
}

impl Drop for ProducerGrant<'_> {
    fn drop(&mut self) {
        debug_assert!(
            self.committed,
            "ProducerGrant dropped without commit; ring state would be inconsistent"
        );
    }
}

/// Single-producer / single-consumer message ring following the transport spec layout.
pub struct MsgRing {
    region: SharedRegion,
    capacity: u32,
    default_envelope: Envelope,
    consumer_meta: Cell<Option<RecordMeta>>,
}

impl MsgRing {
    /// Creates a new ring with `capacity_bytes` usable for payload storage.
    pub fn new(capacity_bytes: usize, default_envelope: Envelope) -> TransportResult<Self> {
        let aligned_capacity = align_up(capacity_bytes.max(MIN_CAPACITY), ALIGN);
        if aligned_capacity >= u32::MAX as usize {
            return Err(TransportError::InvalidCapacity {
                requested: capacity_bytes,
                minimum: MIN_CAPACITY,
            });
        }

        let total_bytes = HEADER_SIZE + aligned_capacity;
        let mut region = SharedRegion::new_aligned(total_bytes, ALIGN.max(64), RegionInit::Zeroed)?;

        // Initialise header in place.
        let header_ptr = region.as_mut_ptr() as *mut MsgRingHeader;
        unsafe {
            header_ptr.write(MsgRingHeader::new(aligned_capacity as u32));
        }

        Ok(Self {
            region,
            capacity: aligned_capacity as u32,
            default_envelope,
            consumer_meta: Cell::new(None),
        })
    }

    /// Returns the maximum payload capacity of the ring.
    pub fn capacity_bytes(&self) -> usize {
        self.capacity as usize
    }

    /// Attempts to reserve space for `need` archived bytes.
    pub fn try_reserve(&mut self, need: usize) -> Option<ProducerGrant<'_>> {
        self.try_reserve_with(self.default_envelope, need)
    }

    /// Variant of `try_reserve` allowing the caller to override the envelope.
    pub fn try_reserve_with(
        &mut self,
        envelope: Envelope,
        need: usize,
    ) -> Option<ProducerGrant<'_>> {
        let total_len = ENVELOPE_LEN.saturating_add(need);
        if total_len >= self.capacity_bytes() {
            return None;
        }

        let record_len = align_up(total_len, ALIGN);
        if record_len >= self.capacity_bytes() {
            return None;
        }

        let capacity = self.capacity_bytes();
        let head = self.load_head_relaxed();
        let tail = self.load_tail_acquire();

        let (write_offset, new_head) = self.reserve_offset(head, tail, record_len, capacity)?;

        Some(ProducerGrant {
            ring: self,
            offset: write_offset,
            payload_capacity: need,
            record_len,
            new_head,
            envelope,
            committed: false,
        })
    }

    /// Returns the next payload without advancing the consumer tail.
    ///
    /// The peek loop performs four steps:
    /// 1. Load producer head/tail and bail out if the ring is empty.
    /// 2. Read the record length; if it equals the sentinel value, reset the
    ///    tail to zero and continue (wrap handling).
    /// 3. Perform bounds checks and hydrate an [`Envelope`] copy for the caller.
    /// 4. Return the archived payload slice so the caller can inspect or
    ///    deserialize it without additional copying.
    ///
    /// `None` indicates the ring is currently empty.
    pub fn consumer_peek(&self) -> Option<Record<'_>> {
        let header = self.header();
        let capacity = self.capacity_bytes();
        let data = self.data_slice();

        let mut tail = header.tail_bytes.load(Ordering::Relaxed) as usize;

        loop {
            let head = header.head_bytes.load(Ordering::Acquire) as usize;
            if head == tail {
                self.consumer_meta.set(None);
                return None;
            }

            let total_len = self
                .read_u32_at(data, tail)
                .unwrap_or_else(|| panic!("corrupt len at {tail}"));

            if total_len == SENTINEL {
                header.tail_bytes.store(0, Ordering::Release);
                tail = 0;
                continue;
            }

            let total_len = total_len as usize;
            if total_len < ENVELOPE_LEN {
                panic!("invalid record length {total_len} (< envelope)");
            }

            let payload_len = total_len - ENVELOPE_LEN;
            let end = tail + total_len;
            if end > capacity {
                panic!("record overruns buffer: end={end} capacity={capacity}");
            }

            let envelope = Envelope {
                tag: data[tail + 4],
                ver: data[tail + 5],
                flags: u16::from_le_bytes([data[tail + 6], data[tail + 7]]),
            };

            let payload = &data[tail + ENVELOPE_LEN..tail + ENVELOPE_LEN + payload_len];
            self.consumer_meta.set(Some(RecordMeta {
                envelope,
                total_len: total_len as u32,
            }));
            return Some(Record { envelope, payload });
        }
    }

    /// Advances the consumer tail past the record returned by the last `consumer_peek`.
    pub fn consumer_pop_advance(&mut self) {
        let header = self.header();
        let tail = header.tail_bytes.load(Ordering::Relaxed) as usize;
        let head = header.head_bytes.load(Ordering::Acquire) as usize;

        if tail == head {
            self.consumer_meta.set(None);
            return;
        }

        let meta = self
            .consumer_meta
            .get()
            .or_else(|| self.peek_envelope_at(tail))
            .expect("consumer_pop_advance requires prior peek");

        let advance = align_up(meta.total_len as usize, ALIGN);
        let mut new_tail = tail + advance;
        let capacity = self.capacity_bytes();
        if new_tail >= capacity {
            new_tail -= capacity;
        }

        header.tail_bytes.store(new_tail as u32, Ordering::Release);
        self.consumer_meta.set(None);
    }

    /// Returns the envelope captured during the most recent peek.
    pub fn consumer_last_envelope(&self) -> Option<Envelope> {
        self.consumer_meta.get().map(|meta| meta.envelope)
    }

    fn header(&self) -> &MsgRingHeader {
        unsafe { &*(self.region.as_ptr() as *const MsgRingHeader) }
    }

    fn data_slice(&self) -> &[u8] {
        let ptr = unsafe {
            // SAFETY: `SharedRegion` allocates at least `HEADER_SIZE + capacity_bytes`
            // contiguous bytes and remains alive for the `'self` lifetime.
            self.region.as_ptr().add(HEADER_SIZE)
        };
        unsafe {
            // SAFETY: Range is fully within the allocation created above.
            std::slice::from_raw_parts(ptr, self.capacity_bytes())
        }
    }

    fn data_slice_mut(&mut self) -> &mut [u8] {
        let ptr = unsafe {
            // SAFETY: `SharedRegion` exposes a unique mutable pointer for the live allocation.
            self.region.as_mut_ptr().add(HEADER_SIZE)
        };
        unsafe {
            // SAFETY: No aliasing occurs because `MsgRing` upholds the single-producer,
            // single-consumer discipline; the slice covers the data section only.
            std::slice::from_raw_parts_mut(ptr, self.capacity_bytes())
        }
    }

    fn finish_producer(
        &mut self,
        offset: usize,
        envelope: Envelope,
        payload_len: usize,
        record_len: usize,
        new_head: usize,
    ) {
        self.write_envelope(offset, envelope, payload_len, record_len);
        self.store_head_release(new_head as u32);
    }

    fn write_envelope(
        &mut self,
        offset: usize,
        envelope: Envelope,
        payload_len: usize,
        record_len: usize,
    ) {
        let total_len = (ENVELOPE_LEN + payload_len) as u32;
        let data = self.data_slice_mut();
        let mut cursor = offset;

        data[cursor..cursor + 4].copy_from_slice(&total_len.to_le_bytes());
        cursor += 4;
        data[cursor] = envelope.tag;
        data[cursor + 1] = envelope.ver;
        data[cursor + 2..cursor + 4].copy_from_slice(&envelope.flags.to_le_bytes());
        cursor += 4;

        let payload_end = cursor + payload_len;
        let record_end = offset + record_len;
        if payload_end < record_end {
            data[payload_end..record_end].fill(0);
        }
    }

    fn load_head_relaxed(&self) -> usize {
        self.header().head_bytes.load(Ordering::Relaxed) as usize
    }

    fn load_tail_acquire(&self) -> usize {
        self.header().tail_bytes.load(Ordering::Acquire) as usize
    }

    fn store_head_release(&self, value: u32) {
        self.header().head_bytes.store(value, Ordering::Release);
    }

    fn reserve_offset(
        &mut self,
        head: usize,
        tail: usize,
        record_len: usize,
        capacity: usize,
    ) -> Option<(usize, usize)> {
        let mut head_pos = head;
        if head_pos >= capacity || tail >= capacity {
            return None;
        }

        if head_pos >= tail {
            let space_at_end = capacity - head_pos;
            if space_at_end >= record_len {
                let mut new_head = head_pos + record_len;
                if new_head == capacity {
                    new_head = 0;
                }
                if new_head == tail {
                    return None;
                }
                Some((head_pos, new_head))
            } else {
                let space_at_start = tail;
                if space_at_start <= record_len {
                    return None;
                }
                if space_at_end < SENTINEL_BYTES {
                    return None;
                }
                self.emit_sentinel(head_pos);
                head_pos = 0;
                let new_head = record_len;
                if new_head == tail {
                    return None;
                }
                Some((head_pos, new_head))
            }
        } else {
            if record_len >= (tail - head_pos) {
                return None;
            }
            let new_head = head_pos + record_len;
            Some((head_pos, new_head))
        }
    }

    fn emit_sentinel(&mut self, offset: usize) {
        let data = self.data_slice_mut();
        let end = offset + SENTINEL_BYTES;
        data[offset..end].copy_from_slice(&SENTINEL.to_le_bytes());

        let pad_end = offset + ENVELOPE_LEN;
        if pad_end <= data.len() {
            data[end..pad_end].fill(0);
        }
    }

    fn peek_envelope_at(&self, tail: usize) -> Option<RecordMeta> {
        let data = self.data_slice();
        let total = self.read_u32_at(data, tail)?;
        if total == SENTINEL {
            return None;
        }
        Some(RecordMeta {
            envelope: Envelope {
                tag: data[tail + 4],
                ver: data[tail + 5],
                flags: u16::from_le_bytes([data[tail + 6], data[tail + 7]]),
            },
            total_len: total,
        })
    }

    fn read_u32_at(&self, data: &[u8], offset: usize) -> Option<u32> {
        let end = offset.checked_add(4)?;
        if end > data.len() {
            return None;
        }
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&data[offset..end]);
        Some(u32::from_le_bytes(buf))
    }
}

fn align_up(value: usize, align: usize) -> usize {
    assert!(align.is_power_of_two());
    (value + (align - 1)) & !(align - 1)
}

#[cfg(all(test, not(feature = "loom")))]
mod tests {
    //! Unit coverage for the single-producer/single-consumer ring.
    use super::*;
    use rand::prelude::*;
    use std::collections::VecDeque;
    use std::io::{Cursor, Write};

    fn ring(capacity: usize) -> MsgRing {
        MsgRing::new(capacity, Envelope::new(0x11, 1)).expect("create ring")
    }

    fn drain_expected(ring: &mut MsgRing, expected: &mut VecDeque<Vec<u8>>) -> bool {
        if let Some(record) = ring.consumer_peek() {
            let lhs = expected.pop_front().expect("expected payload");
            assert_eq!(lhs.as_slice(), record.payload);
            ring.consumer_pop_advance();
            true
        } else {
            false
        }
    }

    #[derive(rkyv::Archive, rkyv::Serialize, Debug, PartialEq, Eq, Clone, Copy)]
    #[archive(check_bytes)]
    #[repr(u8)]
    enum SampleRep {
        Ping { value: u32 },
        Pong { value: u32 },
    }

    type ArchivedSample = <SampleRep as rkyv::Archive>::Archived;

    /// Smoke test: ensure a single record round-trips and envelope metadata is preserved.
    #[test]
    fn single_record_round_trip() {
        let mut ring = ring(256);
        let payload = b"hello sab ring".to_vec();
        let mut grant = ring.try_reserve(payload.len()).expect("reserve payload");
        grant.payload()[..payload.len()].copy_from_slice(&payload);
        assert_eq!(grant.commit(payload.len()), payload.len());

        match ring.consumer_peek() {
            Some(record) => {
                assert_eq!(record.payload, payload.as_slice());
                let envelope = ring.consumer_last_envelope().unwrap();
                assert_eq!(envelope.tag, 0x11);
                assert_eq!(envelope.ver, 1);
                assert_eq!(record.payload.len(), payload.len());
            }
            None => panic!("ring should contain payload"),
        }
        ring.consumer_pop_advance();
        assert!(ring.consumer_peek().is_none());
    }

    /// Wrap test: validate sentinel placement when the producer reaches the end of the buffer.
    #[test]
    fn sentinel_wrap_path() {
        let mut ring = ring(128);
        let block_a = vec![0xAA; 16];
        let block_b = vec![0xBB; 24];
        let block_c = vec![0xCC; 24];

        let mut grant = ring.try_reserve(block_a.len()).expect("reserve");
        grant.payload()[..block_a.len()].copy_from_slice(&block_a);
        grant.commit(block_a.len());
        assert!(ring.consumer_peek().is_some());
        ring.consumer_pop_advance();

        let mut grant = ring.try_reserve(block_b.len()).expect("reserve");
        grant.payload()[..block_b.len()].copy_from_slice(&block_b);
        grant.commit(block_b.len());

        let mut grant = ring.try_reserve(block_c.len()).expect("reserve");
        grant.payload()[..block_c.len()].copy_from_slice(&block_c);
        grant.commit(block_c.len());

        assert!(ring.consumer_peek().is_some());
        ring.consumer_pop_advance();

        match ring.consumer_peek() {
            Some(record) => assert_eq!(record.payload, block_c.as_slice()),
            None => panic!("expected payload after sentinel wrap"),
        }
    }

    /// Capacity test: filling the ring should make the next reservation fail.
    #[test]
    fn backpressure_on_full() {
        let mut ring = ring(128);
        let payload = vec![0xAB; 48];
        while let Some(mut grant) = ring.try_reserve(payload.len()) {
            grant.payload()[..payload.len()].copy_from_slice(&payload);
            grant.commit(payload.len());
        }
        assert!(ring.try_reserve(payload.len()).is_none());
    }

    /// Alignment test: payload slices should start at 8-byte boundaries for zero-copy consumers.
    #[test]
    fn payload_alignment() {
        let mut ring = ring(512);
        for i in 0..10 {
            let len = 8 + i * 7;
            let mut grant = ring.try_reserve(len).expect("reserve");
            for (idx, byte) in grant.payload().iter_mut().enumerate().take(len) {
                *byte = (idx & 0xFF) as u8;
            }
            grant.commit(len);
        }

        while let Some(record) = ring.consumer_peek() {
            assert_eq!(record.payload.as_ptr() as usize % ALIGN, 0);
            ring.consumer_pop_advance();
        }
    }

    /// Randomised stress covering wrap-around, FIFO order, and data retention.
    #[test]
    fn var_len_stress() {
        let mut ring = ring(4096);
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let mut expected = VecDeque::<Vec<u8>>::new();

        for _ in 0..10_000 {
            // Generate lengths from 1 to 2040 to avoid:
            // - Zero-length which would be a no-op
            // - Sizes that are >= half the ring capacity (can't fit due to >= check)
            let len = rng.gen_range(1..=2040);
            let mut payload = vec![0u8; len];
            rng.fill_bytes(&mut payload);

            loop {
                if let Some(mut grant) = ring.try_reserve(len) {
                    grant.payload()[..len].copy_from_slice(&payload);
                    grant.commit(len);
                    expected.push_back(payload);
                    break;
                }
                if !drain_expected(&mut ring, &mut expected) {
                    // Ring is empty but we still can't fit - this shouldn't happen
                    // with our size constraints
                    panic!("Cannot fit payload of size {len} in empty ring");
                }
            }
        }

        while drain_expected(&mut ring, &mut expected) {}
        assert!(ring.consumer_peek().is_none());
        assert!(expected.is_empty());
    }

    /// Serialization test: rkyv archives are written in-place and validate on readback.
    #[test]
    fn rkyv_round_trip() {
        use rkyv::ser::Serializer;

        let mut ring = ring(256);
        let mut expected = VecDeque::<SampleRep>::new();

        for value in 0..16 {
            let rep = if value % 2 == 0 {
                SampleRep::Ping { value }
            } else {
                SampleRep::Pong { value }
            };

            #[derive(Default)]
            struct CountingWriter {
                bytes: usize,
            }

            impl Write for CountingWriter {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    self.bytes += buf.len();
                    Ok(buf.len())
                }

                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let need = {
                let mut writer = CountingWriter::default();
                let mut serializer = rkyv::ser::serializers::WriteSerializer::new(&mut writer);
                serializer.serialize_value(&rep).unwrap();
                writer.bytes
            };

            let mut grant = loop {
                if let Some(grant) = ring.try_reserve(need) {
                    break grant;
                }
                consume_rep(&mut ring, &mut expected);
            };

            let written = {
                let payload = grant.payload();
                let mut cursor = Cursor::new(payload);
                let mut serializer = rkyv::ser::serializers::WriteSerializer::new(&mut cursor);
                serializer.serialize_value(&rep).unwrap();
                cursor.position() as usize
            };
            assert_eq!(written, need);
            grant.commit(written);
            expected.push_back(rep);
        }

        while consume_rep(&mut ring, &mut expected) {}
        assert!(expected.is_empty());
    }

    fn consume_rep(ring: &mut MsgRing, expected: &mut VecDeque<SampleRep>) -> bool {
        if let Some(record) = ring.consumer_peek() {
            let expected_rep = expected.pop_front().expect("expected sample rep");
            #[cfg(debug_assertions)]
            {
                rkyv::check_archived_root::<SampleRep>(record.payload).unwrap();
            }
            let archived = unsafe { rkyv::archived_root::<SampleRep>(record.payload) };
            match (archived, expected_rep) {
                (ArchivedSample::Ping { value }, SampleRep::Ping { value: expected }) => {
                    assert_eq!(*value, expected);
                }
                (ArchivedSample::Pong { value }, SampleRep::Pong { value: expected }) => {
                    assert_eq!(*value, expected);
                }
                _ => panic!("mismatched variant during round trip"),
            }
            ring.consumer_pop_advance();
            true
        } else {
            false
        }
    }
}

#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use loom::sync::Arc;
    use loom::thread;
    use std::cell::UnsafeCell;

    struct SharedMsgRing(UnsafeCell<MsgRing>);

    unsafe impl Send for SharedMsgRing {}
    unsafe impl Sync for SharedMsgRing {}

    impl SharedMsgRing {
        fn new(capacity: usize) -> Self {
            let ring = MsgRing::new(capacity, Envelope::new(0xAB, 1)).expect("create msg ring");
            Self(UnsafeCell::new(ring))
        }

        fn with_mut<R>(&self, f: impl FnOnce(&mut MsgRing) -> R) -> R {
            unsafe { f(&mut *self.0.get()) }
        }
    }

    /// Loom: ensures small fixed-size payloads stay consistent across interleavings.
    #[test]
    #[ignore]
    fn slow_loom_msg_ring_small_records() {
        loom::model(|| {
            let shared = Arc::new(SharedMsgRing::new(128));
            let producer = shared.clone();
            let consumer = shared.clone();

            let producer_thread = thread::spawn(move || {
                for byte in 0u8..3 {
                    loop {
                        let pushed = producer.with_mut(|ring| {
                            if let Some(mut grant) = ring.try_reserve(1) {
                                grant.payload()[0] = byte;
                                grant.commit(1);
                                true
                            } else {
                                false
                            }
                        });

                        if pushed {
                            break;
                        }
                        thread::yield_now();
                    }
                }
            });

            let consumer_thread = thread::spawn(move || {
                for expected in 0u8..3 {
                    let payload = loop {
                        let maybe = consumer.with_mut(|ring| {
                            if let Some(record) = ring.consumer_peek() {
                                let payload = record.payload.to_vec();
                                ring.consumer_pop_advance();
                                Some(payload)
                            } else {
                                None
                            }
                        });
                        if let Some(bytes) = maybe {
                            break bytes;
                        }
                        thread::yield_now();
                    };
                    assert_eq!(payload, vec![expected]);
                }
            });

            producer_thread.join().unwrap();
            consumer_thread.join().unwrap();
        });
    }

    /// Loom: exercises wrap sentinel logic under adversarial scheduling.
    #[test]
    #[ignore]
    fn slow_loom_msg_ring_wrap_pad_sequence() {
        loom::model(|| {
            let shared = Arc::new(SharedMsgRing::new(64));
            let producer = shared.clone();
            let consumer = shared.clone();

            let producer_thread = thread::spawn(move || {
                for chunk in [16usize, 20, 12] {
                    let payload = vec![chunk as u8; chunk];
                    loop {
                        let pushed = producer.with_mut(|ring| {
                            if let Some(mut grant) = ring.try_reserve(payload.len()) {
                                grant.payload()[..payload.len()].copy_from_slice(&payload);
                                grant.commit(payload.len());
                                true
                            } else {
                                false
                            }
                        });
                        if pushed {
                            break;
                        }
                        thread::yield_now();
                    }
                }
            });

            let consumer_thread = thread::spawn(move || {
                for chunk in [16usize, 20, 12] {
                    let payload = loop {
                        let maybe = consumer.with_mut(|ring| {
                            if let Some(record) = ring.consumer_peek() {
                                let payload = record.payload.to_vec();
                                ring.consumer_pop_advance();
                                Some(payload)
                            } else {
                                None
                            }
                        });
                        if let Some(bytes) = maybe {
                            break bytes;
                        }
                        thread::yield_now();
                    };
                    assert_eq!(payload.len(), chunk);
                    assert!(payload.iter().all(|b| *b == chunk as u8));
                }
            });

            producer_thread.join().unwrap();
            consumer_thread.join().unwrap();
        });
    }
}
