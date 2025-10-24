//! WASM UI exports for GBX emulator (wasm32 only).

use js_sys::{Array, Object, Reflect, Uint8Array};
use std::cell::RefCell;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::console;

use app::Scheduler;
use services_fabric::TransportServices;
use transport::{SlotPoolHandle, SlotPop};
use world::{Intent, IntentPriority, KernelRep, Report};

use crate::{fabric_worker_init, fabric_worker_run, worker_register_services};

thread_local! {
    static CTX: RefCell<Option<UiCtx>> = RefCell::new(None);
    static IN_EXPORT: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

struct UiCtx {
    scheduler: Scheduler,
    frame_pool: Arc<SlotPoolHandle>,
    #[allow(dead_code)]
    layout_bytes: Vec<u8>,
}

impl UiCtx {
    fn drive_worker_until_idle(&self) {
        // Run the fabric worker until it reports no additional work.
        for _ in 0..16 {
            let work = fabric_worker_run();
            if work < 0 {
                console::error_1(&format!("fabric_worker_run error: {work}").into());
                break;
            }
            if work == 0 {
                break;
            }
        }
    }
}

fn with_guard<F, R>(f: F) -> Result<R, JsValue>
where
    F: FnOnce() -> Result<R, JsValue>,
{
    IN_EXPORT.with(|g| {
        if g.get() {
            return Err(JsValue::from_str("reentrant export call"));
        }
        g.set(true);
        let r = f();
        g.set(false);
        r
    })
}

#[wasm_bindgen]
pub fn gbx_init() -> Result<(), JsValue> {
    console::log_1(&"gbx_init: starting initialization".into());
    with_guard(|| {
        console::log_1(&"gbx_init: creating transport services".into());
        let services = TransportServices::new()
            .map_err(|e| JsValue::from_str(&format!("transport init failed: {e}")))?;

        let TransportServices {
            kernel,
            fs,
            gpu,
            audio,
            worker,
            scheduler,
        } = services;

        console::log_1(&"gbx_init: serializing fabric layout".into());
        let layout_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&worker.layout)
            .map_err(|e| JsValue::from_str(&format!("layout serialize failed: {e}")))?
            .into_vec();
        let layout_ptr = layout_bytes.as_ptr() as u32;
        let layout_len = layout_bytes.len() as u32;

        console::log_1(&"gbx_init: initializing fabric worker".into());
        let status = fabric_worker_init(layout_ptr, layout_len);
        if status != 0 {
            return Err(JsValue::from_str(&format!(
                "fabric_worker_init status={status}"
            )));
        }

        console::log_1(&"gbx_init: registering services".into());
        let status = worker_register_services(layout_ptr, layout_len);
        if status != 0 {
            return Err(JsValue::from_str(&format!(
                "worker_register_services status={status}"
            )));
        }

        let frame_pool = scheduler
            .kernel
            .slot_pools()
            .get(0)
            .cloned()
            .ok_or_else(|| JsValue::from_str("missing frame slot pool"))?;

        console::log_1(&"gbx_init: building services hub".into());
        let hub = hub::ServicesHubBuilder::new()
            .kernel(kernel)
            .fs(fs)
            .gpu(gpu)
            .audio(audio)
            .build()
            .map_err(|e| JsValue::from_str(&format!("build hub failed: {e}")))?;

        console::log_1(&"gbx_init: creating world and scheduler".into());
        let world = world::World::default();
        let scheduler = Scheduler::new(world, hub);

        CTX.with(|c| {
            *c.borrow_mut() = Some(UiCtx {
                scheduler,
                frame_pool,
                layout_bytes,
            })
        });
        console::log_1(&"gbx_init: initialization complete".into());
        Ok(())
    })
}

#[wasm_bindgen]
pub fn gbx_debug_state() -> Result<JsValue, JsValue> {
    with_guard(|| {
        CTX.with(|c| {
            let guard = c.borrow();
            let ctx = guard
                .as_ref()
                .ok_or_else(|| JsValue::from_str("not inited"))?;

            let state = Object::new();
            Reflect::set(
                &state,
                &"rom_loaded".into(),
                &JsValue::from_bool(ctx.scheduler.world().rom_loaded()),
            )?;
            Reflect::set(
                &state,
                &"frame_id".into(),
                &JsValue::from_f64(ctx.scheduler.world().frame_id() as f64),
            )?;

            let pending = ctx.scheduler.pending_intents();
            let pending_arr = Array::new();
            for count in pending {
                pending_arr.push(&JsValue::from_f64(count as f64));
            }
            Reflect::set(&state, &"pending_intents".into(), pending_arr.as_ref())?;

            Ok(state.into())
        })
    })
}

#[wasm_bindgen]
pub fn gbx_load_rom(bytes: Uint8Array) -> Result<(), JsValue> {
    with_guard(|| {
        CTX.with(|c| {
            let mut guard = c.borrow_mut();
            let ctx = guard
                .as_mut()
                .ok_or_else(|| JsValue::from_str("not inited"))?;

            let mut data = vec![0u8; bytes.length() as usize];
            bytes.copy_to(&mut data[..]);
            let rom = Arc::<[u8]>::from(data.into_boxed_slice());

            ctx.scheduler.enqueue_front_p0(Intent::LoadRom {
                group: 0,
                bytes: rom,
            });
            ctx.scheduler
                .enqueue_intent(IntentPriority::P1, Intent::PumpFrame);
            Ok(())
        })
    })
}

#[wasm_bindgen]
pub fn gbx_tick(max_reports: u32) -> Result<JsValue, JsValue> {
    with_guard(|| {
        CTX.with(|c| {
            let mut opt = c.borrow_mut();
            let ctx = opt
                .as_mut()
                .ok_or_else(|| JsValue::from_str("not inited"))?;

            ctx.scheduler
                .enqueue_intent(IntentPriority::P1, Intent::PumpFrame);

            let mut reports = Vec::new();

            for attempt in 0..4 {
                let batch = ctx.scheduler.run_once_collect();
                if !batch.is_empty() {
                    reports = batch;
                    break;
                }

                ctx.drive_worker_until_idle();

                if attempt == 3 {
                    reports = ctx.scheduler.run_once_collect();
                }
            }

            console::log_1(&format!("gbx_tick: got {} reports", reports.len()).into());

            if !ctx.scheduler.world().auto_pump {
                // Ensure continued progress if autopump disabled in world settings.
                ctx.scheduler
                    .enqueue_intent(IntentPriority::P1, Intent::PumpFrame);
            }

            ctx.drive_worker_until_idle();

            let arr = Array::new();
            for rep in reports.into_iter().take(max_reports as usize) {
                let o = Object::new();
                match &rep {
                    Report::Kernel(KernelRep::LaneFrame {
                        lane,
                        span,
                        frame_id,
                        ..
                    }) => {
                        console::log_1(
                            &format!(
                                "gbx_tick: LaneFrame lane={} frame_id={} w={} h={} has_slot={} pixels_len={}",
                                lane,
                                frame_id,
                                span.width,
                                span.height,
                                span.slot_span.is_some(),
                                span.pixels.len()
                            )
                            .into(),
                        );
                        Reflect::set(&o, &"type".into(), &"Kernel.LaneFrame".into())?;
                        Reflect::set(&o, &"lane".into(), &JsValue::from_f64(*lane as f64))?;
                        Reflect::set(&o, &"frame_id".into(), &JsValue::from_f64(*frame_id as f64))?;
                        Reflect::set(&o, &"width".into(), &JsValue::from_f64(span.width as f64))?;
                        Reflect::set(&o, &"height".into(), &JsValue::from_f64(span.height as f64))?;

                        if let Some(s) = &span.slot_span {
                            Reflect::set(
                                &o,
                                &"start_idx".into(),
                                &JsValue::from_f64(s.start_idx as f64),
                            )?;
                            Reflect::set(
                                &o,
                                &"slot_count".into(),
                                &JsValue::from_f64(s.count as f64),
                            )?;
                        } else if !span.pixels.is_empty() {
                            // No slot pool - embed pixels directly in the report
                            let arr = Uint8Array::from(&span.pixels[..]);
                            Reflect::set(&o, &"pixels".into(), arr.as_ref())?;
                        }
                    }
                    other => {
                        Reflect::set(
                            &o,
                            &"type".into(),
                            &JsValue::from_str(&format!("{other:?}")),
                        )?;
                    }
                }
                arr.push(&o);
            }
            Ok(arr.into())
        })
    })
}

#[wasm_bindgen]
pub fn gbx_consume_frame(
    _start_idx: u32,
    slot_count: u32,
    width: u16,
    height: u16,
) -> Result<JsValue, JsValue> {
    with_guard(|| {
        CTX.with(|c| {
            let mut opt = c.borrow_mut();
            let ctx = opt
                .as_mut()
                .ok_or_else(|| JsValue::from_str("not inited"))?;

            let expected_len = usize::from(width)
                .saturating_mul(usize::from(height))
                .saturating_mul(4);
            let mut pixels = vec![0u8; expected_len];
            let span_slots = slot_count.max(1);

            ctx.frame_pool.with_mut(|pool| {
                let mut written = 0usize;
                let mut consumed = Vec::with_capacity(span_slots as usize);

                for _ in 0..span_slots {
                    match pool.pop_ready() {
                        SlotPop::Ok { slot_idx } => {
                            let slot = pool.slot_mut(slot_idx);
                            let take = expected_len.saturating_sub(written).min(slot.len());
                            let end = written + take;
                            pixels[written..end].copy_from_slice(&slot[..take]);
                            written = end;
                            consumed.push(slot_idx);
                        }
                        SlotPop::Empty => break,
                    }
                }

                #[cfg(debug_assertions)]
                console::log_1(
                    &format!(
                        "consume_frame: span_slots={} ready_len={} free_len={}",
                        span_slots,
                        pool.ready_len(),
                        pool.free_len()
                    )
                    .into(),
                );

                for idx in consumed {
                    pool.release_free(idx);
                }
            });

            let out = Object::new();
            Reflect::set(&out, &"width".into(), &JsValue::from_f64(f64::from(width)))?;
            Reflect::set(
                &out,
                &"height".into(),
                &JsValue::from_f64(f64::from(height)),
            )?;
            let arr = Uint8Array::from(pixels.as_slice());
            Reflect::set(&out, &"pixels".into(), arr.as_ref())?;
            Ok(out.into())
        })
    })
}
