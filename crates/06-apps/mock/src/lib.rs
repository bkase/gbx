//! Mock services hub builder with simple no-op backends.

use hub::{
    AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle, ServicesHub,
    ServicesHubBuilder,
};
use services_audio::AudioService;
use services_fs::FsService;
use services_gpu::GpuService;
use services_kernel::KernelService;
use smallvec::SmallVec;
use std::sync::Arc;

/// Creates a mock services hub with default capacities.
pub fn make_hub() -> ServicesHub {
    make_hub_with_capacities(64, 32, 128, 128)
}

/// Creates a mock services hub with custom capacities for each service.
///
/// Capacity parameters are accepted for API compatibility but ignored because the
/// no-op services do not enqueue work.
pub fn make_hub_with_capacities(kernel: usize, fs: usize, gpu: usize, audio: usize) -> ServicesHub {
    ServicesHubBuilder::new()
        .kernel(kernel_service(kernel))
        .fs(fs_service(fs))
        .gpu(gpu_service(gpu))
        .audio(audio_service(audio))
        .build()
        .expect("mock hub build")
}

/// Adapter that wraps service-abi Arc services to implement hub::Service trait.
struct HubServiceWrapper<Cmd, Rep> {
    inner: Arc<dyn service_abi::Service<Cmd = Cmd, Rep = Rep> + Send + Sync>,
}

impl<Cmd, Rep> hub::Service for HubServiceWrapper<Cmd, Rep>
where
    Cmd: Send + 'static,
    Rep: Send + 'static,
{
    type Cmd = Cmd;
    type Rep = Rep;

    fn try_submit(&self, cmd: &Self::Cmd) -> hub::SubmitOutcome {
        self.inner.try_submit(cmd)
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        self.inner.drain(max)
    }
}

fn kernel_service(capacity: usize) -> KernelServiceHandle {
    Arc::new(HubServiceWrapper {
        inner: KernelService::new_handle(capacity),
    })
}

fn fs_service(capacity: usize) -> FsServiceHandle {
    Arc::new(HubServiceWrapper {
        inner: FsService::new_handle(capacity),
    })
}

fn gpu_service(capacity: usize) -> GpuServiceHandle {
    Arc::new(HubServiceWrapper {
        inner: GpuService::new_handle(capacity),
    })
}

fn audio_service(capacity: usize) -> AudioServiceHandle {
    Arc::new(HubServiceWrapper {
        inner: AudioService::new_handle(capacity),
    })
}
