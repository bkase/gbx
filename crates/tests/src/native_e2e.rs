#![cfg(all(test, not(target_arch = "wasm32")))]

use parking_lot::Mutex;
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use transport::{Envelope, MsgRing, SlotPool, SlotPoolConfig, SlotPop, SlotPush};
use transport_fabric::WorkerRuntime;
use transport_scenarios::{
    verify_backpressure, verify_burst, verify_flood, ArcStatsSink, DrainReport, FabricHandle,
    FrameScenarioEngine, ScenarioKind, ScenarioStats,
};

const EVT_RING_CAPACITY: usize = 512 * 1024;
const FRAME_SLOT_COUNT: u32 = 8;
const FRAME_SLOT_SIZE: usize = 128 * 1024;
const EVENT_PAYLOAD_LEN: usize = 8;
const EVENT_ENVELOPE: Envelope = Envelope {
    tag: transport_scenarios::EVENT_TAG,
    ver: transport_scenarios::EVENT_VER,
    flags: 0,
};

struct SharedSlotPool(UnsafeCell<SlotPool>);

// SAFETY: `SharedSlotPool` only exposes interior mutability via explicit methods that
// serialise access; the underlying `SlotPool` is Send/Sync once guarded this way.
unsafe impl Send for SharedSlotPool {}
// SAFETY: See above; all shared access paths go through `with_mut`, preserving exclusivity.
unsafe impl Sync for SharedSlotPool {}

impl SharedSlotPool {
    fn new(config: SlotPoolConfig) -> Self {
        let pool = SlotPool::new(config).expect("create slot pool");
        Self(UnsafeCell::new(pool))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut SlotPool) -> R) -> R {
        // SAFETY: `UnsafeCell` grants exclusive access; callers must not alias the pool.
        unsafe { f(&mut *self.0.get()) }
    }
}

struct SharedMsgRing(UnsafeCell<MsgRing>);

// SAFETY: `SharedMsgRing` mediates access through `with_mut`, so cross-thread usage respects
// the ring's interior mutability requirements.
unsafe impl Send for SharedMsgRing {}
// SAFETY: As above, shared references can only act via controlled mutable access.
unsafe impl Sync for SharedMsgRing {}

impl SharedMsgRing {
    fn new(capacity: usize, default_envelope: Envelope) -> Self {
        let ring = MsgRing::new(capacity, default_envelope).expect("create msg ring");
        Self(UnsafeCell::new(ring))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut MsgRing) -> R) -> R {
        // SAFETY: Exclusive mutable reference is produced while holding no other aliases.
        unsafe { f(&mut *self.0.get()) }
    }
}

struct TransportChannels {
    frame_pool: Arc<SharedSlotPool>,
    evt_ring: Arc<SharedMsgRing>,
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

    fn handle(&self) -> NativeFabricHandle {
        NativeFabricHandle {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        }
    }

    fn consumer(&self) -> FrameConsumer {
        FrameConsumer {
            frame_pool: Arc::clone(&self.frame_pool),
            evt_ring: Arc::clone(&self.evt_ring),
        }
    }

    fn assert_reconciliation(&self) {
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

struct NativeFabricHandle {
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

    fn read_slot_seq(&self, slot_idx: u32) -> u32 {
        self.frame_pool.with_mut(|pool| {
            let slot = pool.slot_mut(slot_idx);
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&slot[..4]);
            u32::from_le_bytes(bytes)
        })
    }

    fn release_slot(&self, slot_idx: u32) {
        self.frame_pool.with_mut(|pool| pool.release_free(slot_idx));
    }

    fn try_pop_event(&self) -> Option<(u32, u32)> {
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

fn run_runtime_until(
    stats: Arc<Mutex<ScenarioStats>>,
    mut runtime: WorkerRuntime,
    target: u32,
) -> thread::JoinHandle<()> {
    let stats_clone = Arc::clone(&stats);
    thread::spawn(move || loop {
        if stats_clone.lock().produced >= target {
            break;
        }
        let work = runtime.run_tick();
        if work == 0 {
            thread::yield_now();
        }
    })
}

#[test]
fn native_fabric_flood_frames() {
    const FRAME_TARGET: u32 = 10_000;
    let channels = TransportChannels::new();
    let consumer = channels.consumer();
    let stats = Arc::new(Mutex::new(ScenarioStats::default()));
    let mut runtime = WorkerRuntime::new();
    runtime.register(FrameScenarioEngine::new(
        channels.handle(),
        ArcStatsSink::new(Arc::clone(&stats)),
        ScenarioKind::Flood {
            frame_count: FRAME_TARGET,
        },
    ));

    let runtime_handle = run_runtime_until(Arc::clone(&stats), runtime, FRAME_TARGET);

    let mut frames = Vec::with_capacity(FRAME_TARGET as usize);
    let mut events = Vec::with_capacity(FRAME_TARGET as usize);

    while frames.len() < FRAME_TARGET as usize {
        if let Some(slot_idx) = consumer.pop_ready() {
            let (frame_id, evt_slot_idx) = loop {
                if let Some(pair) = consumer.try_pop_event() {
                    break pair;
                }
                thread::yield_now();
            };
            assert_eq!(slot_idx, evt_slot_idx, "event slot mismatch");
            let payload = consumer.read_slot_seq(slot_idx);
            assert_eq!(payload, frame_id, "frame payload mismatch");
            consumer.release_slot(slot_idx);
            frames.push(frame_id);
            events.push(frame_id);
        } else {
            thread::yield_now();
        }
    }

    runtime_handle.join().unwrap();

    let stats_guard = stats.lock();
    let drain = DrainReport {
        frames: &frames,
        events: &events,
        max_ready_depth: None,
    };
    verify_flood(&drain, &stats_guard, FRAME_TARGET).expect("flood verification");
    channels.assert_reconciliation();
}

#[test]
fn native_fabric_burst_fairness() {
    const BURSTS: u32 = 40;
    const BURST_SIZE: u32 = 64;
    const DRAIN_BUDGET: usize = 8;
    let total_frames = (BURSTS * BURST_SIZE) as usize;

    let channels = TransportChannels::new();
    let consumer = channels.consumer();
    let stats = Arc::new(Mutex::new(ScenarioStats::default()));
    let mut runtime = WorkerRuntime::new();
    runtime.register(FrameScenarioEngine::new(
        channels.handle(),
        ArcStatsSink::new(Arc::clone(&stats)),
        ScenarioKind::Burst {
            bursts: BURSTS,
            burst_size: BURST_SIZE,
        },
    ));

    let runtime_handle = run_runtime_until(Arc::clone(&stats), runtime, BURSTS * BURST_SIZE);

    let mut frames = Vec::with_capacity(total_frames);
    let mut events = Vec::with_capacity(total_frames);

    while frames.len() < total_frames {
        let mut drained = 0usize;
        while drained < DRAIN_BUDGET && frames.len() < total_frames {
            if let Some(slot_idx) = consumer.pop_ready() {
                let (frame_id, evt_slot_idx) = loop {
                    if let Some(pair) = consumer.try_pop_event() {
                        break pair;
                    }
                    thread::yield_now();
                };
                assert_eq!(slot_idx, evt_slot_idx, "burst slot mismatch");
                let payload = consumer.read_slot_seq(slot_idx);
                assert_eq!(payload, frame_id, "burst payload mismatch");
                consumer.release_slot(slot_idx);
                frames.push(frame_id);
                events.push(frame_id);
                drained += 1;
            } else {
                break;
            }
        }
        if drained == 0 {
            thread::yield_now();
        }
    }

    runtime_handle.join().unwrap();

    let stats_guard = stats.lock();
    let drain = DrainReport {
        frames: &frames,
        events: &events,
        max_ready_depth: None,
    };
    verify_burst(
        &drain,
        &stats_guard,
        BURSTS * BURST_SIZE,
        FRAME_SLOT_COUNT as usize,
    )
    .expect("burst verification");
    channels.assert_reconciliation();
}

#[test]
fn native_fabric_backpressure_recovery() {
    const FRAMES: u32 = 4_096;
    const PAUSE_AFTER: usize = 12;
    let channels = TransportChannels::new();
    let consumer = channels.consumer();
    let stats = Arc::new(Mutex::new(ScenarioStats::default()));
    let mut runtime = WorkerRuntime::new();
    runtime.register(FrameScenarioEngine::new(
        channels.handle(),
        ArcStatsSink::new(Arc::clone(&stats)),
        ScenarioKind::Backpressure { frames: FRAMES },
    ));

    let runtime_handle = run_runtime_until(Arc::clone(&stats), runtime, FRAMES);

    let mut frames = Vec::with_capacity(FRAMES as usize);
    let mut events = Vec::with_capacity(FRAMES as usize);

    while frames.len() < FRAMES as usize {
        if let Some(slot_idx) = consumer.pop_ready() {
            let (frame_id, evt_slot_idx) = loop {
                if let Some(pair) = consumer.try_pop_event() {
                    break pair;
                }
                thread::yield_now();
            };
            assert_eq!(slot_idx, evt_slot_idx, "backpressure slot mismatch");
            let payload = consumer.read_slot_seq(slot_idx);
            assert_eq!(payload, frame_id, "backpressure payload mismatch");
            consumer.release_slot(slot_idx);
            frames.push(frame_id);
            events.push(frame_id);

            if frames.len() == PAUSE_AFTER {
                thread::sleep(Duration::from_millis(25));
            }
        } else {
            thread::yield_now();
        }
    }

    runtime_handle.join().unwrap();

    let stats_guard = stats.lock();
    let drain = DrainReport {
        frames: &frames,
        events: &events,
        max_ready_depth: None,
    };
    verify_backpressure(&drain, &stats_guard, FRAMES).expect("backpressure verification");
    channels.assert_reconciliation();
}
