//! Service registration for GBX WASM worker.

use fabric_worker_wasm::{
    build_worker_endpoint, EndpointLayouts, FABRIC_ENDPOINTS, FABRIC_RUNTIME,
};
use service_abi::{Service, SubmitOutcome};
use services_audio::AudioService;
use services_fs::FsService;
use services_gpu::GpuService;
use services_kernel::KernelService;
use smallvec::SmallVec;
use std::cell::RefCell;
use transport::schema::{
    TAG_AUDIO_CMD, TAG_AUDIO_REP, TAG_FS_CMD, TAG_FS_REP, TAG_GPU_CMD, TAG_GPU_REP, TAG_KERNEL_CMD,
    TAG_KERNEL_REP,
};
use transport_codecs::{AudioCodec, FsCodec, GpuCodec, KernelCodec};
use transport_fabric::{Codec, ServiceEngine, WorkerEndpoint};
use wasm_bindgen::prelude::*;
use web_sys::console;

const OK: i32 = 0;
const ERR_NOT_INIT: i32 = -3;
const ERR_BAD_LAYOUT: i32 = -4;
const ERR_ALREADY_INIT: i32 = -2;

thread_local! {
    static SERVICES_REGISTERED: RefCell<bool> = RefCell::new(false);
}

struct FabricServiceEngine<S, C>
where
    C: Codec + Send + 'static,
    S: Service<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
{
    endpoint: WorkerEndpoint<C>,
    service: S,
    drain_budget: usize,
    name: &'static str,
}

impl<S, C> FabricServiceEngine<S, C>
where
    C: Codec + Send + 'static,
    S: Service<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
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
    S: Service<Cmd = C::Cmd, Rep = C::Rep> + Send + 'static,
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
                    if matches!(outcome, SubmitOutcome::Accepted | SubmitOutcome::Coalesced) =>
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

#[wasm_bindgen]
pub fn worker_register_services(_layout_ptr: u32, _layout_len: u32) -> i32 {
    if SERVICES_REGISTERED.with(|flag| *flag.borrow()) {
        return ERR_ALREADY_INIT;
    }

    let endpoints = FABRIC_ENDPOINTS.with(|cell| cell.borrow().clone());
    if endpoints.len() < 4 {
        return ERR_BAD_LAYOUT;
    }

    let kernel_endpoint =
        match build_worker_endpoint(&endpoints[0], KernelCodec, TAG_KERNEL_CMD, TAG_KERNEL_REP) {
            Ok(endpoint) => endpoint,
            Err(code) => return code,
        };
    let fs_endpoint = match build_worker_endpoint(&endpoints[1], FsCodec, TAG_FS_CMD, TAG_FS_REP) {
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
