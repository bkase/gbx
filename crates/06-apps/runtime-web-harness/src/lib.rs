#![cfg_attr(
    not(target_arch = "wasm32"),
    allow(missing_docs, unused_macros, unused_mut, dead_code)
)]
#![cfg(target_arch = "wasm32")]
#![deny(missing_docs)]
//! Browser-side harness used by wasm transport integration tests.
//!
//! This mirrors the previous inline test harness and exposes reusable helpers
//! so tests only host thin wasm-bindgen wrappers.

use std::cell::RefCell;
use std::convert::TryFrom;
use std::rc::Rc;
use std::sync::Arc;

pub use fabric_worker_wasm::types::{ScenarioStats, TestConfig};
use futures::channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Object, Reflect};
use rkyv::rancor::Error;
use services_fabric::TransportServices;
use transport::{Envelope, MsgRing, Record, SlotPool, SlotPoolHandle, SlotPop};
use transport_codecs::KernelCodec;
use transport_fabric::{EndpointHandle, PortLayout, PortRole};
use transport_scenarios::{
    verify_backpressure, verify_burst, verify_flood, CheckResult, DrainReport, EVENT_TAG, EVENT_VER,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

/// Convenience macros mirroring the original harness behaviour.
macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(JsValue::from_str(&format!($($arg)*)));
        }
    };
}

const EVENT_ENVELOPE: Envelope = Envelope {
    tag: EVENT_TAG,
    ver: EVENT_VER,
    flags: 0,
};

/// Wrapper around the wasm worker harness for integration testing.
pub struct TransportHarness {
    worker: Worker,
    _services: TransportServices,
    kernel_endpoint: EndpointHandle<KernelCodec>,
    event_ring: MsgRing,
    frame_pool: Arc<SlotPoolHandle>,
    audio_pool: Arc<SlotPoolHandle>,
    /// Number of frame slots.
    pub frame_slot_count: usize,
    #[allow(dead_code)]
    audio_slot_count: usize,
    _layout_bytes: Vec<u8>,
}

impl TransportHarness {
    /// Creates a new harness instance and initialises the wasm worker.
    pub async fn new() -> Result<Self, JsValue> {
        let services = TransportServices::new()
            .map_err(|err| JsValue::from_str(&format!("transport services init failed: {err}")))?;
        let kernel_endpoint = services.scheduler.kernel.clone();

        let frame_pool = kernel_endpoint
            .slot_pools()
            .get(0)
            .cloned()
            .ok_or_else(|| JsValue::from_str("missing frame slot pool"))?;
        let audio_pool = kernel_endpoint
            .slot_pools()
            .get(1)
            .cloned()
            .ok_or_else(|| JsValue::from_str("missing audio slot pool"))?;

        let frame_slot_count = frame_pool.with_ref(|pool| pool.slot_count() as usize);
        let audio_slot_count = audio_pool.with_ref(|pool| pool.slot_count() as usize);

        let layout_bytes = rkyv::to_bytes::<Error>(&services.worker.layout)
            .map_err(|err| JsValue::from_str(&format!("serialize layout failed: {err}")))?
            .into_vec();

        let kernel_layout = services
            .worker
            .layout
            .endpoints
            .get(0)
            .expect("kernel endpoint layout missing");
        let replies_layout = kernel_layout
            .ports
            .iter()
            .find_map(|(role, layout)| match (role, layout) {
                (PortRole::Replies, PortLayout::MsgRing(ring_layout)) => Some(*ring_layout),
                _ => None,
            })
            .expect("kernel replies layout missing");
        let event_ring = unsafe { MsgRing::from_wasm_layout(replies_layout, EVENT_ENVELOPE) };

        let memory = shared_memory()?;
        let worker = spawn_worker()?;
        let layout_ptr = ptr_from_bytes(&layout_bytes);
        let layout_len = layout_bytes.len() as u32;
        let init_msg = make_init_message(layout_ptr, layout_len, &memory)?;
        let ticket = WorkerTicket::new(worker.clone(), init_msg)?;
        let status = ticket.wait_status().await?;
        if status != 0 {
            return Err(JsValue::from_str(&format!(
                "transport worker init failed with status {status}"
            )));
        }

        Ok(Self {
            worker,
            _services: services,
            kernel_endpoint,
            event_ring,
            frame_pool,
            audio_pool,
            frame_slot_count,
            audio_slot_count,
            _layout_bytes: layout_bytes,
        })
    }

    /// Starts the flood scenario on the background worker.
    pub fn start_flood(&self, frame_count: usize) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::flood(frame_count as u32);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
    }

    /// Starts the burst fairness scenario.
    pub fn start_burst(&self, cfg: &BurstScenario) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::burst(cfg.bursts, cfg.burst_size);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
    }

    /// Starts the backpressure scenario.
    pub fn start_backpressure(
        &self,
        cfg: &BackpressureScenario,
    ) -> Result<FabricTestRunner, JsValue> {
        let config = TestConfig::backpressure(cfg.frames);
        let stats = Box::new(ScenarioStats::default());
        FabricTestRunner::new(self.worker.clone(), config, stats)
    }

    /// Consumes frames/events until `target` frames observed.
    pub async fn consume_frames(
        &mut self,
        target: usize,
        drain_budget: Option<usize>,
    ) -> Result<DrainOutcome, JsValue> {
        let budget = drain_budget.unwrap_or(usize::MAX);
        let mut frames = Vec::with_capacity(target);
        let mut events = Vec::with_capacity(target);
        let mut max_ready_depth = 0usize;

        while frames.len() < target {
            self.frame_pool.with_mut(|pool| {
                while let SlotPop::Ok { slot_idx } = pool.pop_ready() {
                    let frame_id = read_frame_slot(pool, slot_idx);
                    frames.push(frame_id);
                    pool.release_free(slot_idx);
                }

                let in_use = pool.slot_count() as usize - pool.free_len() as usize;
                max_ready_depth = max_ready_depth.max(in_use);
            });

            let mut drained = 0usize;
            while drained < budget {
                if let Some(record) = self.event_ring.consumer_peek() {
                    events.push(parse_event_record(&record));
                    drained += 1;
                    self.event_ring.consumer_pop_advance();
                } else {
                    break;
                }
            }

            TimeoutFuture::new(0).await;
        }

        self.frame_pool.with_mut(|pool| {
            while let SlotPop::Ok { slot_idx } = pool.pop_ready() {
                let frame_id = read_frame_slot(pool, slot_idx);
                frames.push(frame_id);
                pool.release_free(slot_idx);
            }
        });

        while let Some(record) = self.event_ring.consumer_peek() {
            events.push(parse_event_record(&record));
            self.event_ring.consumer_pop_advance();
        }

        Ok(DrainOutcome {
            frames,
            events,
            max_ready_depth,
        })
    }

    /// Ensures all rings reconcile at the end of the test.
    #[cfg(target_arch = "wasm32")]
    pub fn assert_reconciliation(&self) -> Result<(), JsValue> {
        self.frame_pool.with_ref(|pool| -> Result<(), JsValue> {
            ensure!(
                pool.free_len() == pool.slot_count(),
                "all frame slots should be free (free={}, total={})",
                pool.free_len(),
                pool.slot_count()
            );
            ensure!(pool.ready_len() == 0, "ready ring drained");
            Ok(())
        })?;
        self.audio_pool.with_ref(|pool| -> Result<(), JsValue> {
            ensure!(
                pool.free_len() == pool.slot_count(),
                "audio slots remain unused and free (free={}, total={})",
                pool.free_len(),
                pool.slot_count()
            );
            ensure!(
                pool.ready_len() == 0,
                "audio ready ring should remain empty"
            );
            Ok(())
        })?;
        ensure!(
            self.event_ring.consumer_peek().is_none(),
            "event ring should be empty after drain"
        );
        let remaining = self
            .kernel_endpoint
            .drain_reports(usize::MAX)
            .map_err(|err| JsValue::from_str(&format!("kernel drain_reports failed: {err}")))?;
        ensure!(
            remaining.is_empty(),
            "kernel reply ring should be empty after drain"
        );
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn assert_reconciliation(&self) -> Result<(), JsValue> {
        Ok(())
    }
}

/// Result of draining frames and events from the harness.
pub struct DrainOutcome {
    /// Emitted frame sequence numbers.
    pub frames: Vec<u32>,
    /// Corresponding event payloads.
    pub events: Vec<u32>,
    /// Maximum ready depth observed during the run.
    pub max_ready_depth: usize,
}

/// Configuration for the burst scenario.
pub struct BurstScenario {
    /// Number of bursts.
    pub bursts: u32,
    /// Frames per burst.
    pub burst_size: u32,
    /// Drain budget per tick.
    pub drain_budget: u32,
}

/// Configuration for the backpressure scenario.
pub struct BackpressureScenario {
    /// Total frames to emit.
    pub frames: u32,
    /// Pause delay (ms) between draining iterations.
    pub pause_ms: u32,
}

#[repr(u32)]
enum WorkerOp {
    Init = 0,
    RegisterTest = 1,
    #[allow(dead_code)]
    RegisterServices = 2,
    Run = 3,
}

/// Runs a fabric test by registering the engine and polling until completion.
pub struct FabricTestRunner {
    completion: Rc<RefCell<Option<oneshot::Sender<WorkerRunResult>>>>,
    _config: Box<TestConfig>,
    _stats: Box<ScenarioStats>,
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
            _config: config,
            _stats: stats,
        })
    }

    /// Waits for the worker to finish executing the scenario.
    pub async fn wait(self) -> Result<WorkerRunResult, JsValue> {
        let (sender, receiver) = oneshot::channel::<WorkerRunResult>();
        *self.completion.borrow_mut() = Some(sender);

        receiver
            .await
            .map_err(|_| JsValue::from_str("fabric worker task died"))
    }
}

/// Ticket returned for asynchronous worker operations.
pub struct WorkerTicket {
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

#[allow(dead_code)]
/// Result returned after the worker finishes executing a scenario.
pub struct WorkerRunResult {
    /// Operation identifier returned by the worker.
    pub op: u32,
    /// Worker exit status (0 indicates success).
    pub status: i32,
    /// Optional scenario stats captured by the worker.
    pub stats: Option<ScenarioStats>,
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
    layout_ptr: u32,
    layout_len: u32,
    memory: &js_sys::WebAssembly::Memory,
) -> Result<JsValue, JsValue> {
    let msg = Object::new();
    set_u32(&msg, "op", WorkerOp::Init as u32)?;
    set_u32(&msg, "layoutPtr", layout_ptr)?;
    set_u32(&msg, "layoutLen", layout_len)?;
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

fn ptr_from_bytes(bytes: &[u8]) -> u32 {
    let addr = bytes.as_ptr() as usize;
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

/// Verifies a flood run by inspecting the stats and drain outcome.
pub fn verify_flood_run(
    outcome: &DrainOutcome,
    stats: &ScenarioStats,
    frame_target: u32,
) -> Result<(), JsValue> {
    let drain = DrainReport {
        frames: &outcome.frames,
        events: &outcome.events,
        max_ready_depth: Some(outcome.max_ready_depth),
    };
    check(verify_flood(&drain, stats, frame_target))
}

/// Verifies burst fairness after collecting the drain outcome.
pub fn verify_burst_run(
    outcome: &DrainOutcome,
    stats: &ScenarioStats,
    cfg: &BurstScenario,
    slot_count: usize,
) -> Result<(), JsValue> {
    let drain = DrainReport {
        frames: &outcome.frames,
        events: &outcome.events,
        max_ready_depth: Some(outcome.max_ready_depth),
    };
    check(verify_burst(
        &drain,
        stats,
        cfg.bursts * cfg.burst_size,
        slot_count,
    ))
}

/// Verifies the backpressure scenario.
pub fn verify_backpressure_run(
    outcome: &DrainOutcome,
    stats: &ScenarioStats,
    frames: u32,
) -> Result<(), JsValue> {
    let drain = DrainReport {
        frames: &outcome.frames,
        events: &outcome.events,
        max_ready_depth: None,
    };
    check(verify_backpressure(&drain, stats, frames))
}

fn check(res: CheckResult) -> Result<(), JsValue> {
    res.map_err(|msg| JsValue::from_str(&msg))
}
