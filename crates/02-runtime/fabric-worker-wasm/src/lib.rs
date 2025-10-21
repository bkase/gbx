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
    pub use transport::wasm::{MailboxLayout, MsgRingLayout, SlotPoolLayout};
    pub use transport_fabric::layout::{
        ArchivedFabricLayout, EndpointLayout, FabricLayout, PortLayout, PortRole,
    };
    pub use transport_scenarios::{ScenarioStats, ScenarioType, TestConfig};
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::types::{self, *};
    use hub::{Service as HubService, SubmitOutcome, SubmitPolicy};
    use services_audio::AudioService;
    use services_fs::FsService;
    use services_gpu::GpuService;
    use services_kernel::KernelService;
    use std::cell::RefCell;
    use std::sync::Arc;
    use transport::schema::{
        SCHEMA_VERSION_V1, TAG_AUDIO_CMD, TAG_AUDIO_REP, TAG_FS_CMD, TAG_FS_REP, TAG_GPU_CMD,
        TAG_GPU_REP, TAG_KERNEL_CMD, TAG_KERNEL_REP,
    };
    use transport::wasm::IntoNativeLayout;
    use transport::{Envelope, Mailbox, MsgRing, SlotPool, SlotPoolHandle, SlotPush};
    use transport_codecs::{AudioCodec, FsCodec, GpuCodec, KernelCodec};
    use transport_fabric::{
        make_port_pair_mailbox, make_port_pair_ring, ArchivedFabricLayout, Codec, ServiceEngine,
        WorkerEndpoint, WorkerRuntime,
    };
    use transport_scenarios::{
        event_payload, FabricHandle, FrameScenarioEngine, PtrStatsSink, StatsSink,
    };
    use transport_scenarios::{EVENT_TAG, EVENT_VER};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsValue;
    use web_sys::console;

    const OK: i32 = 0;
    const ERR_NULL_PTR: i32 = -1;
    const ERR_ALREADY_INIT: i32 = -2;
    const ERR_NOT_INIT: i32 = -3;
    const ERR_BAD_LAYOUT: i32 = -4;
    const ERR_INVALID_TEST_TYPE: i32 = -5;

    const EVENT_ENVELOPE: Envelope = Envelope {
        tag: EVENT_TAG,
        ver: EVENT_VER,
        flags: 0,
    };

    #[derive(Clone)]
    struct EndpointLayouts {
        lossless: Option<types::MsgRingLayout>,
        besteffort: Option<types::MsgRingLayout>,
        mailbox: Option<types::MailboxLayout>,
        replies: types::MsgRingLayout,
        slot_pools: Vec<types::SlotPoolLayout>,
    }

    struct WasmFabricHandle {
        evt_ring: MsgRing,
        frame_pool: SlotPool,
    }

    impl WasmFabricHandle {
        fn from_layout(layout: &EndpointLayouts) -> Self {
            let evt_ring = unsafe { MsgRing::from_wasm_layout(layout.replies, EVENT_ENVELOPE) };
            let frame_pool_layout = layout
                .slot_pools
                .get(0)
                .copied()
                .expect("frame slot pool missing from layout");
            let frame_pool = unsafe { SlotPool::from_wasm_layout(frame_pool_layout) };
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

        fn with_frame_slot_mut<R>(&mut self, slot_idx: u32, f: impl FnOnce(&mut [u8]) -> R) -> R {
            let slot = self.frame_pool.slot_mut(slot_idx);
            f(slot)
        }
    }

    thread_local! {
        static FABRIC_ENDPOINTS: RefCell<Vec<EndpointLayouts>> = RefCell::new(Vec::new());
        static FABRIC_RUNTIME: RefCell<Option<WorkerRuntime>> = RefCell::new(None);
        static SERVICES_REGISTERED: RefCell<bool> = RefCell::new(false);
    }

    struct FabricServiceEngine<S, C>
    where
        C: Codec + Send + 'static,
        S: HubService<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
    {
        endpoint: WorkerEndpoint<C>,
        service: S,
        drain_budget: usize,
        name: &'static str,
    }

    impl<S, C> FabricServiceEngine<S, C>
    where
        C: Codec + Send + 'static,
        S: HubService<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
    {
        fn new(endpoint: WorkerEndpoint<C>, service: S, name: &'static str) -> Self {
            Self {
                endpoint,
                service,
                drain_budget: 32,
                name,
            }
        }
    }

    impl<S, C> ServiceEngine for FabricServiceEngine<S, C>
    where
        C: Codec + Send + 'static,
        S: HubService<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
    {
        fn poll(&mut self) -> usize {
            let mut work = 0usize;
            let submit_budget = self.drain_budget;
            if let Err(err) = self.endpoint.drain_commands(submit_budget, |cmd| {
                let outcome = self.service.try_submit(cmd);
                if matches!(outcome, SubmitOutcome::Accepted | SubmitOutcome::Coalesced) {
                    work += 1;
                }
            }) {
                console::error_1(&JsValue::from_str(&format!(
                    "{}: failed to drain commands: {err}",
                    self.name
                )));
            }

            let reports = self.service.drain(self.drain_budget);
            for rep in reports.into_iter() {
                match self.endpoint.publish_report(&rep) {
                    Ok(outcome)
                        if matches!(
                            outcome,
                            SubmitOutcome::Accepted | SubmitOutcome::Coalesced
                        ) =>
                    {
                        work += 1;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        console::error_1(&JsValue::from_str(&format!(
                            "{}: failed to publish report: {err}",
                            self.name
                        )));
                    }
                }
            }
            work
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    fn build_worker_endpoint<C>(
        layout: &EndpointLayouts,
        codec: C,
        cmd_tag: u8,
        rep_tag: u8,
    ) -> Result<WorkerEndpoint<C>, i32>
    where
        C: Codec,
    {
        unsafe {
            let lossless = layout.lossless.map(|ring_layout| {
                let ring = MsgRing::from_wasm_layout(
                    ring_layout,
                    Envelope::new(cmd_tag, SCHEMA_VERSION_V1),
                );
                make_port_pair_ring(SubmitPolicy::Lossless, ring).consumer
            });

            let besteffort = layout.besteffort.map(|ring_layout| {
                let ring = MsgRing::from_wasm_layout(
                    ring_layout,
                    Envelope::new(cmd_tag, SCHEMA_VERSION_V1),
                );
                make_port_pair_ring(SubmitPolicy::BestEffort, ring).consumer
            });

            let coalesce = layout.mailbox.map(|mailbox_layout| {
                let mailbox = Mailbox::from_wasm_layout(
                    mailbox_layout,
                    Envelope::new(cmd_tag, SCHEMA_VERSION_V1),
                );
                make_port_pair_mailbox(mailbox).consumer
            });

            let replies_ring = MsgRing::from_wasm_layout(
                layout.replies,
                Envelope::new(rep_tag, SCHEMA_VERSION_V1),
            );
            let replies = make_port_pair_ring(SubmitPolicy::Lossless, replies_ring).producer;

            let slot_pools = layout
                .slot_pools
                .iter()
                .map(|pool_layout| {
                    let pool = SlotPool::from_wasm_layout(*pool_layout);
                    Arc::new(SlotPoolHandle::new(pool))
                })
                .collect();

            Ok(WorkerEndpoint::new(
                lossless, besteffort, coalesce, replies, slot_pools, codec,
            ))
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
    pub fn worker_register_services(layout_ptr: u32, layout_len: u32) -> i32 {
        let _ = (layout_ptr, layout_len);

        if SERVICES_REGISTERED.with(|flag| *flag.borrow()) {
            return ERR_ALREADY_INIT;
        }

        let endpoints = FABRIC_ENDPOINTS.with(|cell| cell.borrow().clone());
        if endpoints.len() < 4 {
            return ERR_BAD_LAYOUT;
        }

        let kernel_endpoint =
            match build_worker_endpoint(&endpoints[0], KernelCodec, TAG_KERNEL_CMD, TAG_KERNEL_REP)
            {
                Ok(endpoint) => endpoint,
                Err(code) => return code,
            };
        let fs_endpoint =
            match build_worker_endpoint(&endpoints[1], FsCodec, TAG_FS_CMD, TAG_FS_REP) {
                Ok(endpoint) => endpoint,
                Err(code) => return code,
            };
        let gpu_endpoint =
            match build_worker_endpoint(&endpoints[2], GpuCodec, TAG_GPU_CMD, TAG_GPU_REP) {
                Ok(endpoint) => endpoint,
                Err(code) => return code,
            };
        let audio_endpoint =
            match build_worker_endpoint(&endpoints[3], AudioCodec, TAG_AUDIO_CMD, TAG_AUDIO_REP) {
                Ok(endpoint) => endpoint,
                Err(code) => return code,
            };

        let status = FABRIC_RUNTIME.with(move |runtime_cell| {
            let mut guard = runtime_cell.borrow_mut();
            let runtime = match guard.as_mut() {
                Some(rt) => rt,
                None => return ERR_NOT_INIT,
            };

            runtime.register(FabricServiceEngine::new(
                kernel_endpoint,
                KernelService::default(),
                "kernel",
            ));
            runtime.register(FabricServiceEngine::new(
                fs_endpoint,
                FsService::default(),
                "fs",
            ));
            runtime.register(FabricServiceEngine::new(
                gpu_endpoint,
                GpuService::default(),
                "gpu",
            ));
            runtime.register(FabricServiceEngine::new(
                audio_endpoint,
                AudioService::default(),
                "audio",
            ));
            OK
        });

        if status == OK {
            SERVICES_REGISTERED.with(|flag| *flag.borrow_mut() = true);
        }

        status
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
                SERVICES_REGISTERED.with(|flag| *flag.borrow_mut() = false);
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

                let mut lossless = None;
                let mut besteffort = None;
                let mut mailbox = None;
                let mut replies = None;
                let mut slot_pools: Vec<Option<types::SlotPoolLayout>> = Vec::new();

                for port_tuple in endpoint_layout.ports.iter() {
                    let role = &port_tuple.0;
                    let port_layout = &port_tuple.1;

                    match (role, port_layout) {
                        (
                            Archived::<PortRole>::CmdLossless,
                            Archived::<PortLayout>::MsgRing(ring_layout),
                        ) => lossless = Some(ring_layout.into_native()),
                        (
                            Archived::<PortRole>::CmdBestEffort,
                            Archived::<PortLayout>::MsgRing(ring_layout),
                        ) => besteffort = Some(ring_layout.into_native()),
                        (
                            Archived::<PortRole>::CmdMailbox,
                            Archived::<PortLayout>::Mailbox(mailbox_layout),
                        ) => {
                            let layout_native = types::MailboxLayout {
                                header: (&mailbox_layout.header).into_native(),
                                data: (&mailbox_layout.data).into_native(),
                            };
                            mailbox = Some(layout_native);
                        }
                        (
                            Archived::<PortRole>::Replies,
                            Archived::<PortLayout>::MsgRing(ring_layout),
                        ) => replies = Some(ring_layout.into_native()),
                        (
                            Archived::<PortRole>::SlotPool(idx),
                            Archived::<PortLayout>::SlotPool(pool_layout),
                        ) => {
                            let native_idx = idx.to_native() as usize;
                            if slot_pools.len() <= native_idx {
                                slot_pools.resize(native_idx + 1, None);
                            }
                            slot_pools[native_idx] = Some(pool_layout.into_native());
                        }
                        _ => {}
                    }
                }

                if let Some(replies_layout) = replies {
                    let pool_layouts = slot_pools
                        .into_iter()
                        .filter_map(|entry| entry)
                        .collect::<Vec<_>>();
                    endpoints.push(EndpointLayouts {
                        lossless,
                        besteffort,
                        mailbox,
                        replies: replies_layout,
                        slot_pools: pool_layouts,
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
            SERVICES_REGISTERED.with(|flag| *flag.borrow_mut() = false);
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
pub use wasm::{
    fabric_worker_init, fabric_worker_run, worker_register_services, worker_register_test,
};

#[cfg(not(target_arch = "wasm32"))]
mod stubs {
    #[no_mangle]
    pub extern "C" fn worker_register_test(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_register_services(_layout_ptr: u32, _layout_len: u32) -> i32 {
        let _ = (_layout_ptr, _layout_len);
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
