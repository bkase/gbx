//! Reusable transport worker for WASM using wasm-bindgen pattern.
//!
//! This crate provides app-agnostic worker functions for transport operations.
//! It has NO dependencies on app/hub/world - it's purely about transport layer.
//!
//! For GBX-specific test orchestration, see the `gbx-wasm` crate which re-exports
//! these functions and adds test entry points.
#![allow(missing_docs)]

pub use types::*;

pub mod types {
    pub use transport::wasm::{MsgRingLayout, SlotPoolLayout};
    pub use transport_fabric::layout::{
        ArchivedFabricLayout, EndpointLayout, FabricLayout, PortLayout, PortRole,
    };
    pub use transport_scenarios::{ScenarioStats, ScenarioType, TestConfig};

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WorkerInitDescriptor {
        pub cmd_ring: MsgRingLayout,
        pub evt_ring: MsgRingLayout,
        pub frame_pool: SlotPoolLayout,
        pub audio_pool: SlotPoolLayout,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::types::*;
    use std::cell::RefCell;
    use transport::wasm::IntoNativeLayout;
    use transport::{Envelope, MsgRing, SlotPool, SlotPush};
    use transport_fabric::{ArchivedFabricLayout, WorkerRuntime};
    use transport_scenarios::{
        event_payload, FabricHandle, FrameScenarioEngine, PtrStatsSink, StatsSink,
    };
    use transport_scenarios::{EVENT_TAG, EVENT_VER};
    use wasm_bindgen::prelude::*;

    const OK: i32 = 0;
    const ERR_NULL_PTR: i32 = -1;
    const ERR_ALREADY_INIT: i32 = -2;
    const ERR_NOT_INIT: i32 = -3;
    const ERR_INVALID_TEST_TYPE: i32 = -5;

    const EVENT_ENVELOPE: Envelope = Envelope {
        tag: EVENT_TAG,
        ver: EVENT_VER,
        flags: 0,
    };

    #[derive(Clone, Copy)]
    struct EndpointLayouts {
        evt_ring: transport::wasm::MsgRingLayout,
        frame_pool: transport::wasm::SlotPoolLayout,
        _audio_pool: transport::wasm::SlotPoolLayout,
    }

    struct WasmFabricHandle {
        evt_ring: MsgRing,
        frame_pool: SlotPool,
    }

    impl WasmFabricHandle {
        fn from_layout(layout: &EndpointLayouts) -> Self {
            let evt_ring = unsafe { MsgRing::from_wasm_layout(layout.evt_ring, EVENT_ENVELOPE) };
            let frame_pool = unsafe { SlotPool::from_wasm_layout(layout.frame_pool) };
            Self {
                evt_ring,
                frame_pool,
            }
        }
    }

    impl FabricHandle for WasmFabricHandle {
        fn acquire_free_slot(&mut self) -> Option<u32> {
            self.frame_pool.try_acquire_free()
        }

        fn wait_for_free_slot(&self) {
            self.frame_pool.wait_for_free_slot();
        }

        fn write_frame(&mut self, slot_idx: u32, frame_id: u32) {
            if self.frame_pool.slot_size() < 4 {
                return;
            }
            let slot = self.frame_pool.slot_mut(slot_idx);
            slot[..4].copy_from_slice(&frame_id.to_le_bytes());
        }

        fn push_ready(&mut self, slot_idx: u32) -> SlotPush {
            self.frame_pool.push_ready(slot_idx)
        }

        fn wait_for_ready_drain(&self) {
            self.frame_pool.wait_for_ready_drain();
        }

        fn try_push_event(&mut self, frame_id: u32, slot_idx: u32) -> bool {
            let payload = event_payload(frame_id, slot_idx);
            if let Some(mut grant) = self.evt_ring.try_reserve(payload.len()) {
                grant.payload()[..payload.len()].copy_from_slice(&payload);
                grant.commit(payload.len());
                true
            } else {
                false
            }
        }

        fn wait_for_event_space(&self) {
            self.evt_ring.wait_for_space();
        }
    }

    thread_local! {
        static FABRIC_ENDPOINTS: RefCell<Vec<EndpointLayouts>> = RefCell::new(Vec::new());
        static FABRIC_RUNTIME: RefCell<Option<WorkerRuntime>> = RefCell::new(None);
    }

    #[wasm_bindgen]
    pub fn worker_init(descriptor_ptr: u32) -> i32 {
        unsafe {
            let descriptor = match ref_from_u32::<WorkerInitDescriptor>(descriptor_ptr) {
                Some(value) => value,
                None => return ERR_NULL_PTR,
            };

            let mut layout = FabricLayout::default();
            let mut endpoint_layout = EndpointLayout::default();

            endpoint_layout.push_port(PortRole::Replies, PortLayout::MsgRing(descriptor.evt_ring));

            endpoint_layout.push_port(
                PortRole::SlotPool(0),
                PortLayout::SlotPool(descriptor.frame_pool),
            );

            endpoint_layout.push_port(
                PortRole::SlotPool(1),
                PortLayout::SlotPool(descriptor.audio_pool),
            );

            layout.add_endpoint(endpoint_layout);

            use rkyv::rancor::Error;
            let bytes = rkyv::to_bytes::<Error>(&layout).unwrap();
            fabric_worker_init(bytes.as_ptr() as u32, bytes.len() as u32)
        }
    }

    #[wasm_bindgen]
    pub fn worker_register_test(config_ptr: u32, stats_ptr: u32) -> i32 {
        unsafe {
            let config = match ref_from_u32::<TestConfig>(config_ptr) {
                Some(c) => c,
                None => return ERR_NULL_PTR,
            };
            let stats_sink = match PtrStatsSink::new(stats_ptr as *mut ScenarioStats) {
                Some(s) => s,
                None => return ERR_NULL_PTR,
            };

            let scenario = match config.scenario_kind() {
                Some(kind) => kind,
                None => return ERR_INVALID_TEST_TYPE,
            };

            stats_sink.with_stats(|stats| stats.reset());

            FABRIC_RUNTIME.with(|runtime_cell| {
                let mut runtime_guard = runtime_cell.borrow_mut();
                let runtime = match runtime_guard.as_mut() {
                    Some(rt) => rt,
                    None => return ERR_NOT_INIT,
                };

                let handle = FABRIC_ENDPOINTS.with(|endpoints_cell| {
                    let endpoints = endpoints_cell.borrow();
                    let layout = endpoints.first().expect("fabric endpoint missing");
                    WasmFabricHandle::from_layout(layout)
                });

                runtime.register(FrameScenarioEngine::new(handle, stats_sink, scenario));
                OK
            })
        }
    }

    #[wasm_bindgen]
    pub fn fabric_worker_init(layout_ptr: u32, layout_len: u32) -> i32 {
        use rkyv::access_unchecked;

        if layout_ptr == 0 || layout_len == 0 {
            return FABRIC_RUNTIME.with(|runtime| {
                if runtime.borrow().is_some() {
                    return ERR_ALREADY_INIT;
                }
                *runtime.borrow_mut() = Some(WorkerRuntime::new());
                OK
            });
        }

        unsafe {
            let layout_bytes =
                std::slice::from_raw_parts(layout_ptr as *const u8, layout_len as usize);
            let archived_layout = access_unchecked::<ArchivedFabricLayout>(layout_bytes);

            let mut endpoints = Vec::new();
            for endpoint_layout in archived_layout.endpoints.iter() {
                use rkyv::Archived;

                let mut evt_ring = None;
                let mut frame_pool = None;
                let mut audio_pool = None;

                for port_tuple in endpoint_layout.ports.iter() {
                    let role = &port_tuple.0;
                    let port_layout = &port_tuple.1;

                    match (role, port_layout) {
                        (
                            Archived::<PortRole>::Replies,
                            Archived::<PortLayout>::MsgRing(ring_layout),
                        ) => {
                            evt_ring = Some(ring_layout.into_native());
                        }
                        (
                            Archived::<PortRole>::SlotPool(idx),
                            Archived::<PortLayout>::SlotPool(pool_layout),
                        ) if idx.to_native() == 0 => {
                            frame_pool = Some(pool_layout.into_native());
                        }
                        (
                            Archived::<PortRole>::SlotPool(idx),
                            Archived::<PortLayout>::SlotPool(pool_layout),
                        ) if idx.to_native() == 1 => {
                            audio_pool = Some(pool_layout.into_native());
                        }
                        _ => {}
                    }
                }

                if let (Some(evt_ring), Some(frame_pool), Some(audio_pool)) =
                    (evt_ring, frame_pool, audio_pool)
                {
                    endpoints.push(EndpointLayouts {
                        evt_ring,
                        frame_pool,
                        _audio_pool: audio_pool,
                    });
                }
            }

            if FABRIC_ENDPOINTS.with(|cell| !cell.borrow().is_empty()) {
                return ERR_ALREADY_INIT;
            }
            FABRIC_ENDPOINTS.with(|cell| {
                *cell.borrow_mut() = endpoints;
            });

            if FABRIC_RUNTIME.with(|runtime| runtime.borrow().is_some()) {
                return ERR_ALREADY_INIT;
            }
            FABRIC_RUNTIME.with(|runtime| {
                *runtime.borrow_mut() = Some(WorkerRuntime::new());
            });
            OK
        }
    }

    #[wasm_bindgen]
    pub fn fabric_worker_run() -> i32 {
        FABRIC_RUNTIME.with(|runtime| {
            let mut guard = runtime.borrow_mut();
            match guard.as_mut() {
                Some(rt) => rt.run_tick() as i32,
                None => ERR_NOT_INIT,
            }
        })
    }

    unsafe fn ref_from_u32<T>(ptr: u32) -> Option<&'static T> {
        (ptr as *const T).as_ref()
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{fabric_worker_init, fabric_worker_run, worker_init, worker_register_test};

#[cfg(not(target_arch = "wasm32"))]
mod stubs {
    #[no_mangle]
    pub extern "C" fn worker_init(_descriptor_ptr: u32) -> i32 {
        let _ = _descriptor_ptr;
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_register_test(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }

    #[no_mangle]
    pub extern "C" fn fabric_worker_init(_layout_ptr: u32, _layout_len: u32) -> i32 {
        let _ = (_layout_ptr, _layout_len);
        -1
    }

    #[no_mangle]
    pub extern "C" fn fabric_worker_run() -> i32 {
        -1
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use stubs::*;
