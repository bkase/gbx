//! Browser-focused wasm-bindgen tests exercising the SharedArrayBuffer transport.

use std::cell::RefCell;
use std::convert::TryFrom;
use std::ptr::NonNull;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Object, Reflect};
use transport::{Envelope, MsgRing, Record, SlotPool, SlotPoolConfig, SlotPop, TransportError};
use transport_worker::types::{ScenarioStats, TestConfig, WorkerInitDescriptor};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

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

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(JsValue::from_str(&format!($($arg)*)));
        }
    };
}

macro_rules! ensure_eq {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        let left_val = $left;
        let right_val = $right;
        if left_val != right_val {
            return Err(JsValue::from_str(&format!(
                concat!("{}", " != ", "{}", ": ", $($arg)*),
                left_val, right_val
            )));
        }
    };
}

#[wasm_bindgen]
pub fn wasm_smoke_test() -> Result<(), JsValue> {
    let sum: i32 = [1, 1].iter().copied().sum();
    ensure!(sum == 2, "smoke test sum mismatch: {}", sum);
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_flood_frames() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let ticket = harness.start_flood(10_000)?;
    let outcome = harness.consume_frames(10_000, None).await?;
    ensure_eq!(outcome.frames.len(), 10_000, "all frames should drain");
    ensure!(
        outcome.frames == outcome.events,
        "event ring should mirror ready frames"
    );
    let result = ticket.wait().await?;
    ensure!(
        result.status == 0,
        "worker flood result status {}",
        result.status
    );
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing flood stats"))?;
    ensure!(
        stats.produced as usize == 10_000,
        "worker produced {} frames (expected 10_000)",
        stats.produced
    );
    ensure!(
        stats.would_block_ready == 0,
        "flood should not hit ready backpressure"
    );
    ensure!(
        stats.would_block_evt == 0,
        "flood should not congest the event ring"
    );
    harness.assert_reconciliation()?;
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_burst_fairness() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let config = BurstScenario {
        bursts: 40,
        burst_size: 64,
        drain_budget: 8,
    };
    let ticket = harness.start_burst(&config)?;
    let outcome = harness
        .consume_frames(
            (config.bursts * config.burst_size) as usize,
            Some(config.drain_budget as usize),
        )
        .await?;
    ensure!(
        outcome.frames.len() == outcome.events.len(),
        "frame/event counts align ({} vs {})",
        outcome.frames.len(),
        outcome.events.len()
    );
    ensure!(
        outcome.frames == outcome.events,
        "bursty events maintain ordering"
    );
    ensure!(
        outcome.max_ready_depth <= FRAME_SLOT_COUNT,
        "ready ring exceeded slot budget ({} > {})",
        outcome.max_ready_depth,
        FRAME_SLOT_COUNT
    );
    let result = ticket.wait().await?;
    ensure!(result.status == 0, "burst status {}", result.status);
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing burst stats"))?;
    ensure!(
        stats.produced == (config.bursts * config.burst_size),
        "worker burst produced {} frames",
        stats.produced
    );
    ensure!(
        stats.would_block_evt == 0,
        "bursty workload should not overflow event ring"
    );
    harness.assert_reconciliation()?;
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_backpressure_recovery() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let cfg = BackpressureScenario {
        frames: 4096,
        pause_ms: 25,
    };
    let ticket = harness.start_backpressure(&cfg)?;
    TimeoutFuture::new(cfg.pause_ms).await;
    let outcome = harness.consume_frames(cfg.frames as usize, None).await?;
    ensure!(
        outcome.frames.len() == outcome.events.len(),
        "frame/event counts align after recovery ({} vs {})",
        outcome.frames.len(),
        outcome.events.len()
    );
    harness.assert_reconciliation()?;
    let result = ticket.wait().await?;
    ensure!(result.status == 0, "backpressure status {}", result.status);
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing backpressure stats"))?;
    ensure!(
        stats.produced == cfg.frames,
        "worker produced {} frames (expected {})",
        stats.produced,
        cfg.frames
    );
    ensure!(
        stats.would_block_ready > 0 || stats.free_waits > 0,
        "producer should observe backpressure on ready or free rings"
    );
    Ok(())
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
    RegisterTest = 1,
    Run = 2,
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
        let init_msg = make_init_message(buffers.descriptor_ptr(), &memory)?;
        let ticket = WorkerTicket::new(worker.clone(), init_msg)?;
        let status = ticket.wait_status().await?;
        if status != 0 {
            return Err(JsValue::from_str(&format!(
                "transport worker init failed with status {status}"
            )));
        }

        Ok(Self { worker, buffers })
    }

    fn start_flood(&self, frame_count: usize) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::flood(frame_count as u32);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
    }

    fn start_burst(&self, cfg: &BurstScenario) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::burst(cfg.bursts, cfg.burst_size);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
    }

    fn start_backpressure(&self, cfg: &BackpressureScenario) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::backpressure(cfg.frames);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
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

    fn assert_reconciliation(&self) -> Result<(), JsValue> {
        ensure!(
            self.buffers.frame_pool.free_len() == self.buffers.frame_pool.slot_count(),
            "all frame slots should be free (free={}, total={})",
            self.buffers.frame_pool.free_len(),
            self.buffers.frame_pool.slot_count()
        );
        ensure!(
            self.buffers.frame_pool.ready_len() == 0,
            "ready ring drained"
        );
        ensure!(
            self.buffers.evt_ring.consumer_peek().is_none(),
            "event ring should be empty after drain"
        );
        ensure!(
            self.buffers.audio_pool.free_len() == self.buffers.audio_pool.slot_count(),
            "audio slots remain unused and free"
        );
        ensure!(
            self.buffers.audio_pool.ready_len() == 0,
            "audio ready ring should remain empty"
        );
        Ok(())
    }
}

struct DrainOutcome {
    frames: Vec<u32>,
    events: Vec<u32>,
    max_ready_depth: usize,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
struct WorkerRunResult {
    op: u32,
    status: i32,
    stats: Option<ScenarioStats>,
}

/// Runs a fabric test by registering the test engine and continuously polling until complete
struct FabricTestRunner {
    completion: Rc<RefCell<Option<oneshot::Sender<WorkerRunResult>>>>,
    #[allow(dead_code)]
    config: Box<TestConfig>,
    #[allow(dead_code)]
    stats: Box<ScenarioStats>,
}

impl FabricTestRunner {
    fn new(worker: Worker, config: TestConfig, stats: Box<ScenarioStats>) -> Result<Self, JsValue> {
        let config = Box::new(config);
        let completion = Rc::new(RefCell::new(None::<oneshot::Sender<WorkerRunResult>>));

        // Register the test engine
        let register_msg = make_register_test_message(ptr_u32(&*config), ptr_u32(&*stats))?;

        // Kick off background task to run the fabric worker
        let worker_clone = worker.clone();
        let completion_clone = completion.clone();
        let stats_ptr = ptr_u32(&*stats);

        wasm_bindgen_futures::spawn_local(async move {
            // Wait for registration response
            let (reg_sender, reg_receiver) = oneshot::channel::<JsValue>();
            let reg_sender_cell = Rc::new(RefCell::new(Some(reg_sender)));
            let reg_sender_clone = reg_sender_cell.clone();
            let reg_closure = Closure::wrap(Box::new(move |event: MessageEvent| {
                if let Some(sender) = reg_sender_clone.borrow_mut().take() {
                    let _ = sender.send(event.data());
                }
            }) as Box<dyn FnMut(MessageEvent)>);

            worker_clone.set_onmessage(Some(reg_closure.as_ref().unchecked_ref()));
            let _ = worker_clone.post_message(&register_msg);

            let reg_result = match reg_receiver.await {
                Ok(v) => v,
                Err(_) => return,
            };

            let reg_status = get_i32_field(&reg_result, "status").unwrap_or(-1);
            if reg_status != 0 {
                return;
            }

            // Continuously run fabric worker until complete
            loop {
                let run_msg = match make_run_message() {
                    Ok(msg) => msg,
                    Err(_) => break,
                };

                let (run_sender, run_receiver) = oneshot::channel::<JsValue>();
                let run_sender_cell = Rc::new(RefCell::new(Some(run_sender)));
                let run_sender_clone = run_sender_cell.clone();
                let run_closure = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(sender) = run_sender_clone.borrow_mut().take() {
                        let _ = sender.send(event.data());
                    }
                }) as Box<dyn FnMut(MessageEvent)>);

                worker_clone.set_onmessage(Some(run_closure.as_ref().unchecked_ref()));
                let _ = worker_clone.post_message(&run_msg);

                let result = match run_receiver.await {
                    Ok(v) => v,
                    Err(_) => break,
                };

                let work = get_u32_field(&result, "work").unwrap_or(0);

                // If no work done, test is complete
                if work == 0 {
                    break;
                }

                // Yield to let frames be consumed
                TimeoutFuture::new(0).await;
            }

            worker_clone.set_onmessage(None);

            // Read final stats
            let stats = unsafe {
                let ptr = stats_ptr as *const ScenarioStats;
                *ptr
            };

            // Signal completion
            if let Some(sender) = completion_clone.borrow_mut().take() {
                let _ = sender.send(WorkerRunResult {
                    op: 0,
                    status: 0,
                    stats: Some(stats),
                });
            }
        });

        Ok(Self {
            completion,
            config,
            stats,
        })
    }

    async fn wait(self) -> Result<WorkerRunResult, JsValue> {
        let (sender, receiver) = oneshot::channel::<WorkerRunResult>();
        *self.completion.borrow_mut() = Some(sender);

        receiver
            .await
            .map_err(|_| JsValue::from_str("fabric worker task died"))
    }
}

struct WorkerTicket {
    receiver: oneshot::Receiver<JsValue>,
    worker: Worker,
    closure: Option<Closure<dyn FnMut(MessageEvent)>>,
}

impl WorkerTicket {
    fn new(worker: Worker, message: JsValue) -> Result<Self, JsValue> {
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
        })
    }

    async fn wait_status(mut self) -> Result<i32, JsValue> {
        let value = self
            .receiver
            .await
            .map_err(|_| JsValue::from_str("worker dropped channel"))?;
        self.worker.set_onmessage(None);
        if let Some(closure) = self.closure.take() {
            drop(closure);
        }

        let status = get_i32_field(&value, "status")?;
        Ok(status)
    }
}

fn shared_memory() -> Result<js_sys::WebAssembly::Memory, JsValue> {
    wasm_bindgen::memory().dyn_into::<js_sys::WebAssembly::Memory>()
}

fn spawn_worker() -> Result<Worker, JsValue> {
    // Load worker from server instead of blob to allow ES6 imports to work
    let worker_url = "./pkg/worker.js";
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options(worker_url, &options)?;
    Ok(worker)
}

fn make_init_message(
    descriptor_ptr: u32,
    memory: &js_sys::WebAssembly::Memory,
) -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", WorkerOp::Init as u32)?;
    set_u32(&msg, "descriptorPtr", descriptor_ptr)?;
    Reflect::set(&msg, &JsValue::from_str("memory"), memory)?;
    Ok(msg.into())
}

fn make_register_test_message(config_ptr: u32, stats_ptr: u32) -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", WorkerOp::RegisterTest as u32)?;
    set_u32(&msg, "configPtr", config_ptr)?;
    set_u32(&msg, "statsPtr", stats_ptr)?;
    Ok(msg.into())
}

fn make_run_message() -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", WorkerOp::Run as u32)?;
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
