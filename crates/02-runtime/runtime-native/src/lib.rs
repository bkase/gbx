#![deny(missing_docs)]
//! Native transport harness shared by integration tests and demos.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::thread;

use transport::{Envelope, MsgRing, SlotPool, SlotPoolConfig, SlotPop, SlotPush};
use transport_scenarios::{FabricHandle, EVENT_TAG, EVENT_VER};

const EVENT_RING_CAPACITY: usize = 512 * 1024;
const FRAME_SLOT_COUNT: u32 = 8;
const FRAME_SLOT_SIZE: usize = 128 * 1024;
const EVENT_PAYLOAD_LEN: usize = 8;

const EVENT_ENVELOPE: Envelope = Envelope {
    tag: EVENT_TAG,
    ver: EVENT_VER,
    flags: 0,
};

struct SharedSlotPool(UnsafeCell<SlotPool>);

// SAFETY: `SharedSlotPool` only grants interior mutability through the provided helpers.
unsafe impl Send for SharedSlotPool {}
// SAFETY: As above; we gate access through explicit critical sections.
unsafe impl Sync for SharedSlotPool {}

impl SharedSlotPool {
    fn new(config: SlotPoolConfig) -> Self {
        let pool = SlotPool::new(config).expect("create slot pool");
        Self(UnsafeCell::new(pool))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut SlotPool) -> R) -> R {
        // SAFETY: callers uphold exclusive access through the harness API.
        unsafe { f(&mut *self.0.get()) }
    }
}

struct SharedMsgRing(UnsafeCell<MsgRing>);

// SAFETY: Access to the ring is funnelled via `with_mut`.
unsafe impl Send for SharedMsgRing {}
// SAFETY: As above, shared references only observe the ring through exclusive sections.
unsafe impl Sync for SharedMsgRing {}

impl SharedMsgRing {
    fn new(capacity: usize, default_envelope: Envelope) -> Self {
        let ring = MsgRing::new(capacity, default_envelope).expect("create msg ring");
        Self(UnsafeCell::new(ring))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut MsgRing) -> R) -> R {
        // SAFETY: callers guarantee they will not alias active mutable borrows.
        unsafe { f(&mut *self.0.get()) }
    }
}

/// Shared ring/slot state backing the native transport harness.
#[derive(Clone)]
pub struct NativeChannels {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

impl NativeChannels {
    /// Creates the shared channels used by the native harness.
    pub fn new() -> Self {
        let frame_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT,
            slot_size: FRAME_SLOT_SIZE,
        }));
        let evt_ring = Arc::new(SharedMsgRing::new(EVENT_RING_CAPACITY, EVENT_ENVELOPE));
        Self {
            frame_pool,
            evt_ring,
        }
    }

    /// Returns a [`NativeFabricHandle`] that can drive scenario engines.
    pub fn handle(&self) -> NativeFabricHandle {
        NativeFabricHandle {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        }
    }

    /// Returns a [`NativeConsumer`] used for draining frames and events.
    pub fn consumer(&self) -> NativeConsumer {
        NativeConsumer {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        }
    }

    /// Asserts that frame slots and event rings reconciled after the run.
    pub fn assert_reconciliation(&self) {
        self.frame_pool.with_mut(|pool| {
            let mut acquired = Vec::new();
            while let Some(idx) = pool.try_acquire_free() {
                acquired.push(idx);
            }
            for idx in &acquired {
                pool.release_free(*idx);
            }
            assert_eq!(
                acquired.len() as u32,
                FRAME_SLOT_COUNT,
                "all frame slots returned free"
            );
            match pool.pop_ready() {
                SlotPop::Empty => {}
                SlotPop::Ok { slot_idx } => {
                    pool.release_free(slot_idx);
                    panic!("ready ring should be empty");
                }
            }
        });

        let ring_empty = self
            .evt_ring
            .with_mut(|ring| ring.consumer_peek().is_none());
        assert!(ring_empty, "event ring drained");
    }
}

impl Default for NativeChannels {
    fn default() -> Self {
        Self::new()
    }
}

/// Fabric handle used by the transport scenarios on native targets.
pub struct NativeFabricHandle {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

impl FabricHandle for NativeFabricHandle {
    fn acquire_free_slot(&mut self) -> Option<u32> {
        self.frame_pool.with_mut(|pool| pool.try_acquire_free())
    }

    fn wait_for_free_slot(&self) {
        thread::yield_now();
    }

    fn write_frame(&mut self, slot_idx: u32, frame_id: u32) {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            slot[..4].copy_from_slice(&frame_id.to_le_bytes());
        });
    }

    fn push_ready(&mut self, slot_idx: u32) -> SlotPush {
        self.frame_pool.with_mut(|pool| pool.push_ready(slot_idx))
    }

    fn wait_for_ready_drain(&self) {
        thread::yield_now();
    }

    fn try_push_event(&mut self, frame_id: u32, slot_idx: u32) -> bool {
        self.evt_ring.with_mut(|ring| {
            if let Some(mut grant) = ring.try_reserve(EVENT_PAYLOAD_LEN) {
                let payload = transport_scenarios::event_payload(frame_id, slot_idx);
                grant.payload()[..EVENT_PAYLOAD_LEN].copy_from_slice(&payload);
                grant.commit(EVENT_PAYLOAD_LEN);
                true
            } else {
                false
            }
        })
    }

    fn wait_for_event_space(&self) {
        thread::yield_now();
    }

    fn with_frame_slot_mut<R>(&mut self, slot_idx: u32, f: impl FnOnce(&mut [u8]) -> R) -> R {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            f(slot)
        })
    }
}

/// Consumer view over the transport channels.
#[derive(Clone)]
pub struct NativeConsumer {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

impl NativeConsumer {
    /// Attempts to pop a ready frame slot.
    pub fn pop_ready(&self) -> Option<u32> {
        self.frame_pool.with_mut(|pool| match pool.pop_ready() {
            SlotPop::Ok { slot_idx } => Some(slot_idx),
            SlotPop::Empty => None,
        })
    }

    /// Reads the frame sequence from the specified slot.
    pub fn read_slot_seq(&self, slot_idx: u32) -> u32 {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&slot[..4]);
            u32::from_le_bytes(bytes)
        })
    }

    /// Releases the slot back into the free pool.
    pub fn release_slot(&self, slot_idx: u32) {
        self.frame_pool.with_mut(|pool| pool.release_free(slot_idx));
    }

    /// Attempts to pop an event describing a completed frame.
    pub fn try_pop_event(&self) -> Option<(u32, u32)> {
        self.evt_ring.with_mut(|ring| {
            if let Some(record) = ring.consumer_peek() {
                assert_eq!(
                    record.payload.len(),
                    EVENT_PAYLOAD_LEN,
                    "event payload length"
                );
                let mut seq_bytes = [0u8; 4];
                let mut slot_bytes = [0u8; 4];
                seq_bytes.copy_from_slice(&record.payload[..4]);
                slot_bytes.copy_from_slice(&record.payload[4..]);
                ring.consumer_pop_advance();
                Some((
                    u32::from_le_bytes(seq_bytes),
                    u32::from_le_bytes(slot_bytes),
                ))
            } else {
                None
            }
        })
    }
}
