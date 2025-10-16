//! Mock services hub builder with simple no-op backends.

use hub::{
    AudioCmd, AudioRep, AudioServiceHandle, FsCmd, FsRep, FsServiceHandle, GpuCmd, GpuRep,
    GpuServiceHandle, KernelCmd, KernelRep, KernelServiceHandle, Service, ServicesHub,
    ServicesHubBuilder,
};
use std::sync::Arc;

/// No-op kernel service used in tests.
pub struct NoopKernel;

impl Service for NoopKernel {
    type Cmd = KernelCmd;
    type Rep = KernelRep;
}

impl NoopKernel {
    fn handle() -> KernelServiceHandle {
        Arc::new(Self)
    }
}

/// No-op filesystem service used in tests.
pub struct NoopFs;

impl Service for NoopFs {
    type Cmd = FsCmd;
    type Rep = FsRep;
}

impl NoopFs {
    fn handle() -> FsServiceHandle {
        Arc::new(Self)
    }
}

/// No-op GPU service used in tests.
pub struct NoopGpu;

impl Service for NoopGpu {
    type Cmd = GpuCmd;
    type Rep = GpuRep;
}

impl NoopGpu {
    fn handle() -> GpuServiceHandle {
        Arc::new(Self)
    }
}

/// No-op audio service used in tests.
pub struct NoopAudio;

impl Service for NoopAudio {
    type Cmd = AudioCmd;
    type Rep = AudioRep;
}

impl NoopAudio {
    fn handle() -> AudioServiceHandle {
        Arc::new(Self)
    }
}

/// Creates a mock services hub with default capacities.
pub fn make_hub() -> ServicesHub {
    make_hub_with_capacities(64, 32, 128, 128)
}

/// Creates a mock services hub with custom capacities for each service.
///
/// Capacity parameters are accepted for API compatibility but ignored because the
/// no-op services do not enqueue work.
pub fn make_hub_with_capacities(
    _kernel: usize,
    _fs: usize,
    _gpu: usize,
    _audio: usize,
) -> ServicesHub {
    ServicesHubBuilder::new()
        .kernel(NoopKernel::handle())
        .fs(NoopFs::handle())
        .gpu(NoopGpu::handle())
        .audio(NoopAudio::handle())
        .build()
        .expect("mock hub build")
}
