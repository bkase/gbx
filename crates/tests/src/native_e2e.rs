#![cfg(all(test, not(target_arch = "wasm32")))]

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use rkyv::{api::high::access, api::high::to_bytes, rancor::Error};
use transport::{Envelope, MsgRing, SlotPool, SlotPoolConfig, SlotPop, SlotPush};

// Lane identifier for doorbell events
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Frame = 0,
    Audio = 1,
}

// Test message for rkyv end-to-end validation
#[derive(rkyv::Archive, rkyv::Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[rkyv(bytecheck())]
struct TestMessage {
    seq: u32,
    slot_idx: u32,
    frame_id: u64,
    checksum: u32,
}

const EVT_RING_CAPACITY: usize = 512 * 1024;
const EVT_RING_CAPACITY_SMALL: usize = 512; // Small ring for wrap/backpressure tests
const EVT_RING_CAPACITY_TINY: usize = 128; // Tiny ring for aggressive backpressure test
const FRAME_SLOT_COUNT: u32 = 8;
const FRAME_SLOT_SIZE: usize = 128 * 1024;
const AUDIO_SLOT_COUNT: u32 = 6;
const AUDIO_SLOT_SIZE: usize = 16 * 1024;
const EVENT_PAYLOAD_LEN: usize = 8;
const EVENT_DOORBELL_LEN: usize = 1; // For dual-lane fairness tests using doorbells
const EVENT_ENVELOPE: Envelope = Envelope {
    tag: 0x13,
    ver: 1,
    flags: 0,
};

struct SharedSlotPool(UnsafeCell<SlotPool>);

// SAFETY: The pool is accessed by a single producer and single consumer thread via `with_mut`,
// which ensures exclusive mutable access serialized by the SPSC protocol.
unsafe impl Send for SharedSlotPool {}
// SAFETY: Same reasoning as Send; access is serialized through `with_mut`.
unsafe impl Sync for SharedSlotPool {}

impl SharedSlotPool {
    fn new(config: SlotPoolConfig) -> Self {
        let pool = SlotPool::new(config).expect("create slot pool");
        Self(UnsafeCell::new(pool))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut SlotPool) -> R) -> R {
        // SAFETY: The pool is accessed by a single producer and single consumer thread.
        unsafe { f(&mut *self.0.get()) }
    }
}

struct SharedMsgRing(UnsafeCell<MsgRing>);

// SAFETY: Exactly one producer and one consumer operate on the ring via `with_mut`,
// ensuring exclusive mutable access serialized by the SPSC protocol.
unsafe impl Send for SharedMsgRing {}
// SAFETY: Same reasoning as Send; access is serialized through `with_mut`.
unsafe impl Sync for SharedMsgRing {}

impl SharedMsgRing {
    fn new(capacity: usize, default_envelope: Envelope) -> Self {
        let ring = MsgRing::new(capacity, default_envelope).expect("create msg ring");
        Self(UnsafeCell::new(ring))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut MsgRing) -> R) -> R {
        // SAFETY: Exactly one producer and one consumer operate on the ring.
        unsafe { f(&mut *self.0.get()) }
    }
}

struct TransportChannels {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

struct DualLaneChannels {
    frame_pool: Arc<SharedSlotPool>,
    audio_pool: Arc<SharedSlotPool>,
    frame_evt_ring: Arc<SharedMsgRing>,
    audio_evt_ring: Arc<SharedMsgRing>,
}

impl TransportChannels {
    fn new() -> Self {
        let frame_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT,
            slot_size: FRAME_SLOT_SIZE,
        }));
        let evt_ring = Arc::new(SharedMsgRing::new(EVT_RING_CAPACITY, EVENT_ENVELOPE));
        Self {
            frame_pool,
            evt_ring,
        }
    }

    fn new_small_evt() -> Self {
        let frame_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT,
            slot_size: FRAME_SLOT_SIZE,
        }));
        let evt_ring = Arc::new(SharedMsgRing::new(EVT_RING_CAPACITY_SMALL, EVENT_ENVELOPE));
        Self {
            frame_pool,
            evt_ring,
        }
    }

    fn new_tiny_evt() -> Self {
        let frame_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT,
            slot_size: FRAME_SLOT_SIZE,
        }));
        let evt_ring = Arc::new(SharedMsgRing::new(EVT_RING_CAPACITY_TINY, EVENT_ENVELOPE));
        Self {
            frame_pool,
            evt_ring,
        }
    }

    fn split(&self) -> (FrameProducer, FrameConsumer) {
        let producer = FrameProducer {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        };
        let consumer = FrameConsumer {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        };
        (producer, consumer)
    }

    fn assert_reconciliation(&self) {
        self.frame_pool.with_mut(|pool| {
            let mut free_count = 0u32;
            let mut acquired = Vec::with_capacity(FRAME_SLOT_COUNT as usize);
            while let Some(idx) = pool.try_acquire_free() {
                free_count += 1;
                acquired.push(idx);
            }
            for idx in acquired {
                pool.release_free(idx);
            }
            assert_eq!(
                free_count, FRAME_SLOT_COUNT,
                "all frame slots must return to free ring"
            );

            match pool.pop_ready() {
                SlotPop::Empty => {}
                SlotPop::Ok { slot_idx } => {
                    pool.release_free(slot_idx);
                    panic!("ready ring should be empty");
                }
            }
        });

        let evt_empty = self
            .evt_ring
            .with_mut(|ring| ring.consumer_peek().is_none());
        assert!(evt_empty, "event ring should be empty at teardown");
    }
}

struct FrameProducer {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

impl FrameProducer {
    fn acquire_slot(&self) -> u32 {
        loop {
            if let Some(idx) = self.frame_pool.with_mut(|pool| pool.try_acquire_free()) {
                return idx;
            }
            thread::yield_now();
        }
    }

    fn write_slot(&self, slot_idx: u32, seq: u32) {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            slot[..4].copy_from_slice(&seq.to_le_bytes());
        });
    }

    fn push_ready(&self, slot_idx: u32) -> SlotPush {
        self.frame_pool.with_mut(|pool| pool.push_ready(slot_idx))
    }

    fn push_event(&self, seq: u32, slot_idx: u32) {
        loop {
            let committed = self.evt_ring.with_mut(|ring| {
                if let Some(mut grant) = ring.try_reserve(EVENT_PAYLOAD_LEN) {
                    encode_event(grant.payload(), seq, slot_idx);
                    grant.commit(EVENT_PAYLOAD_LEN);
                    true
                } else {
                    false
                }
            });
            if committed {
                break;
            }
            thread::yield_now();
        }
    }
}

struct FrameConsumer {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
}

impl FrameConsumer {
    fn pop_ready(&self) -> Option<u32> {
        self.frame_pool.with_mut(|pool| match pool.pop_ready() {
            SlotPop::Ok { slot_idx } => Some(slot_idx),
            SlotPop::Empty => None,
        })
    }

    fn release_slot(&self, slot_idx: u32) {
        self.frame_pool.with_mut(|pool| pool.release_free(slot_idx));
    }

    fn read_slot_seq(&self, slot_idx: u32) -> u32 {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&slot[..4]);
            u32::from_le_bytes(bytes)
        })
    }

    fn try_pop_event(&self) -> Option<(u32, u32)> {
        self.evt_ring.with_mut(|ring| {
            if let Some(record) = ring.consumer_peek() {
                assert_eq!(
                    record.payload.len(),
                    EVENT_PAYLOAD_LEN,
                    "unexpected event payload length"
                );
                let (seq, slot_idx) = decode_event(record.payload);
                ring.consumer_pop_advance();
                Some((seq, slot_idx))
            } else {
                None
            }
        })
    }
}

fn encode_event(buf: &mut [u8], seq: u32, slot_idx: u32) {
    buf[..4].copy_from_slice(&seq.to_le_bytes());
    buf[4..8].copy_from_slice(&slot_idx.to_le_bytes());
}

fn decode_event(payload: &[u8]) -> (u32, u32) {
    let mut seq_bytes = [0u8; 4];
    seq_bytes.copy_from_slice(&payload[..4]);
    let mut slot_bytes = [0u8; 4];
    slot_bytes.copy_from_slice(&payload[4..8]);
    (
        u32::from_le_bytes(seq_bytes),
        u32::from_le_bytes(slot_bytes),
    )
}

// Doorbell event encoding/decoding for dual-lane tests
fn encode_doorbell(buf: &mut [u8], lane: Lane) {
    buf[0] = lane as u8;
}

#[allow(dead_code)]
fn decode_doorbell(payload: &[u8]) -> Lane {
    match payload[0] {
        0 => Lane::Frame,
        1 => Lane::Audio,
        _ => panic!("invalid lane byte"),
    }
}

/// Validates lock-free SPSC transport under high throughput by producing and consuming 10,000 frames with slot/event coordination.
#[test]
fn native_transport_flood_frames() {
    const FRAME_TARGET: u32 = 10_000;
    let transport = TransportChannels::new();
    let (producer, consumer) = transport.split();

    let producer_thread = thread::spawn(move || {
        for seq in 0..FRAME_TARGET {
            let slot_idx = producer.acquire_slot();
            producer.write_slot(slot_idx, seq);
            loop {
                match producer.push_ready(slot_idx) {
                    SlotPush::Ok => break,
                    SlotPush::WouldBlock => thread::yield_now(),
                }
            }
            producer.push_event(seq, slot_idx);
        }
    });

    let consumer_thread = thread::spawn(move || {
        let mut frames = Vec::with_capacity(FRAME_TARGET as usize);
        let mut events = Vec::with_capacity(FRAME_TARGET as usize);

        while frames.len() < FRAME_TARGET as usize {
            if let Some(slot_idx) = consumer.pop_ready() {
                let (seq, evt_slot_idx) = loop {
                    if let Some(pair) = consumer.try_pop_event() {
                        break pair;
                    }
                    thread::yield_now();
                };
                assert_eq!(slot_idx, evt_slot_idx, "event slot mismatch");
                let slot_seq = consumer.read_slot_seq(slot_idx);
                assert_eq!(slot_seq, seq, "slot payload mismatch");
                consumer.release_slot(slot_idx);
                frames.push(seq);
                events.push(seq);
            } else {
                thread::yield_now();
            }
        }

        assert!(
            consumer.try_pop_event().is_none(),
            "event ring should be empty after flood drain"
        );
        (frames, events)
    });

    producer_thread.join().unwrap();
    let (frames, events) = consumer_thread.join().unwrap();

    assert_eq!(frames.len() as u32, FRAME_TARGET, "drained frame count");
    assert_eq!(events, frames, "event ordering should mirror frames");

    transport.assert_reconciliation();
}

/// Exercises bursty production with limited drain budget to verify ready-ring depth stays bounded by slot count.
#[test]
fn native_transport_burst_fairness() {
    struct BurstConfig {
        bursts: u32,
        burst_size: u32,
        drain_budget: u32,
    }

    let config = BurstConfig {
        bursts: 40,
        burst_size: 64,
        drain_budget: 8,
    };
    let total_frames = config.bursts * config.burst_size;

    let transport = TransportChannels::new();
    let (producer, consumer) = transport.split();
    let produced_counter = Arc::new(AtomicU32::new(0));

    let producer_count = Arc::clone(&produced_counter);
    let producer_thread = thread::spawn(move || {
        let mut seq = 0u32;
        for _ in 0..config.bursts {
            for _ in 0..config.burst_size {
                let slot_idx = producer.acquire_slot();
                producer.write_slot(slot_idx, seq);
                loop {
                    match producer.push_ready(slot_idx) {
                        SlotPush::Ok => break,
                        SlotPush::WouldBlock => thread::yield_now(),
                    }
                }
                producer.push_event(seq, slot_idx);
                producer_count.fetch_add(1, Ordering::Release);
                seq += 1;
            }
        }
    });

    let consumer_count = Arc::clone(&produced_counter);
    let consumer_thread = thread::spawn(move || {
        let mut frames = Vec::with_capacity(total_frames as usize);
        let mut events = Vec::with_capacity(total_frames as usize);
        let mut max_ready_depth = 0u32;

        while frames.len() < total_frames as usize {
            let mut drained = 0u32;
            while drained < config.drain_budget && frames.len() < total_frames as usize {
                match consumer.pop_ready() {
                    Some(slot_idx) => {
                        let (seq, evt_slot_idx) = loop {
                            if let Some(pair) = consumer.try_pop_event() {
                                break pair;
                            }
                            thread::yield_now();
                        };
                        assert_eq!(slot_idx, evt_slot_idx, "burst slot mismatch");
                        let slot_seq = consumer.read_slot_seq(slot_idx);
                        assert_eq!(slot_seq, seq, "burst slot payload mismatch");
                        consumer.release_slot(slot_idx);
                        frames.push(seq);
                        events.push(seq);
                        drained += 1;
                    }
                    None => break,
                }
            }
            let produced = consumer_count.load(Ordering::Acquire);
            let ready_depth = produced.saturating_sub(frames.len() as u32);
            if ready_depth > max_ready_depth {
                max_ready_depth = ready_depth;
            }
            thread::yield_now();
        }

        assert!(
            consumer.try_pop_event().is_none(),
            "event ring should drain after bursts"
        );

        (frames, events, max_ready_depth)
    });

    producer_thread.join().unwrap();
    let (frames, events, max_ready_depth) = consumer_thread.join().unwrap();

    assert_eq!(frames.len() as u32, total_frames, "burst drain frame count");
    assert_eq!(events, frames, "event ordering should match bursts");
    assert!(
        max_ready_depth <= FRAME_SLOT_COUNT,
        "ready ring depth should not exceed slot budget"
    );

    transport.assert_reconciliation();
}

/// Verifies graceful backpressure handling when consumer pauses, ensuring producer observes free-ring exhaustion and recovers.
#[test]
fn native_transport_backpressure_recovery() {
    // Test that system handles slow consumer gracefully
    // Producer should hit backpressure (free ring exhaustion when consumer is slow)
    const FRAME_TARGET: u32 = 256;
    const PAUSE_FRAMES: u32 = 12; // Pause after this many frames to create backpressure
    const PAUSE_MS: u64 = 25; // Pause duration to allow producer to hit backpressure

    let transport = TransportChannels::new();
    let (producer, consumer) = transport.split();

    let free_waits = Arc::new(AtomicU32::new(0));
    let would_block_count = Arc::new(AtomicU32::new(0));

    let consumer_thread = thread::spawn(move || {
        let mut frames = Vec::with_capacity(FRAME_TARGET as usize);

        while frames.len() < FRAME_TARGET as usize {
            if let Some(slot_idx) = consumer.pop_ready() {
                let (seq, evt_slot_idx) = loop {
                    if let Some(pair) = consumer.try_pop_event() {
                        break pair;
                    }
                    thread::yield_now();
                };
                assert_eq!(slot_idx, evt_slot_idx, "backpressure slot mismatch");
                let slot_seq = consumer.read_slot_seq(slot_idx);
                assert_eq!(slot_seq, seq, "backpressure slot payload mismatch");
                consumer.release_slot(slot_idx);
                frames.push(seq);

                // After consuming PAUSE_FRAMES, pause briefly to create backpressure
                if frames.len() == PAUSE_FRAMES as usize {
                    thread::sleep(std::time::Duration::from_millis(PAUSE_MS));
                }
            } else {
                thread::yield_now();
            }
        }

        assert!(
            consumer.try_pop_event().is_none(),
            "event ring should be empty after drain"
        );

        frames
    });

    let producer_waits = Arc::clone(&free_waits);
    let producer_would_block = Arc::clone(&would_block_count);
    let producer_thread = thread::spawn(move || {
        for seq in 0..FRAME_TARGET {
            // Track if we need to wait for free slots
            let mut wait_count = 0;
            let slot_idx = loop {
                if let Some(idx) = producer.frame_pool.with_mut(|pool| pool.try_acquire_free()) {
                    if wait_count > 0 {
                        producer_waits.fetch_add(1, Ordering::Relaxed);
                    }
                    break idx;
                }
                wait_count += 1;
                thread::yield_now();
            };

            producer.write_slot(slot_idx, seq);

            // Try to push to ready ring, handle backpressure
            loop {
                match producer.push_ready(slot_idx) {
                    SlotPush::Ok => break,
                    SlotPush::WouldBlock => {
                        producer_would_block.fetch_add(1, Ordering::Relaxed);
                        thread::yield_now();
                    }
                }
            }

            producer.push_event(seq, slot_idx);
        }
    });

    producer_thread.join().unwrap();
    let frames = consumer_thread.join().unwrap();

    let expected: Vec<u32> = (0..FRAME_TARGET).collect();
    assert_eq!(frames, expected, "should consume all frames in order");
    assert_eq!(frames.len() as u32, FRAME_TARGET, "frame count");

    let total_free_waits = free_waits.load(Ordering::Relaxed);
    let total_would_block = would_block_count.load(Ordering::Relaxed);

    assert!(
        total_free_waits > 0 || total_would_block > 0,
        "producer should observe backpressure (free_waits={total_free_waits}, would_block={total_would_block})"
    );

    transport.assert_reconciliation();
}

/// Validates MsgRing wrap-around and sentinel logic with a small ring and variable payload sizes.
#[test]
fn native_transport_forced_wrap_sentinel() {
    // A1: Test that MsgRing wrap-around and sentinel logic works end-to-end
    // Uses a small event ring (512 bytes) and alternating payload sizes to force wrap
    const SMALL_PAYLOAD: usize = 8;
    const MEDIUM_PAYLOAD: usize = 24;
    const LARGE_PAYLOAD: usize = 40;
    const RECORD_COUNT: u32 = 100;

    let transport = TransportChannels::new_small_evt();
    let (producer, consumer) = transport.split();

    let producer_thread = thread::spawn(move || {
        for seq in 0..RECORD_COUNT {
            let slot_idx = producer.acquire_slot();
            producer.write_slot(slot_idx, seq);
            loop {
                match producer.push_ready(slot_idx) {
                    SlotPush::Ok => break,
                    SlotPush::WouldBlock => thread::yield_now(),
                }
            }

            let payload_len = match seq % 3 {
                0 => SMALL_PAYLOAD,
                1 => MEDIUM_PAYLOAD,
                _ => LARGE_PAYLOAD,
            };

            loop {
                let committed = producer.evt_ring.with_mut(|ring| {
                    if let Some(mut grant) = ring.try_reserve(payload_len) {
                        let payload = grant.payload();
                        payload[..4].copy_from_slice(&seq.to_le_bytes());
                        payload[4..8].copy_from_slice(&slot_idx.to_le_bytes());
                        if payload_len > 8 {
                            payload[8..payload_len].fill((seq & 0xFF) as u8);
                        }
                        grant.commit(payload_len);
                        true
                    } else {
                        false
                    }
                });
                if committed {
                    break;
                }
                thread::yield_now();
            }
        }
    });

    let consumer_thread = thread::spawn(move || {
        let mut received = Vec::with_capacity(RECORD_COUNT as usize);

        while received.len() < RECORD_COUNT as usize {
            if let Some(slot_idx) = consumer.pop_ready() {
                let (seq, evt_slot_idx) = loop {
                    let result = consumer.evt_ring.with_mut(|ring| {
                        if let Some(record) = ring.consumer_peek() {
                            let mut seq_bytes = [0u8; 4];
                            seq_bytes.copy_from_slice(&record.payload[..4]);
                            let mut slot_bytes = [0u8; 4];
                            slot_bytes.copy_from_slice(&record.payload[4..8]);
                            let seq = u32::from_le_bytes(seq_bytes);
                            let slot = u32::from_le_bytes(slot_bytes);
                            ring.consumer_pop_advance();
                            Some((seq, slot))
                        } else {
                            None
                        }
                    });
                    if let Some(pair) = result {
                        break pair;
                    }
                    thread::yield_now();
                };

                assert_eq!(slot_idx, evt_slot_idx, "wrap test slot mismatch");
                let slot_seq = consumer.read_slot_seq(slot_idx);
                assert_eq!(slot_seq, seq, "wrap test payload mismatch");
                consumer.release_slot(slot_idx);
                received.push(seq);
            } else {
                thread::yield_now();
            }
        }

        received
    });

    producer_thread.join().unwrap();
    let received = consumer_thread.join().unwrap();

    assert_eq!(
        received.len(),
        RECORD_COUNT as usize,
        "wrap test record count"
    );
    for (i, &seq) in received.iter().enumerate() {
        assert_eq!(seq, i as u32, "wrap test sequence at index {i}");
    }

    transport.assert_reconciliation();
}

/// Exercises rkyv serialization, deserialization, and envelope validation across the event ring.
#[test]
fn native_transport_rkyv_end_to_end() {
    // A2: Test rkyv serialization end-to-end with envelope validation
    const TEST_TAG: u8 = 0x42;
    const TEST_VER: u8 = 2;
    const RECORD_COUNT: u32 = 500;

    let transport = TransportChannels::new();
    let (producer, consumer) = transport.split();

    let producer_thread = thread::spawn(move || {
        for seq in 0..RECORD_COUNT {
            let slot_idx = producer.acquire_slot();
            producer.write_slot(slot_idx, seq);
            loop {
                match producer.push_ready(slot_idx) {
                    SlotPush::Ok => break,
                    SlotPush::WouldBlock => thread::yield_now(),
                }
            }

            // Serialize TestMessage with rkyv and send it into the event ring
            let msg = TestMessage {
                seq,
                slot_idx,
                frame_id: (seq as u64) * 1000,
                checksum: seq.wrapping_mul(0x9E3779B9),
            };

            let bytes = to_bytes::<Error>(&msg).expect("serialize test message");
            let need = bytes.len();

            loop {
                let committed = producer.evt_ring.with_mut(|ring| {
                    if let Some(mut grant) = ring.try_reserve(need) {
                        grant.set_envelope(Envelope {
                            tag: TEST_TAG,
                            ver: TEST_VER,
                            flags: 0,
                        });

                        {
                            let payload = grant.payload();
                            payload[..need].copy_from_slice(bytes.as_ref());
                        }
                        grant.commit(need);
                        true
                    } else {
                        false
                    }
                });
                if committed {
                    break;
                }
                thread::yield_now();
            }
        }
    });

    let consumer_thread = thread::spawn(move || {
        let mut received = Vec::with_capacity(RECORD_COUNT as usize);

        while received.len() < RECORD_COUNT as usize {
            if let Some(slot_idx) = consumer.pop_ready() {
                let (seq, frame_id, checksum) = loop {
                    if let Some((_envelope, seq, frame_id, checksum)) =
                        consumer.evt_ring.with_mut(|ring| {
                            if let Some(record) = ring.consumer_peek() {
                                // Validate envelope
                                assert_eq!(record.envelope.tag, TEST_TAG, "envelope tag mismatch");
                                assert_eq!(
                                    record.envelope.ver, TEST_VER,
                                    "envelope version mismatch"
                                );

                                let archived =
                                    access::<rkyv::Archived<TestMessage>, Error>(record.payload)
                                        .expect("rkyv validation failed");

                                let result = (
                                    record.envelope,
                                    archived.seq.to_native(),
                                    archived.frame_id.to_native(),
                                    archived.checksum.to_native(),
                                );

                                ring.consumer_pop_advance();
                                Some(result)
                            } else {
                                None
                            }
                        })
                    {
                        break (seq, frame_id, checksum);
                    }
                    thread::yield_now();
                };

                let slot_seq = consumer.read_slot_seq(slot_idx);
                assert_eq!(slot_seq, seq, "rkyv slot payload mismatch");
                assert_eq!(frame_id, (seq as u64) * 1000, "rkyv frame_id mismatch");
                assert_eq!(
                    checksum,
                    seq.wrapping_mul(0x9E3779B9),
                    "rkyv checksum mismatch"
                );

                consumer.release_slot(slot_idx);
                received.push(seq);
            } else {
                thread::yield_now();
            }
        }

        received
    });

    producer_thread.join().unwrap();
    let received = consumer_thread.join().unwrap();

    assert_eq!(
        received.len(),
        RECORD_COUNT as usize,
        "rkyv test record count"
    );
    for (i, &seq) in received.iter().enumerate() {
        assert_eq!(seq, i as u32, "rkyv test sequence at index {i}");
    }

    transport.assert_reconciliation();
}

/// Confirms event-ring backpressure detection with a tiny (128-byte) ring and slow consumer.
#[test]
fn native_transport_evt_ring_backpressure() {
    // A3: Test event-ring backpressure with tiny capacity (128 bytes)
    const RECORD_COUNT: u32 = 100;
    const PAUSE_AFTER: u32 = 3; // Pause very early to create backpressure
    const PAUSE_MS: u64 = 50; // Pause to allow ring to fill

    let transport = TransportChannels::new_tiny_evt();
    let (producer, consumer) = transport.split();

    let evt_would_block = Arc::new(AtomicU32::new(0));

    let producer_would_block = Arc::clone(&evt_would_block);
    let producer_thread = thread::spawn(move || {
        for seq in 0..RECORD_COUNT {
            let slot_idx = producer.acquire_slot();
            producer.write_slot(slot_idx, seq);
            loop {
                match producer.push_ready(slot_idx) {
                    SlotPush::Ok => break,
                    SlotPush::WouldBlock => thread::yield_now(),
                }
            }

            // Try to push event with backpressure tracking
            let mut wait_count = 0;
            loop {
                let committed = producer.evt_ring.with_mut(|ring| {
                    if let Some(mut grant) = ring.try_reserve(EVENT_PAYLOAD_LEN) {
                        encode_event(grant.payload(), seq, slot_idx);
                        grant.commit(EVENT_PAYLOAD_LEN);
                        if wait_count > 0 {
                            producer_would_block.fetch_add(1, Ordering::Relaxed);
                        }
                        true
                    } else {
                        wait_count += 1;
                        false
                    }
                });
                if committed {
                    break;
                }
                thread::yield_now();
            }
        }
    });

    let consumer_thread = thread::spawn(move || {
        let mut received = Vec::with_capacity(RECORD_COUNT as usize);

        while received.len() < RECORD_COUNT as usize {
            // Pause event drain after PAUSE_AFTER to create backpressure
            if received.len() == PAUSE_AFTER as usize {
                thread::sleep(std::time::Duration::from_millis(PAUSE_MS));
            }

            if let Some(slot_idx) = consumer.pop_ready() {
                let (seq, evt_slot_idx) = loop {
                    if let Some(pair) = consumer.try_pop_event() {
                        break pair;
                    }
                    thread::yield_now();
                };

                assert_eq!(slot_idx, evt_slot_idx, "evt backpressure slot mismatch");
                let slot_seq = consumer.read_slot_seq(slot_idx);
                assert_eq!(slot_seq, seq, "evt backpressure payload mismatch");
                consumer.release_slot(slot_idx);
                received.push(seq);
            } else {
                thread::yield_now();
            }
        }

        received
    });

    producer_thread.join().unwrap();
    let received = consumer_thread.join().unwrap();

    assert_eq!(
        received.len(),
        RECORD_COUNT as usize,
        "evt backpressure record count"
    );

    let total_evt_would_block = evt_would_block.load(Ordering::Relaxed);
    assert!(
        total_evt_would_block > 0,
        "producer should observe evt-ring backpressure (evt_would_block={total_evt_would_block})"
    );

    transport.assert_reconciliation();
}

impl DualLaneChannels {
    fn new() -> Self {
        let frame_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT,
            slot_size: FRAME_SLOT_SIZE,
        }));
        let audio_pool = Arc::new(SharedSlotPool::new(SlotPoolConfig {
            slot_count: AUDIO_SLOT_COUNT,
            slot_size: AUDIO_SLOT_SIZE,
        }));
        let frame_evt_ring = Arc::new(SharedMsgRing::new(EVT_RING_CAPACITY, EVENT_ENVELOPE));
        let audio_evt_ring = Arc::new(SharedMsgRing::new(EVT_RING_CAPACITY, EVENT_ENVELOPE));
        Self {
            frame_pool,
            audio_pool,
            frame_evt_ring,
            audio_evt_ring,
        }
    }

    fn assert_reconciliation(&self) {
        // Check frame pool
        self.frame_pool.with_mut(|pool| {
            let mut free_count = 0u32;
            let mut acquired = Vec::with_capacity(FRAME_SLOT_COUNT as usize);
            while let Some(idx) = pool.try_acquire_free() {
                free_count += 1;
                acquired.push(idx);
            }
            for idx in acquired {
                pool.release_free(idx);
            }
            assert_eq!(
                free_count, FRAME_SLOT_COUNT,
                "all frame slots must return to free ring"
            );

            match pool.pop_ready() {
                SlotPop::Empty => {}
                SlotPop::Ok { slot_idx } => {
                    pool.release_free(slot_idx);
                    panic!("frame ready ring should be empty");
                }
            }
        });

        // Check audio pool
        self.audio_pool.with_mut(|pool| {
            let mut free_count = 0u32;
            let mut acquired = Vec::with_capacity(AUDIO_SLOT_COUNT as usize);
            while let Some(idx) = pool.try_acquire_free() {
                free_count += 1;
                acquired.push(idx);
            }
            for idx in acquired {
                pool.release_free(idx);
            }
            assert_eq!(
                free_count, AUDIO_SLOT_COUNT,
                "all audio slots must return to free ring"
            );

            match pool.pop_ready() {
                SlotPop::Empty => {}
                SlotPop::Ok { slot_idx } => {
                    pool.release_free(slot_idx);
                    panic!("audio ready ring should be empty");
                }
            }
        });

        let frame_evt_empty = self
            .frame_evt_ring
            .with_mut(|ring| ring.consumer_peek().is_none());
        let audio_evt_empty = self
            .audio_evt_ring
            .with_mut(|ring| ring.consumer_peek().is_none());
        assert!(
            frame_evt_empty,
            "frame event ring should be empty at teardown"
        );
        assert!(
            audio_evt_empty,
            "audio event ring should be empty at teardown"
        );
    }
}

/// Stresses dual-lane (frame and audio) transport fairness using round-robin consumption and doorbell events.
#[test]
fn native_transport_cross_lane_fairness() {
    // A4: Test cross-lane fairness with frames + audio pools
    const FRAME_BURSTS: u32 = 20;
    const AUDIO_BURSTS: u32 = 15;
    const FRAME_BURST_SIZE: u32 = 32;
    const AUDIO_BURST_SIZE: u32 = 17;
    const DRAIN_BUDGET: u32 = 10; // Total per iteration, split via RR

    let total_frames = FRAME_BURSTS * FRAME_BURST_SIZE;
    let total_audio = AUDIO_BURSTS * AUDIO_BURST_SIZE;

    let channels = DualLaneChannels::new();

    let frame_pool = Arc::clone(&channels.frame_pool);
    let audio_pool = Arc::clone(&channels.audio_pool);
    let frame_evt_ring = Arc::clone(&channels.frame_evt_ring);
    let audio_evt_ring = Arc::clone(&channels.audio_evt_ring);

    let frame_produced = Arc::new(AtomicU32::new(0));
    let audio_produced = Arc::new(AtomicU32::new(0));

    // Frame producer - uses doorbell pattern
    let frame_counter = Arc::clone(&frame_produced);
    let frame_pool_prod = Arc::clone(&frame_pool);
    let evt_ring_frame = Arc::clone(&frame_evt_ring);
    let frame_thread = thread::spawn(move || {
        for burst_idx in 0..FRAME_BURSTS {
            for i in 0..FRAME_BURST_SIZE {
                let seq = burst_idx * FRAME_BURST_SIZE + i;
                let slot_idx = loop {
                    if let Some(idx) = frame_pool_prod.with_mut(|pool| pool.try_acquire_free()) {
                        break idx;
                    }
                    thread::yield_now();
                };

                frame_pool_prod.with_mut(|pool| {
                    let slot = pool.slot_mut(slot_idx);
                    // Write seq as payload (for validation)
                    slot[..4].copy_from_slice(&seq.to_le_bytes());
                    // Write lane marker to distinguish from audio
                    slot[4] = Lane::Frame as u8;
                });

                // Push to ready BEFORE pushing event (happens-before)
                loop {
                    match frame_pool_prod.with_mut(|pool| pool.push_ready(slot_idx)) {
                        SlotPush::Ok => break,
                        SlotPush::WouldBlock => thread::yield_now(),
                    }
                }

                // Push doorbell event (just lane, no slot index)
                loop {
                    let committed = evt_ring_frame.with_mut(|ring| {
                        if let Some(mut grant) = ring.try_reserve(EVENT_DOORBELL_LEN) {
                            encode_doorbell(grant.payload(), Lane::Frame);
                            grant.commit(EVENT_DOORBELL_LEN);
                            true
                        } else {
                            false
                        }
                    });
                    if committed {
                        break;
                    }
                    thread::yield_now();
                }

                frame_counter.fetch_add(1, Ordering::Release);
            }
        }
    });

    // Audio producer - uses doorbell pattern
    let audio_counter = Arc::clone(&audio_produced);
    let audio_pool_prod = Arc::clone(&audio_pool);
    let evt_ring_audio = Arc::clone(&audio_evt_ring);
    let audio_thread = thread::spawn(move || {
        for burst_idx in 0..AUDIO_BURSTS {
            for i in 0..AUDIO_BURST_SIZE {
                let seq = burst_idx * AUDIO_BURST_SIZE + i;
                let slot_idx = loop {
                    if let Some(idx) = audio_pool_prod.with_mut(|pool| pool.try_acquire_free()) {
                        break idx;
                    }
                    thread::yield_now();
                };

                audio_pool_prod.with_mut(|pool| {
                    let slot = pool.slot_mut(slot_idx);
                    // Write seq as payload (for validation)
                    slot[..4].copy_from_slice(&seq.to_le_bytes());
                    // Write lane marker to distinguish from frame
                    slot[4] = Lane::Audio as u8;
                });

                // Push to ready BEFORE pushing event (happens-before)
                loop {
                    match audio_pool_prod.with_mut(|pool| pool.push_ready(slot_idx)) {
                        SlotPush::Ok => break,
                        SlotPush::WouldBlock => thread::yield_now(),
                    }
                }

                // Push doorbell event (just lane, no slot index)
                loop {
                    let committed = evt_ring_audio.with_mut(|ring| {
                        if let Some(mut grant) = ring.try_reserve(EVENT_DOORBELL_LEN) {
                            encode_doorbell(grant.payload(), Lane::Audio);
                            grant.commit(EVENT_DOORBELL_LEN);
                            true
                        } else {
                            false
                        }
                    });
                    if committed {
                        break;
                    }
                    thread::yield_now();
                }

                audio_counter.fetch_add(1, Ordering::Release);
            }
        }
    });

    // Consumer - converts events to tickets, drains with round-robin
    let frame_pool_cons = Arc::clone(&frame_pool);
    let audio_pool_cons = Arc::clone(&audio_pool);
    let frame_evt_ring_cons = Arc::clone(&frame_evt_ring);
    let audio_evt_ring_cons = Arc::clone(&audio_evt_ring);
    let consumer_thread = thread::spawn(move || {
        let mut frames_received = Vec::with_capacity(total_frames as usize);
        let mut audio_received = Vec::with_capacity(total_audio as usize);
        let mut max_frame_depth = 0u32;
        let mut max_audio_depth = 0u32;

        // Ticket-based consumption: events grant tickets to consume ready slots
        let mut tickets = [0usize; 2]; // [frame, audio]
        let mut since_served = [0usize; 2];
        let mut max_wait = [0usize; 2];

        while frames_received.len() < total_frames as usize
            || audio_received.len() < total_audio as usize
        {
            // Convert events to tickets
            while frame_evt_ring_cons.with_mut(|ring| {
                if ring.consumer_peek().is_some() {
                    ring.consumer_pop_advance();
                    true
                } else {
                    false
                }
            }) {
                tickets[0] += 1;
            }

            while audio_evt_ring_cons.with_mut(|ring| {
                if ring.consumer_peek().is_some() {
                    ring.consumer_pop_advance();
                    true
                } else {
                    false
                }
            }) {
                tickets[1] += 1;
            }

            // Round-robin drain honoring tickets
            let mut did_any = false;
            for _ in 0..DRAIN_BUDGET {
                for lane_idx in 0..2 {
                    if tickets[lane_idx] == 0 {
                        continue;
                    }

                    let slot_result = match lane_idx {
                        0 => frame_pool_cons.with_mut(|pool| pool.pop_ready()),
                        _ => audio_pool_cons.with_mut(|pool| pool.pop_ready()),
                    };

                    match slot_result {
                        SlotPop::Ok { slot_idx } => {
                            match lane_idx {
                                0 => {
                                    let (seq, lane_marker) = frame_pool_cons.with_mut(|pool| {
                                        let slot = pool.slot_mut(slot_idx);
                                        let mut seq_bytes = [0u8; 4];
                                        seq_bytes.copy_from_slice(&slot[..4]);
                                        (u32::from_le_bytes(seq_bytes), slot[4])
                                    });
                                    assert_eq!(lane_marker, Lane::Frame as u8);
                                    frame_pool_cons.with_mut(|pool| pool.release_free(slot_idx));
                                    frames_received.push(seq);
                                }
                                _ => {
                                    let (seq, lane_marker) = audio_pool_cons.with_mut(|pool| {
                                        let slot = pool.slot_mut(slot_idx);
                                        let mut seq_bytes = [0u8; 4];
                                        seq_bytes.copy_from_slice(&slot[..4]);
                                        (u32::from_le_bytes(seq_bytes), slot[4])
                                    });
                                    assert_eq!(lane_marker, Lane::Audio as u8);
                                    audio_pool_cons.with_mut(|pool| pool.release_free(slot_idx));
                                    audio_received.push(seq);
                                }
                            }
                            tickets[lane_idx] -= 1;
                            max_wait[lane_idx] = max_wait[lane_idx].max(since_served[lane_idx]);
                            since_served[lane_idx] = 0;
                            did_any = true;
                        }
                        SlotPop::Empty => {
                            // Have tickets but ready ring is empty - this shouldn't happen
                            // with proper happens-before, but track it as starvation
                            since_served[lane_idx] += 1;
                        }
                    }
                }
            }
            if !did_any && tickets == [0, 0] {
                // No work done and no tickets, yield
                thread::yield_now();
            }

            // Track max depths
            let frame_depth = frame_produced
                .load(Ordering::Acquire)
                .saturating_sub(frames_received.len() as u32);
            let audio_depth = audio_produced
                .load(Ordering::Acquire)
                .saturating_sub(audio_received.len() as u32);

            max_frame_depth = max_frame_depth.max(frame_depth);
            max_audio_depth = max_audio_depth.max(audio_depth);
        }

        // Drain any remaining events from both rings
        while frame_evt_ring_cons.with_mut(|ring| {
            if ring.consumer_peek().is_some() {
                ring.consumer_pop_advance();
                true
            } else {
                false
            }
        }) {}

        while audio_evt_ring_cons.with_mut(|ring| {
            if ring.consumer_peek().is_some() {
                ring.consumer_pop_advance();
                true
            } else {
                false
            }
        }) {}

        (
            frames_received,
            audio_received,
            max_frame_depth,
            max_audio_depth,
            max_wait,
        )
    });

    frame_thread.join().unwrap();
    audio_thread.join().unwrap();
    let (frames, audio, max_frame_depth, max_audio_depth, max_wait) =
        consumer_thread.join().unwrap();

    // Invariant 1: Counts match per lane (consumed == produced)
    assert_eq!(frames.len(), total_frames as usize, "frame count mismatch");
    assert_eq!(audio.len(), total_audio as usize, "audio count mismatch");

    // Invariant 2: Bounded depth (ready ring depth ≤ slot count)
    assert!(
        max_frame_depth <= FRAME_SLOT_COUNT,
        "frame ready depth exceeded slot budget: {max_frame_depth} > {FRAME_SLOT_COUNT}"
    );

    assert!(
        max_audio_depth <= AUDIO_SLOT_COUNT,
        "audio ready depth exceeded slot budget: {max_audio_depth} > {AUDIO_SLOT_COUNT}"
    );

    // Invariant 3: No starvation under backlog (max wait ticks ≤ budget)
    // With DRAIN_BUDGET=10 and 2 lanes, worst case is ~2*DRAIN_BUDGET ticks
    let max_allowed_wait = (DRAIN_BUDGET * 2) as usize;
    assert!(
        max_wait[0] <= max_allowed_wait,
        "frame lane starvation: max_wait={} > {}",
        max_wait[0],
        max_allowed_wait
    );
    assert!(
        max_wait[1] <= max_allowed_wait,
        "audio lane starvation: max_wait={} > {}",
        max_wait[1],
        max_allowed_wait
    );

    // Invariant 4: No leaks (all slots returned, rings empty)
    channels.assert_reconciliation();
}

/// Challenges MsgRing boundary logic with exact-fit, almost-fit, and non-fit payload sizes.
#[test]
fn native_transport_adversarial_boundaries() {
    // A5: Test adversarial boundary conditions (exact-fit, almost-fit, non-fit)
    const RING_CAPACITY: usize = 256;

    // Test exact-fit: payload that exactly fills to capacity when wrapped
    // Record = envelope(8) + payload(N), aligned to 8
    // For 256-byte ring, try payloads that create exact-fit scenarios

    let test_cases = vec![
        ("exact-fit", 240), // 8 + 240 = 248, aligned = 248 (fits exactly with room for next)
        ("almost-fit", 232), // 8 + 232 = 240, leaves small gap
        ("non-fit-forces-wrap", 200), // After partial fill, this forces sentinel
    ];

    for (name, payload_size) in test_cases {
        // Reset ring state by creating fresh instances for each test
        let evt_ring_test = Arc::new(SharedMsgRing::new(RING_CAPACITY, EVENT_ENVELOPE));

        let produced = Arc::new(AtomicU32::new(0));
        let produced_clone = Arc::clone(&produced);

        let evt_ring_prod = Arc::clone(&evt_ring_test);
        let producer_thread = thread::spawn(move || {
            let mut seq = 0u32;
            // Fill the ring multiple times to test wrap scenarios
            for _ in 0..10 {
                let committed = evt_ring_prod.with_mut(|ring| {
                    if let Some(mut grant) = ring.try_reserve(payload_size) {
                        let payload = grant.payload();
                        payload[..4].copy_from_slice(&seq.to_le_bytes());
                        if payload_size >= 8 {
                            payload[4..8].copy_from_slice(&seq.to_le_bytes());
                        }
                        if payload_size > 8 {
                            payload[8..payload_size].fill((seq & 0xFF) as u8);
                        }
                        grant.commit(payload_size);
                        produced_clone.fetch_add(1, Ordering::Release);
                        seq += 1;
                        true
                    } else {
                        false
                    }
                });

                if !committed {
                    break;
                }
            }
        });

        let evt_ring_cons = Arc::clone(&evt_ring_test);
        let produced_cons = Arc::clone(&produced);
        let consumer_thread = thread::spawn(move || {
            let mut received = Vec::new();

            // Keep consuming until we've received all produced records
            loop {
                let result = evt_ring_cons.with_mut(|ring| {
                    if let Some(record) = ring.consumer_peek() {
                        let mut seq_bytes = [0u8; 4];
                        seq_bytes.copy_from_slice(&record.payload[..4]);
                        let seq = u32::from_le_bytes(seq_bytes);
                        ring.consumer_pop_advance();
                        Some(seq)
                    } else {
                        None
                    }
                });

                if let Some(seq) = result {
                    received.push(seq);
                } else {
                    // Check if producer is done
                    let produced_count = produced_cons.load(Ordering::Acquire);
                    if produced_count > 0 && received.len() >= produced_count as usize {
                        break;
                    }
                    // Otherwise yield and try again
                    thread::yield_now();
                }
            }

            received
        });

        producer_thread.join().unwrap();
        let received = consumer_thread.join().unwrap();

        let produced_count = produced.load(Ordering::Acquire);
        assert_eq!(
            received.len(),
            produced_count as usize,
            "{name}: received count mismatch"
        );

        for (i, &seq) in received.iter().enumerate() {
            assert_eq!(seq, i as u32, "{name}: sequence mismatch at index {i}");
        }
    }

    // Final reconciliation not needed as we create fresh rings per test case
}
