//! Mock services hub builder for testing.

use hub::{ServicesHub, ServicesHubBuilder};

/// Creates a mock services hub with default capacities.
pub fn make_hub() -> ServicesHub {
    make_hub_with_capacities(64, 32, 128, 128)
}

/// Creates a mock services hub with custom capacities for each service.
pub fn make_hub_with_capacities(kernel: usize, fs: usize, gpu: usize, audio: usize) -> ServicesHub {
    ServicesHubBuilder::new()
        .kernel(services_kernel::KernelService::new_handle(kernel))
        .fs(services_fs::FsService::new_handle(fs))
        .gpu(services_gpu::GpuService::new_handle(gpu))
        .audio(services_audio::AudioService::new_handle(audio))
        .build()
        .expect("mock hub build")
}
