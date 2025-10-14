//! Fixed-size slot pools backed by shared memory rings.
//!
//! Frame and audio payloads share a common transport primitive: a set of slots
//! backed by `SharedArrayBuffer`/`mmap` pages and two single-producer/single-
//! consumer (SPSC) index rings. One ring tracks free slots, the other advertises
//! ready work. This module provides a safe façade over that layout.

use crate::region::{RegionInit, SharedRegion};
use crate::{TransportError, TransportResult};
#[cfg(feature = "loom")]
use loom::sync::atomic::{AtomicU32, Ordering};
use std::mem;
#[cfg(not(feature = "loom"))]
use std::sync::atomic::{AtomicU32, Ordering};

/// Alignment enforced for every slot inside the pool.
pub const SLOT_ALIGNMENT: usize = 64;

/// Magic words written into ring headers when debug assertions are enabled.
#[cfg(debug_assertions)]
const FREE_RING_MAGIC: u64 = 0x5350_4F4F_4C46_5245; // "SPOOLFRE"
#[cfg(debug_assertions)]
const READY_RING_MAGIC: u64 = 0x5350_4F4F_4C52_4459; // "SPOOLRDY"
#[cfg(not(debug_assertions))]
const FREE_RING_MAGIC: u64 = 0;
#[cfg(not(debug_assertions))]
const READY_RING_MAGIC: u64 = 0;

#[repr(C, align(8))]
struct IndexRingHeader {
    capacity: u32,
    head: AtomicU32,
    tail: AtomicU32,
    pad: u32,
    magic: u64,
    reserved: u64,
}

impl IndexRingHeader {
    fn new(capacity: u32, magic: u64) -> Self {
        Self {
            capacity,
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            pad: 0,
            magic,
            reserved: 0,
        }
    }
}

/// Configuration describing the shape of a slot pool.
#[derive(Clone, Copy, Debug)]
pub struct SlotPoolConfig {
    /// Number of slots managed by the pool.
    pub slot_count: u32,
    /// Size in bytes of each slot; must be a multiple of [`SLOT_ALIGNMENT`].
    pub slot_size: usize,
}

/// Result of pushing a slot index into a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotPush {
    /// The index was enqueued successfully.
    Ok,
    /// The ring is full; callers should treat this as backpressure.
    WouldBlock,
}

/// Result of popping a slot index from the ready ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotPop {
    /// Returned the next ready slot index.
    Ok {
        /// Index of the slot ready for consumption.
        slot_idx: u32,
    },
    /// No ready indices are available at the moment.
    Empty,
}

struct IndexRing {
    region: SharedRegion,
}

impl IndexRing {
    fn new(capacity: u32, magic: u64) -> TransportResult<Self> {
        let header_size = mem::size_of::<IndexRingHeader>();
        let entries_len = mem::size_of::<u32>() * capacity as usize;
        let mut region =
            SharedRegion::new_aligned(header_size + entries_len, header_size, RegionInit::Zeroed)?;
        *region.prefix_mut::<IndexRingHeader>() = IndexRingHeader::new(capacity, magic);
        Ok(Self { region })
    }

    fn capacity(&self) -> u32 {
        self.region.prefix::<IndexRingHeader>().capacity
    }

    fn push(&mut self, value: u32) -> Result<(), ()> {
        let (capacity, head, tail) = {
            let header = self.region.prefix::<IndexRingHeader>();
            (
                header.capacity,
                header.head.load(Ordering::Relaxed),
                header.tail.load(Ordering::Acquire),
            )
        };

        if head.wrapping_sub(tail) >= capacity {
            return Err(());
        }

        {
            let entries = self.entries_mut();
            let index = (head % capacity) as usize;
            entries[index] = value;
        }

        self.region
            .prefix_mut::<IndexRingHeader>()
            .head
            .store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    fn pop(&mut self) -> Option<u32> {
        let (capacity, head, tail) = {
            let header = self.region.prefix::<IndexRingHeader>();
            (
                header.capacity,
                header.head.load(Ordering::Acquire),
                header.tail.load(Ordering::Relaxed),
            )
        };

        if tail == head {
            return None;
        }

        let value = {
            let entries = self.entries();
            let index = (tail % capacity) as usize;
            entries[index]
        };

        self.region
            .prefix_mut::<IndexRingHeader>()
            .tail
            .store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    fn fill_sequential(&mut self) {
        let capacity = self.capacity();
        {
            let entries = self.entries_mut();
            for (i, entry) in entries.iter_mut().enumerate() {
                *entry = i as u32;
            }
        }

        let header = self.region.prefix_mut::<IndexRingHeader>();
        header.tail.store(0, Ordering::Relaxed);
        header.head.store(capacity, Ordering::Release);
    }

    fn entries(&self) -> &[u32] {
        let offset = mem::size_of::<IndexRingHeader>();
        self.region.slice::<u32>(offset, self.capacity() as usize)
    }

    fn entries_mut(&mut self) -> &mut [u32] {
        let offset = mem::size_of::<IndexRingHeader>();
        let capacity = self.capacity() as usize;
        self.region.slice_mut::<u32>(offset, capacity)
    }
}

/// Fixed-size slot pool with shared rings for free and ready indices.
pub struct SlotPool {
    slots: SharedRegion,
    free_ring: IndexRing,
    ready_ring: IndexRing,
    slot_size: usize,
    slot_count: u32,
}

impl SlotPool {
    /// Allocates a new slot pool using the provided configuration.
    ///
    /// All slots begin in the free-ring, ready to be acquired via
    /// [`SlotPool::try_acquire_free`]. Callers are expected to recycle indices
    /// by feeding them through [`SlotPool::push_ready`] and
    /// [`SlotPool::release_free`].
    pub fn new(config: SlotPoolConfig) -> TransportResult<Self> {
        validate_config(&config)?;
        let SlotPoolConfig {
            slot_count,
            slot_size,
        } = config;

        let slots_len =
            slot_size
                .checked_mul(slot_count as usize)
                .ok_or(TransportError::InvalidCapacity {
                    requested: slot_size,
                    minimum: SLOT_ALIGNMENT,
                })?;

        let slots = SharedRegion::new_aligned(
            slots_len,
            SLOT_ALIGNMENT.max(4096),
            RegionInit::Uninitialized,
        )?;
        let mut free_ring = IndexRing::new(slot_count, FREE_RING_MAGIC)?;
        let ready_ring = IndexRing::new(slot_count, READY_RING_MAGIC)?;

        free_ring.fill_sequential();

        Ok(Self {
            slots,
            free_ring,
            ready_ring,
            slot_size,
            slot_count,
        })
    }

    /// Returns the number of slots managed by the pool.
    pub fn slot_count(&self) -> u32 {
        self.slot_count
    }

    /// Returns the size in bytes of each slot.
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Attempts to pop the next free slot index.
    ///
    /// Returns `None` when all slots are currently checked out.
    pub fn try_acquire_free(&mut self) -> Option<u32> {
        self.free_ring.pop()
    }

    /// Provides mutable access to the slot identified by `idx`.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of range.
    pub fn slot_mut(&mut self, idx: u32) -> &mut [u8] {
        assert!(idx < self.slot_count, "slot {idx} out of range");
        let offset = idx as usize * self.slot_size;
        let end = offset + self.slot_size;
        &mut self.slots.as_mut_slice()[offset..end]
    }

    /// Enqueues a slot index into the ready ring.
    pub fn push_ready(&mut self, idx: u32) -> SlotPush {
        assert!(idx < self.slot_count, "slot {idx} out of range");
        match self.ready_ring.push(idx) {
            Ok(()) => SlotPush::Ok,
            Err(()) => SlotPush::WouldBlock,
        }
    }

    /// Pops a slot index from the ready ring.
    pub fn pop_ready(&mut self) -> SlotPop {
        match self.ready_ring.pop() {
            Some(slot_idx) => SlotPop::Ok { slot_idx },
            None => SlotPop::Empty,
        }
    }

    /// Returns a slot index back to the free ring.
    pub fn release_free(&mut self, idx: u32) {
        assert!(idx < self.slot_count, "slot {idx} out of range");
        if self.free_ring.push(idx).is_err() {
            debug_assert!(
                false,
                "free ring overflowed – did pop paths stall? idx={idx}"
            );
        }
    }
}

fn validate_config(config: &SlotPoolConfig) -> TransportResult<()> {
    if config.slot_count == 0 {
        return Err(TransportError::InvalidCapacity {
            requested: 0,
            minimum: 1,
        });
    }

    if config.slot_size == 0 || !config.slot_size.is_multiple_of(SLOT_ALIGNMENT) {
        return Err(TransportError::InvalidCapacity {
            requested: config.slot_size,
            minimum: SLOT_ALIGNMENT,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const SLOT_COUNT: u32 = 8;
    const SLOT_SIZE: usize = SLOT_ALIGNMENT * 2;

    fn pool(count: u32) -> SlotPool {
        SlotPool::new(SlotPoolConfig {
            slot_count: count,
            slot_size: SLOT_SIZE,
        })
        .expect("create slot pool")
    }

    fn drain_free(pool: &mut SlotPool) -> Vec<u32> {
        let mut slots = Vec::new();
        while let Some(idx) = pool.try_acquire_free() {
            slots.push(idx);
        }
        for &idx in &slots {
            pool.release_free(idx);
        }
        slots
    }

    #[test]
    fn lifecycle_roundtrip() {
        let mut pool = pool(SLOT_COUNT);
        let mut acquired = Vec::new();
        for _ in 0..pool.slot_count() {
            acquired.push(pool.try_acquire_free().expect("expected free slot"));
        }
        assert!(
            pool.try_acquire_free().is_none(),
            "pool should be exhausted"
        );

        for &idx in &acquired {
            assert_eq!(pool.push_ready(idx), SlotPush::Ok);
        }
        assert_eq!(
            pool.push_ready(acquired[0]),
            SlotPush::WouldBlock,
            "ready ring must report backpressure when full"
        );

        for expected in &acquired {
            match pool.pop_ready() {
                SlotPop::Ok { slot_idx } => assert_eq!(slot_idx, *expected),
                other => panic!("expected ready slot, got {other:?}"),
            }
        }
        assert!(
            matches!(pool.pop_ready(), SlotPop::Empty),
            "ready ring should be empty after draining"
        );

        for &idx in &acquired {
            pool.release_free(idx);
        }

        let mut reacquired = Vec::new();
        for _ in 0..pool.slot_count() {
            reacquired.push(pool.try_acquire_free().expect("expected free slot"));
        }

        let mut original = acquired.clone();
        let mut roundtrip = reacquired.clone();
        original.sort_unstable();
        roundtrip.sort_unstable();
        assert_eq!(original, roundtrip);

        for idx in reacquired {
            pool.release_free(idx);
        }
    }

    #[test]
    fn slot_alignment_and_length() {
        let mut pool = pool(SLOT_COUNT);
        let idx = pool
            .try_acquire_free()
            .expect("expected at least one free slot");
        let slot = pool.slot_mut(idx);
        assert_eq!(slot.len(), SLOT_SIZE);
        let addr = slot.as_ptr() as usize;
        assert_eq!(addr % SLOT_ALIGNMENT, 0, "slot must honor alignment");
        pool.release_free(idx);
    }

    #[test]
    fn ready_ring_fifo() {
        let mut pool = pool(SLOT_COUNT);
        let mut order = VecDeque::new();
        while let Some(idx) = pool.try_acquire_free() {
            assert_eq!(pool.push_ready(idx), SlotPush::Ok);
            order.push_back(idx);
        }

        while let Some(expected) = order.pop_front() {
            match pool.pop_ready() {
                SlotPop::Ok { slot_idx } => assert_eq!(slot_idx, expected),
                SlotPop::Empty => panic!("ready ring emptied too soon"),
            }
            pool.release_free(expected);
        }
        assert!(matches!(pool.pop_ready(), SlotPop::Empty));
    }

    #[test]
    fn churn_does_not_leak_slots() {
        let mut pool = pool(SLOT_COUNT);
        for i in 0..10_000 {
            let idx = pool
                .try_acquire_free()
                .unwrap_or_else(|| panic!("run {i} expected free slot"));
            let slot = pool.slot_mut(idx);
            slot.fill(i as u8);
            assert_eq!(pool.push_ready(idx), SlotPush::Ok);
            match pool.pop_ready() {
                SlotPop::Ok { slot_idx } => pool.release_free(slot_idx),
                SlotPop::Empty => panic!("ready ring should contain slot"),
            }
        }

        let slots = drain_free(&mut pool);
        assert_eq!(
            slots.len() as u32,
            pool.slot_count(),
            "all slots must be returned to the free ring"
        );
    }
}

#[cfg(all(test, feature = "loom"))]
mod loom_tests {
    use super::*;
    use loom::sync::Arc;
    use loom::thread;

    #[derive(Debug)]
    struct LoomIndexRing {
        capacity: u32,
        head: AtomicU32,
        tail: AtomicU32,
        entries: Vec<AtomicU32>,
    }

    impl LoomIndexRing {
        fn new(capacity: u32) -> Self {
            let entries = (0..capacity).map(|_| AtomicU32::new(0)).collect::<Vec<_>>();
            Self {
                capacity,
                head: AtomicU32::new(0),
                tail: AtomicU32::new(0),
                entries,
            }
        }

        fn push(&self, value: u32) -> Result<(), ()> {
            let capacity = self.capacity;
            let head = self.head.load(Ordering::Relaxed);
            let tail = self.tail.load(Ordering::Acquire);
            if head.wrapping_sub(tail) >= capacity {
                return Err(());
            }
            let index = (head % capacity) as usize;
            self.entries[index].store(value, Ordering::Relaxed);
            self.head.store(head.wrapping_add(1), Ordering::Release);
            Ok(())
        }

        fn pop(&self) -> Option<u32> {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Relaxed);
            if tail == head {
                return None;
            }
            let capacity = self.capacity;
            let index = (tail % capacity) as usize;
            let value = self.entries[index].load(Ordering::Relaxed);
            self.tail.store(tail.wrapping_add(1), Ordering::Release);
            Some(value)
        }

        fn reset_empty(&self) {
            self.head.store(0, Ordering::Relaxed);
            self.tail.store(0, Ordering::Relaxed);
        }
    }

    #[test]
    #[ignore]
    fn slow_loom_index_ring_spsc_round_trip() {
        loom::model(|| {
            const COUNT: u32 = 4;
            let ring = Arc::new(LoomIndexRing::new(COUNT));
            ring.reset_empty();

            let producer = ring.clone();
            let consumer = ring.clone();

            let producer_thread = thread::spawn(move || {
                for idx in 0..COUNT {
                    loop {
                        if producer.push(idx).is_ok() {
                            break;
                        }
                        thread::yield_now();
                    }
                }
            });

            let consumer_thread = thread::spawn(move || {
                for expected in 0..COUNT {
                    let slot = loop {
                        if let Some(value) = consumer.pop() {
                            break value;
                        }
                        thread::yield_now();
                    };
                    assert_eq!(slot, expected);
                }
            });

            producer_thread.join().unwrap();
            consumer_thread.join().unwrap();
            let head = ring.head.load(Ordering::Acquire);
            let tail = ring.tail.load(Ordering::Acquire);
            assert_eq!(head, tail, "ring must be empty at end");
        });
    }

    #[test]
    #[ignore]
    fn slow_loom_index_ring_wraps_without_leak() {
        loom::model(|| {
            const CAPACITY: u32 = 3;
            const ITERATIONS: u32 = 3;
            let ring = Arc::new(LoomIndexRing::new(CAPACITY));
            ring.reset_empty();

            let producer = ring.clone();
            let consumer = ring.clone();

            let producer_thread = thread::spawn(move || {
                for idx in 0..ITERATIONS {
                    let value = idx % CAPACITY;
                    loop {
                        if producer.push(value).is_ok() {
                            break;
                        }
                        thread::yield_now();
                    }
                }
            });

            let consumer_thread = thread::spawn(move || {
                for idx in 0..ITERATIONS {
                    let expected = idx % CAPACITY;
                    let value = loop {
                        if let Some(val) = consumer.pop() {
                            break val;
                        }
                        thread::yield_now();
                    };
                    assert_eq!(value, expected);
                }
            });

            producer_thread.join().unwrap();
            consumer_thread.join().unwrap();
            let head = ring.head.load(Ordering::Acquire);
            let tail = ring.tail.load(Ordering::Acquire);
            assert_eq!(
                head, tail,
                "ring returns to empty state after wraparound operations"
            );
        });
    }
}
