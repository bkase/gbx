//! Mock services hub builder with simple no-op backends.

use hub::{
    AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle, ServicesHub,
    ServicesHubBuilder,
};
use services_audio::AudioService;
use services_fs::FsService;
use services_gpu::GpuService;
use services_kernel::KernelService;

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

fn kernel_service(capacity: usize) -> KernelServiceHandle {
    KernelService::new_handle(capacity)
}

fn fs_service(capacity: usize) -> FsServiceHandle {
    FsService::new_handle(capacity)
}

fn gpu_service(capacity: usize) -> GpuServiceHandle {
    GpuService::new_handle(capacity)
}

fn audio_service(capacity: usize) -> AudioServiceHandle {
    AudioService::new_handle(capacity)
}
