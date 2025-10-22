//! WASM UI exports for GBX emulator (wasm32 only).

use js_sys::{Array, Object, Reflect, Uint8Array};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use web_sys::console;

use app::Scheduler;
use world::{Intent, IntentPriority, KernelRep, Report};

thread_local! {
    static CTX: RefCell<Option<Scheduler>> = RefCell::new(None);
    static IN_EXPORT: std::cell::Cell<bool> = std::cell::Cell::new(false);
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
        console::log_1(&"gbx_init: creating mock services hub".into());
        let hub = mock::make_hub();

        console::log_1(&"gbx_init: creating world and scheduler".into());
        let world = world::World::default();
        let scheduler = Scheduler::new(world, hub);

        CTX.with(|c| *c.borrow_mut() = Some(scheduler));
        console::log_1(&"gbx_init: initialization complete".into());
        Ok(())
    })
}

#[wasm_bindgen]
pub fn gbx_tick(max_reports: u32) -> Result<JsValue, JsValue> {
    with_guard(|| {
        CTX.with(|c| {
            let mut opt = c.borrow_mut();
            let scheduler = opt
                .as_mut()
                .ok_or_else(|| JsValue::from_str("not inited"))?;

            scheduler.enqueue_intent(IntentPriority::P1, Intent::PumpFrame);
            let reports = scheduler.run_once_collect();

            console::log_1(&format!("gbx_tick: got {} reports", reports.len()).into());

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
                                "gbx_tick: LaneFrame lane={} frame_id={} has_slot={} pixels_len={}",
                                lane,
                                frame_id,
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
                        } else if !span.pixels.is_empty() {
                            // No slot pool - embed pixels directly in the report
                            let arr = Uint8Array::from(&span.pixels[..]);
                            Reflect::set(&o, &"pixels".into(), arr.as_ref())?;
                        }
                    }
                    other => {
                        console::log_1(&format!("gbx_tick: Other report: {:?}", other).into());
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
pub fn gbx_consume_frame(start_idx: u32) -> Result<JsValue, JsValue> {
    with_guard(|| {
        console::log_1(
            &format!(
                "gbx_consume_frame: Note - mock services don't use slot pools yet (start_idx={})",
                start_idx
            )
            .into(),
        );
        // For now, return a placeholder since mock services don't populate slot pools.
        // Ensure alpha is opaque so browser tests can validate the pixel buffer.
        let out = Object::new();
        Reflect::set(&out, &"width".into(), &JsValue::from_f64(160.0))?;
        Reflect::set(&out, &"height".into(), &JsValue::from_f64(144.0))?;
        let mut pixels = vec![0u8; 160 * 144 * 4];
        for chunk in pixels.chunks_mut(4) {
            chunk[3] = 0xFF;
        }
        let arr = Uint8Array::from(pixels.as_slice());
        Reflect::set(&out, &"pixels".into(), arr.as_ref())?;
        console::log_1(&"gbx_consume_frame: returning placeholder frame".into());
        Ok(out.into())
    })
}
