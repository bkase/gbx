//! Browser-focused wasm-bindgen tests exercising the SharedArrayBuffer transport.

use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_timers::future::TimeoutFuture;
use js_sys::{Object, Reflect, SharedArrayBuffer};
use transport::wasm::{IndexRingLayout, MsgRingLayout, Region};
use transport::{Envelope, MsgRing, Record, SlotPool, SlotPoolConfig, SlotPop, TransportError};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Blob, BlobPropertyBag, MessageEvent, Url, Worker, WorkerOptions, WorkerType};

wasm_bindgen_test_configure!(run_in_browser);

const WORKER_SOURCE: &str = include_str!("../../../web/worker.js");

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
    let produced = get_u32(&result, "produced");
    assert_eq!(produced as usize, 10_000, "worker produced all frames");
    let would_block = get_u32(&result, "wouldBlockReady");
    assert_eq!(would_block, 0, "flood should not hit ready backpressure");
    let would_block_evt = get_u32(&result, "wouldBlockEvt");
    assert_eq!(
        would_block_evt, 0,
        "flood should not congest the event ring"
    );
    harness.assert_reconciliation();
}

#[wasm_bindgen_test]
async fn transport_worker_burst_fairness() {
    let mut harness = TransportHarness::new().await.expect("init harness");
    let config = BurstConfig {
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
    let produced = get_u32(&result, "produced");
    assert_eq!(
        produced,
        (config.bursts * config.burst_size) as u32,
        "worker burst produced expected frames"
    );
    let would_block_evt = get_u32(&result, "wouldBlockEvt");
    assert_eq!(
        would_block_evt, 0,
        "bursty workload should not overflow event ring"
    );
    harness.assert_reconciliation();
}

#[wasm_bindgen_test]
async fn transport_worker_backpressure_recovery() {
    let mut harness = TransportHarness::new().await.expect("init harness");
    let cfg = BackpressureConfig {
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
    let produced = get_u32(&result, "produced");
    assert_eq!(produced, cfg.frames, "worker produced requested frames");
    let would_block_ready = get_u32(&result, "wouldBlockReady");
    let free_waits = get_u32(&result, "freeWaits");
    assert!(
        would_block_ready > 0 || free_waits > 0,
        "producer should observe backpressure on ready or free rings"
    );
}

struct BurstConfig {
    bursts: u32,
    burst_size: u32,
    drain_budget: u32,
}

struct BackpressureConfig {
    frames: u32,
    pause_ms: u32,
}

struct TransportHarness {
    worker: Worker,
    _cmd_ring: MsgRing,
    evt_ring: MsgRing,
    frame_pool: SlotPool,
    audio_pool: SlotPool,
}

impl TransportHarness {
    async fn new() -> Result<Self, JsValue> {
        let buffers = TransportBuffers::new().map_err(|err| JsValue::from_str(&err.to_string()))?;
        let layout = buffers.layout_object()?;
        let config = buffers.config_object()?;
        let memory = shared_memory_buffer()?;

        let worker = spawn_worker()?;
        let init = make_message("init", Some(memory), Some(layout), Some(config));
        let ticket = WorkerTicket::new(worker.clone(), init)?;
        ticket.wait().await?;

        Ok(Self {
            worker,
            _cmd_ring: buffers.cmd_ring,
            evt_ring: buffers.evt_ring,
            frame_pool: buffers.frame_pool,
            audio_pool: buffers.audio_pool,
        })
    }

    fn start_flood(&self, frame_count: usize) -> Result<WorkerTicket, JsValue> {
        let cfg = object_with_u32("frameCount", frame_count as u32)?;
        let msg = make_message("flood", None, None, Some(cfg));
        WorkerTicket::new(self.worker.clone(), msg)
    }

    fn start_burst(&self, cfg: &BurstConfig) -> Result<WorkerTicket, JsValue> {
        let msg_cfg = Object::new();
        Reflect::set(
            &msg_cfg,
            &JsValue::from_str("bursts"),
            &JsValue::from(cfg.bursts),
        )?;
        Reflect::set(
            &msg_cfg,
            &JsValue::from_str("burstSize"),
            &JsValue::from(cfg.burst_size),
        )?;
        Reflect::set(
            &msg_cfg,
            &JsValue::from_str("drainBudget"),
            &JsValue::from(cfg.drain_budget),
        )?;
        let msg = make_message("burst", None, None, Some(msg_cfg));
        WorkerTicket::new(self.worker.clone(), msg)
    }

    fn start_backpressure(&self, cfg: &BackpressureConfig) -> Result<WorkerTicket, JsValue> {
        let msg_cfg = Object::new();
        Reflect::set(
            &msg_cfg,
            &JsValue::from_str("frames"),
            &JsValue::from(cfg.frames),
        )?;
        Reflect::set(
            &msg_cfg,
            &JsValue::from_str("pauseMs"),
            &JsValue::from(cfg.pause_ms),
        )?;
        let msg = make_message("backpressure", None, None, Some(msg_cfg));
        WorkerTicket::new(self.worker.clone(), msg)
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
            while let SlotPop::Ok { slot_idx } = self.frame_pool.pop_ready() {
                let frame_id = read_frame_slot(&mut self.frame_pool, slot_idx);
                frames.push(frame_id);
                self.frame_pool.release_free(slot_idx);
            }

            let mut drained = 0usize;
            while drained < budget {
                if let Some(record) = self.evt_ring.consumer_peek() {
                    events.push(parse_event_record(&record));
                    drained += 1;
                    self.evt_ring.consumer_pop_advance();
                } else {
                    break;
                }
            }

            let in_use =
                self.frame_pool.slot_count() as usize - self.frame_pool.free_len() as usize;
            max_ready_depth = max_ready_depth.max(in_use);
            TimeoutFuture::new(0).await;
        }

        while let SlotPop::Ok { slot_idx } = self.frame_pool.pop_ready() {
            let frame_id = read_frame_slot(&mut self.frame_pool, slot_idx);
            frames.push(frame_id);
            self.frame_pool.release_free(slot_idx);
        }

        while let Some(record) = self.evt_ring.consumer_peek() {
            events.push(parse_event_record(&record));
            self.evt_ring.consumer_pop_advance();
        }

        Ok(DrainOutcome {
            frames,
            events,
            max_ready_depth,
        })
    }

    fn assert_reconciliation(&self) {
        assert_eq!(
            self.frame_pool.free_len(),
            self.frame_pool.slot_count(),
            "all frame slots should be free"
        );
        assert_eq!(self.frame_pool.ready_len(), 0, "ready ring drained");
        assert!(
            self.evt_ring.consumer_peek().is_none(),
            "event ring should be empty after drain"
        );
        assert_eq!(
            self.audio_pool.free_len(),
            self.audio_pool.slot_count(),
            "audio slots remain unused and free"
        );
        assert_eq!(
            self.audio_pool.ready_len(),
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
        Ok(Self {
            cmd_ring,
            evt_ring,
            frame_pool,
            audio_pool,
        })
    }

    fn layout_object(&self) -> Result<Object, JsValue> {
        let layout = Object::new();
        let frame_layout = self.frame_pool.wasm_layout();
        let audio_layout = self.audio_pool.wasm_layout();
        Reflect::set(
            &layout,
            &JsValue::from_str("cmdRing"),
            &msg_ring_layout_to_js(self.cmd_ring.wasm_layout())?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("evtRing"),
            &msg_ring_layout_to_js(self.evt_ring.wasm_layout())?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("frameSlots"),
            &region_to_js(frame_layout.slots)?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("frameFree"),
            &index_ring_layout_to_js(frame_layout.free)?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("frameReady"),
            &index_ring_layout_to_js(frame_layout.ready)?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("audioSlots"),
            &region_to_js(audio_layout.slots)?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("audioFree"),
            &index_ring_layout_to_js(audio_layout.free)?,
        )?;
        Reflect::set(
            &layout,
            &JsValue::from_str("audioReady"),
            &index_ring_layout_to_js(audio_layout.ready)?,
        )?;
        Ok(layout)
    }

    fn config_object(&self) -> Result<Object, JsValue> {
        let cfg = Object::new();
        let frame_layout = self.frame_pool.wasm_layout();
        let audio_layout = self.audio_pool.wasm_layout();
        Reflect::set(
            &cfg,
            &JsValue::from_str("frameSlotCount"),
            &JsValue::from(frame_layout.slot_count),
        )?;
        Reflect::set(
            &cfg,
            &JsValue::from_str("frameSlotSize"),
            &JsValue::from(frame_layout.slot_size),
        )?;
        Reflect::set(
            &cfg,
            &JsValue::from_str("audioSlotCount"),
            &JsValue::from(audio_layout.slot_count),
        )?;
        Reflect::set(
            &cfg,
            &JsValue::from_str("audioSlotSize"),
            &JsValue::from(audio_layout.slot_size),
        )?;
        Ok(cfg)
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

    async fn wait(mut self) -> Result<JsValue, JsValue> {
        let value = self
            .receiver
            .await
            .map_err(|_| JsValue::from_str("worker dropped channel"))?;
        self.worker.set_onmessage(None);
        if let Some(closure) = self.closure.take() {
            drop(closure);
        }
        Ok(value)
    }
}

fn shared_memory_buffer() -> Result<SharedArrayBuffer, JsValue> {
    let memory = wasm_bindgen::memory().unchecked_into::<js_sys::WebAssembly::Memory>();
    let buffer = memory.buffer();
    buffer
        .dyn_into::<SharedArrayBuffer>()
        .map_err(|_| JsValue::from_str("expected shared linear memory"))
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

fn make_message(
    kind: &str,
    memory: Option<SharedArrayBuffer>,
    layout: Option<Object>,
    config: Option<Object>,
) -> JsValue {
    let msg = Object::new();
    Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str(kind)).unwrap();
    if let Some(memory) = memory {
        Reflect::set(&msg, &JsValue::from_str("memory"), &memory.into()).unwrap();
    }
    if let Some(layout) = layout {
        Reflect::set(&msg, &JsValue::from_str("layout"), &layout).unwrap();
    }
    if let Some(config) = config {
        Reflect::set(&msg, &JsValue::from_str("config"), &config).unwrap();
    }
    msg.into()
}

fn object_with_u32(key: &str, value: u32) -> Result<Object, JsValue> {
    let obj = Object::new();
    Reflect::set(&obj, &JsValue::from_str(key), &JsValue::from(value))?;
    Ok(obj)
}

fn region_to_js(region: Region) -> Result<JsValue, JsValue> {
    let obj = Object::new();
    Reflect::set(
        &obj,
        &JsValue::from_str("offset"),
        &JsValue::from(region.offset),
    )?;
    Reflect::set(
        &obj,
        &JsValue::from_str("length"),
        &JsValue::from(region.length),
    )?;
    Ok(obj.into())
}

fn msg_ring_layout_to_js(layout: MsgRingLayout) -> Result<JsValue, JsValue> {
    let obj = Object::new();
    Reflect::set(
        &obj,
        &JsValue::from_str("header"),
        &region_to_js(layout.header)?,
    )?;
    Reflect::set(
        &obj,
        &JsValue::from_str("data"),
        &region_to_js(layout.data)?,
    )?;
    Reflect::set(
        &obj,
        &JsValue::from_str("capacity"),
        &JsValue::from(layout.capacity_bytes),
    )?;
    Ok(obj.into())
}

fn index_ring_layout_to_js(layout: IndexRingLayout) -> Result<JsValue, JsValue> {
    let obj = Object::new();
    Reflect::set(
        &obj,
        &JsValue::from_str("header"),
        &region_to_js(layout.header)?,
    )?;
    Reflect::set(
        &obj,
        &JsValue::from_str("entries"),
        &region_to_js(layout.entries)?,
    )?;
    Reflect::set(
        &obj,
        &JsValue::from_str("capacity"),
        &JsValue::from(layout.capacity),
    )?;
    Ok(obj.into())
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

fn get_u32(value: &JsValue, key: &str) -> u32 {
    Reflect::get(value, &JsValue::from_str(key))
        .unwrap_or(JsValue::UNDEFINED)
        .as_f64()
        .unwrap_or_default() as u32
}
