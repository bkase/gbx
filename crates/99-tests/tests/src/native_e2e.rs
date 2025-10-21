#![cfg(all(test, not(target_arch = "wasm32")))]

use parking_lot::Mutex;
use runtime_native::NativeChannels;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use transport_fabric::WorkerRuntime;
use transport_scenarios::{
    verify_backpressure, verify_burst, verify_flood, ArcStatsSink, DrainReport,
    FrameScenarioEngine, ScenarioKind, ScenarioStats,
};

const FRAME_SLOT_COUNT: u32 = 8;

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
    let channels = NativeChannels::new();
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

    let channels = NativeChannels::new();
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
    let channels = NativeChannels::new();
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
