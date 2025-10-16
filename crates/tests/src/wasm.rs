//! Browser-focused wasm-bindgen tests exercising the SharedArrayBuffer transport.

use std::cell::RefCell;
use std::convert::TryFrom;
use std::ptr::NonNull;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Object, Reflect};
use transport::{Envelope, MsgRing, Record, SlotPool, SlotPoolConfig, SlotPop, TransportError};
use transport_worker::types::{
    BackpressureConfig as WorkerBackpressureConfig, BurstConfig as WorkerBurstConfig,
    FloodConfig as WorkerFloodConfig, ScenarioStats, WorkerInitDescriptor,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Blob, BlobPropertyBag, MessageEvent, Url, Worker, WorkerOptions, WorkerType};

wasm_bindgen_test_configure!(run_in_browser);

const WORKER_SOURCE: &str = include_str!("../../../web/worker.js");
const TRANSPORT_WORKER_WASM: &[u8] = include_bytes!("../../../web/pkg/transport_worker.wasm");

const CMD_RING_CAPACITY: usize = 32 * 1024;
const EVT_RING_CAPACITY: usize = 512 * 1024;
const FRAME_SLOT_COUNT: usize = 8;
const FRAME_SLOT_SIZE: usize = 128 * 1024;
const AUDIO_SLOT_COUNT: usize = 16;
const AUDIO_SLOT_SIZE: usize = 32 * 1024;

const EVENT_ENVELOPE: Envelope = Envelope {
    tag: 0x13,
    ver: 1,
    flags: 0,
};

#[wasm_bindgen_test]
fn wasm_smoke_test() {
    let sum: i32 = [1, 1].iter().copied().sum();
    assert_eq!(sum, 2);
}

#[wasm_bindgen_test]
async fn transport_worker_flood_frames() {
    let mut harness = TransportHarness::new().await.expect("init harness");
    let ticket = harness.start_flood(10_000).expect("post flood command");
    let outcome = harness
        .consume_frames(10_000, None)
        .await
        .expect("drain frames");
    assert_eq!(outcome.frames.len(), 10_000, "all frames drained");
    assert_eq!(
        outcome.frames, outcome.events,
        "event ring should mirror ready frames"
    );
    let result = ticket.wait().await.expect("worker flood result");
    assert_eq!(result.status, 0, "worker flood result status");
    let stats = result.stats.expect("flood stats present");
    assert_eq!(
        stats.produced as usize, 10_000,
        "worker produced all frames"
    );
    assert_eq!(
        stats.would_block_ready, 0,
        "flood should not hit ready backpressure"
    );
    assert_eq!(
        stats.would_block_evt, 0,
        "flood should not congest the event ring"
    );
    harness.assert_reconciliation();
}

#[wasm_bindgen_test]
async fn transport_worker_burst_fairness() {
    let mut harness = TransportHarness::new().await.expect("init harness");
    let config = BurstScenario {
        bursts: 40,
        burst_size: 64,
        drain_budget: 8,
    };
    let ticket = harness.start_burst(&config).expect("post burst command");
    let outcome = harness
        .consume_frames(
            (config.bursts * config.burst_size) as usize,
            Some(config.drain_budget as usize),
        )
        .await
        .expect("drain bursts");
    assert_eq!(
        outcome.frames.len(),
        outcome.events.len(),
        "frame/event counts align"
    );
    assert_eq!(
        outcome.frames, outcome.events,
        "bursty events maintain ordering"
    );
    assert!(
        outcome.max_ready_depth <= FRAME_SLOT_COUNT,
        "ready ring never exceeded slot budget"
    );
    let result = ticket.wait().await.expect("worker burst result");
    assert_eq!(result.status, 0, "burst status");
    let stats = result.stats.expect("burst stats");
    assert_eq!(
        stats.produced,
        (config.bursts * config.burst_size) as u32,
        "worker burst produced expected frames"
    );
    assert_eq!(
        stats.would_block_evt, 0,
        "bursty workload should not overflow event ring"
    );
    harness.assert_reconciliation();
}

#[wasm_bindgen_test]
async fn transport_worker_backpressure_recovery() {
    let mut harness = TransportHarness::new().await.expect("init harness");
    let cfg = BackpressureScenario {
        frames: 4096,
        pause_ms: 25,
    };
    let ticket = harness
        .start_backpressure(&cfg)
        .expect("post backpressure command");
    TimeoutFuture::new(cfg.pause_ms).await;
    let outcome = harness
        .consume_frames(cfg.frames as usize, None)
        .await
        .expect("drain after pause");
    assert_eq!(outcome.frames.len(), outcome.events.len());
    harness.assert_reconciliation();
    let result = ticket.wait().await.expect("worker backpressure result");
    assert_eq!(result.status, 0, "backpressure status");
    let stats = result.stats.expect("backpressure stats");
    assert_eq!(
        stats.produced, cfg.frames,
        "worker produced requested frames"
    );
    assert!(
        stats.would_block_ready > 0 || stats.free_waits > 0,
        "producer should observe backpressure on ready or free rings"
    );
}

struct BurstScenario {
    bursts: u32,
    burst_size: u32,
    drain_budget: u32,
}

struct BackpressureScenario {
    frames: u32,
    pause_ms: u32,
}

#[repr(u32)]
#[derive(Clone, Copy)]
enum WorkerOp {
    Init = 0,
    Flood = 1,
    Burst = 2,
    Backpressure = 3,
}

enum WorkerConfig {
    Flood(Box<WorkerFloodConfig>),
    Burst(Box<WorkerBurstConfig>),
    Backpressure(Box<WorkerBackpressureConfig>),
}

struct TransportHarness {
    worker: Worker,
    buffers: TransportBuffers,
}

impl TransportHarness {
    async fn new() -> Result<Self, JsValue> {
        let buffers = TransportBuffers::new().map_err(|err| JsValue::from_str(&err.to_string()))?;
        let memory = shared_memory()?;
        let worker = spawn_worker()?;
        let module = worker_module_buffer();
        let init_msg = make_init_message(buffers.descriptor_ptr(), &memory, &module)?;
        let ticket = WorkerTicket::new(worker.clone(), init_msg, None, None)?;
        let status = ticket.wait_status().await?;
        if status != 0 {
            return Err(JsValue::from_str(&format!(
                "transport worker init failed with status {status}"
            )));
        }

        Ok(Self { worker, buffers })
    }

    fn start_flood(&self, frame_count: usize) -> Result<WorkerTicket, JsValue> {
        let config = Box::new(WorkerFloodConfig {
            frame_count: frame_count as u32,
        });
        let stats = Box::new(ScenarioStats::default());
        let msg = make_run_message(WorkerOp::Flood, ptr_u32(&*config), ptr_u32(&*stats))?;
        WorkerTicket::new(
            self.worker.clone(),
            msg,
            Some(WorkerConfig::Flood(config)),
            Some(stats),
        )
    }

    fn start_burst(&self, cfg: &BurstScenario) -> Result<WorkerTicket, JsValue> {
        let config = Box::new(WorkerBurstConfig {
            bursts: cfg.bursts,
            burst_size: cfg.burst_size,
        });
        let stats = Box::new(ScenarioStats::default());
        let msg = make_run_message(WorkerOp::Burst, ptr_u32(&*config), ptr_u32(&*stats))?;
        WorkerTicket::new(
            self.worker.clone(),
            msg,
            Some(WorkerConfig::Burst(config)),
            Some(stats),
        )
    }

    fn start_backpressure(&self, cfg: &BackpressureScenario) -> Result<WorkerTicket, JsValue> {
        let config = Box::new(WorkerBackpressureConfig { frames: cfg.frames });
        let stats = Box::new(ScenarioStats::default());
        let msg = make_run_message(WorkerOp::Backpressure, ptr_u32(&*config), ptr_u32(&*stats))?;
        WorkerTicket::new(
            self.worker.clone(),
            msg,
            Some(WorkerConfig::Backpressure(config)),
            Some(stats),
        )
    }

    async fn consume_frames(
        &mut self,
        target: usize,
        drain_budget: Option<usize>,
    ) -> Result<DrainOutcome, JsValue> {
        let budget = drain_budget.unwrap_or(usize::MAX);
        let mut frames = Vec::with_capacity(target);
        let mut events = Vec::with_capacity(target);
        let mut max_ready_depth = 0usize;

        while frames.len() < target {
            while let SlotPop::Ok { slot_idx } = self.buffers.frame_pool.pop_ready() {
                let frame_id = read_frame_slot(&mut self.buffers.frame_pool, slot_idx);
                frames.push(frame_id);
                self.buffers.frame_pool.release_free(slot_idx);
            }

            let mut drained = 0usize;
            while drained < budget {
                if let Some(record) = self.buffers.evt_ring.consumer_peek() {
                    events.push(parse_event_record(&record));
                    drained += 1;
                    self.buffers.evt_ring.consumer_pop_advance();
                } else {
                    break;
                }
            }

            let in_use = self.buffers.frame_pool.slot_count() as usize
                - self.buffers.frame_pool.free_len() as usize;
            max_ready_depth = max_ready_depth.max(in_use);
            TimeoutFuture::new(0).await;
        }

        while let SlotPop::Ok { slot_idx } = self.buffers.frame_pool.pop_ready() {
            let frame_id = read_frame_slot(&mut self.buffers.frame_pool, slot_idx);
            frames.push(frame_id);
            self.buffers.frame_pool.release_free(slot_idx);
        }

        while let Some(record) = self.buffers.evt_ring.consumer_peek() {
            events.push(parse_event_record(&record));
            self.buffers.evt_ring.consumer_pop_advance();
        }

        Ok(DrainOutcome {
            frames,
            events,
            max_ready_depth,
        })
    }

    fn assert_reconciliation(&self) {
        assert_eq!(
            self.buffers.frame_pool.free_len(),
            self.buffers.frame_pool.slot_count(),
            "all frame slots should be free"
        );
        assert_eq!(self.buffers.frame_pool.ready_len(), 0, "ready ring drained");
        assert!(
            self.buffers.evt_ring.consumer_peek().is_none(),
            "event ring should be empty after drain"
        );
        assert_eq!(
            self.buffers.audio_pool.free_len(),
            self.buffers.audio_pool.slot_count(),
            "audio slots remain unused and free"
        );
        assert_eq!(
            self.buffers.audio_pool.ready_len(),
            0,
            "audio ready ring should remain empty"
        );
    }
}

struct DrainOutcome {
    frames: Vec<u32>,
    events: Vec<u32>,
    max_ready_depth: usize,
}

struct TransportBuffers {
    cmd_ring: MsgRing,
    evt_ring: MsgRing,
    frame_pool: SlotPool,
    audio_pool: SlotPool,
    descriptor: NonNull<WorkerInitDescriptor>,
}

impl TransportBuffers {
    fn new() -> Result<Self, TransportError> {
        let cmd_ring = MsgRing::new(CMD_RING_CAPACITY, Envelope::new(0x01, 1))?;
        let evt_ring = MsgRing::new(EVT_RING_CAPACITY, EVENT_ENVELOPE)?;
        let frame_pool = SlotPool::new(SlotPoolConfig {
            slot_count: FRAME_SLOT_COUNT as u32,
            slot_size: FRAME_SLOT_SIZE,
        })?;
        let audio_pool = SlotPool::new(SlotPoolConfig {
            slot_count: AUDIO_SLOT_COUNT as u32,
            slot_size: AUDIO_SLOT_SIZE,
        })?;
        let descriptor = Box::new(WorkerInitDescriptor {
            cmd_ring: cmd_ring.wasm_layout(),
            evt_ring: evt_ring.wasm_layout(),
            frame_pool: frame_pool.wasm_layout(),
            audio_pool: audio_pool.wasm_layout(),
        });
        let descriptor = unsafe { NonNull::new_unchecked(Box::into_raw(descriptor)) };
        Ok(Self {
            cmd_ring,
            evt_ring,
            frame_pool,
            audio_pool,
            descriptor,
        })
    }

    fn descriptor_ptr(&self) -> u32 {
        ptr_u32(unsafe { self.descriptor.as_ref() })
    }
}

impl Drop for TransportBuffers {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.descriptor.as_ptr()));
        }
    }
}

struct WorkerRunResult {
    op: u32,
    status: i32,
    stats: Option<ScenarioStats>,
}

struct WorkerTicket {
    receiver: oneshot::Receiver<JsValue>,
    worker: Worker,
    closure: Option<Closure<dyn FnMut(MessageEvent)>>,
    _config: Option<WorkerConfig>,
    stats: Option<Box<ScenarioStats>>,
}

impl WorkerTicket {
    fn new(
        worker: Worker,
        message: JsValue,
        config: Option<WorkerConfig>,
        stats: Option<Box<ScenarioStats>>,
    ) -> Result<Self, JsValue> {
        let (sender, receiver) = oneshot::channel::<JsValue>();
        let sender_cell = Rc::new(RefCell::new(Some(sender)));
        let sender_clone = sender_cell.clone();
        let closure = Closure::wrap(Box::new(move |event: MessageEvent| {
            if let Some(sender) = sender_clone.borrow_mut().take() {
                let _ = sender.send(event.data());
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        worker.set_onmessage(Some(closure.as_ref().unchecked_ref()));
        worker.post_message(&message)?;

        Ok(Self {
            receiver,
            worker,
            closure: Some(closure),
            _config: config,
            stats,
        })
    }

    async fn wait(mut self) -> Result<WorkerRunResult, JsValue> {
        let value = self
            .receiver
            .await
            .map_err(|_| JsValue::from_str("worker dropped channel"))?;
        self.worker.set_onmessage(None);
        if let Some(closure) = self.closure.take() {
            drop(closure);
        }

        let status = get_i32_field(&value, "status")?;
        let op = get_u32_field(&value, "op")?;
        let stats = self.stats.map(|boxed| *boxed);

        Ok(WorkerRunResult { op, status, stats })
    }

    async fn wait_status(self) -> Result<i32, JsValue> {
        let result = self.wait().await?;
        Ok(result.status)
    }
}

fn shared_memory() -> Result<js_sys::WebAssembly::Memory, JsValue> {
    wasm_bindgen::memory().dyn_into::<js_sys::WebAssembly::Memory>()
}

fn spawn_worker() -> Result<Worker, JsValue> {
    let sources = js_sys::Array::new();
    sources.push(&JsValue::from_str(WORKER_SOURCE));
    let bag = BlobPropertyBag::new();
    bag.set_type("application/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&sources, &bag)?;
    let url = Url::create_object_url_with_blob(&blob)?;
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options(&url, &options)?;
    Url::revoke_object_url(&url)?;
    Ok(worker)
}

fn worker_module_buffer() -> js_sys::ArrayBuffer {
    let bytes = TRANSPORT_WORKER_WASM;
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    array.buffer()
}

fn make_init_message(
    descriptor_ptr: u32,
    memory: &js_sys::WebAssembly::Memory,
    module: &js_sys::ArrayBuffer,
) -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", WorkerOp::Init as u32)?;
    set_u32(&msg, "descriptorPtr", descriptor_ptr)?;
    Reflect::set(&msg, &JsValue::from_str("memory"), memory)?;
    Reflect::set(&msg, &JsValue::from_str("module"), module)?;
    Ok(msg.into())
}

fn make_run_message(op: WorkerOp, config_ptr: u32, stats_ptr: u32) -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", op as u32)?;
    set_u32(&msg, "configPtr", config_ptr)?;
    set_u32(&msg, "statsPtr", stats_ptr)?;
    Ok(msg.into())
}

fn set_u32(target: &Object, key: &str, value: u32) -> Result<(), JsValue> {
    Reflect::set(target, &JsValue::from_str(key), &JsValue::from(value))?;
    Ok(())
}

fn get_u32_field(value: &JsValue, key: &str) -> Result<u32, JsValue> {
    let field = Reflect::get(value, &JsValue::from_str(key))?;
    field
        .as_f64()
        .ok_or_else(|| JsValue::from_str(&format!("expected number field '{key}'")))
        .map(|number| number as u32)
}

fn get_i32_field(value: &JsValue, key: &str) -> Result<i32, JsValue> {
    let field = Reflect::get(value, &JsValue::from_str(key))?;
    field
        .as_f64()
        .ok_or_else(|| JsValue::from_str(&format!("expected number field '{key}'")))
        .map(|number| number as i32)
}

fn ptr_u32<T>(value: &T) -> u32 {
    let addr = value as *const T as usize;
    u32::try_from(addr).expect("wasm pointers fit in u32")
}

fn parse_event_record(record: &Record<'_>) -> u32 {
    const FRAME_ID_BYTES: usize = 4;
    if record.payload.len() < FRAME_ID_BYTES {
        panic!("event payload too short: {}", record.payload.len());
    }
    read_u32(record.payload, 0)
}

fn read_frame_slot(pool: &mut SlotPool, slot_idx: u32) -> u32 {
    let slot = pool.slot_mut(slot_idx);
    read_u32(slot, 0)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let slice = &bytes[offset..offset + 4];
    u32::from_le_bytes(slice.try_into().expect("slice length is 4"))
}
