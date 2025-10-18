#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
use hub::{
    AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle, ServicesHub,
    ServicesHubBuilder, SubmitPolicy,
};
use transport::schema::{
    TAG_AUDIO_CMD, TAG_AUDIO_REP, TAG_FS_CMD, TAG_FS_REP, TAG_GPU_CMD, TAG_GPU_REP, TAG_KERNEL_CMD,
    TAG_KERNEL_REP,
};
use transport_codecs::{AudioCodec, FsCodec, GpuCodec, KernelCodec};
use transport_fabric::{
    build_service, EndpointHandle, FabricLayout, MailboxSpec, RingSpec, ServiceAdapter,
    ServiceSpec, WorkerEndpoint,
};

/// Aggregates transport-backed service adapters alongside the worker topology.
pub struct TransportServices {
    pub hub: ServicesHub,
    pub worker: WorkerTopology,
}

pub struct WorkerTopology {
    pub kernel: WorkerEndpoint<KernelCodec>,
    pub fs: WorkerEndpoint<FsCodec>,
    pub gpu: WorkerEndpoint<GpuCodec>,
    pub audio: WorkerEndpoint<AudioCodec>,
    pub layout: FabricLayout,
}

impl TransportServices {
    /// Instantiates the default transport fabric and service hub.
    pub fn new() -> Result<Self> {
        #[allow(unused_mut)]
        let mut layout = FabricLayout::default();

        let kernel_spec = ServiceSpec {
            codec: KernelCodec,
            lossless: Some(RingSpec {
                capacity_bytes: 32 * 1024,
                envelope_tag: TAG_KERNEL_CMD,
            }),
            besteffort: Some(RingSpec {
                capacity_bytes: 32 * 1024,
                envelope_tag: TAG_KERNEL_CMD,
            }),
            coalesce: Some(MailboxSpec {
                payload_bytes: 64 * 1024,
                envelope_tag: TAG_KERNEL_CMD,
            }),
            replies: RingSpec {
                capacity_bytes: 512 * 1024,
                envelope_tag: TAG_KERNEL_REP,
            },
            reply_policy: SubmitPolicy::Lossless,
        };
        let (kernel, kernel_worker, _kernel_layout) = build_service(kernel_spec)?;
        #[cfg(target_arch = "wasm32")]
        layout.add_endpoint(_kernel_layout);

        let fs_spec = ServiceSpec {
            codec: FsCodec,
            lossless: Some(RingSpec {
                capacity_bytes: 32 * 1024,
                envelope_tag: TAG_FS_CMD,
            }),
            besteffort: None,
            coalesce: None,
            replies: RingSpec {
                capacity_bytes: 64 * 1024,
                envelope_tag: TAG_FS_REP,
            },
            reply_policy: SubmitPolicy::Lossless,
        };
        let (fs, fs_worker, _fs_layout) = build_service(fs_spec)?;
        #[cfg(target_arch = "wasm32")]
        layout.add_endpoint(_fs_layout);

        let gpu_spec = ServiceSpec {
            codec: GpuCodec,
            lossless: Some(RingSpec {
                capacity_bytes: 64 * 1024,
                envelope_tag: TAG_GPU_CMD,
            }),
            besteffort: None,
            coalesce: None,
            replies: RingSpec {
                capacity_bytes: 64 * 1024,
                envelope_tag: TAG_GPU_REP,
            },
            reply_policy: SubmitPolicy::Lossless,
        };
        let (gpu, gpu_worker, _gpu_layout) = build_service(gpu_spec)?;
        #[cfg(target_arch = "wasm32")]
        layout.add_endpoint(_gpu_layout);

        let audio_spec = ServiceSpec {
            codec: AudioCodec,
            lossless: Some(RingSpec {
                capacity_bytes: 32 * 1024,
                envelope_tag: TAG_AUDIO_CMD,
            }),
            besteffort: None,
            coalesce: None,
            replies: RingSpec {
                capacity_bytes: 32 * 1024,
                envelope_tag: TAG_AUDIO_REP,
            },
            reply_policy: SubmitPolicy::Lossless,
        };
        let (audio, audio_worker, _audio_layout) = build_service(audio_spec)?;
        #[cfg(target_arch = "wasm32")]
        layout.add_endpoint(_audio_layout);

        let worker = WorkerTopology {
            kernel: kernel_worker,
            fs: fs_worker,
            gpu: gpu_worker,
            audio: audio_worker,
            layout,
        };

        let hub = ServicesHubBuilder::new()
            .kernel(kernel_handle(kernel))
            .fs(fs_handle(fs))
            .gpu(gpu_handle(gpu))
            .audio(audio_handle(audio))
            .build()
            .expect("transport services hub build");

        Ok(Self { hub, worker })
    }
}

fn kernel_handle(endpoint: EndpointHandle<KernelCodec>) -> KernelServiceHandle {
    Arc::new(ServiceAdapter::new(endpoint)) as KernelServiceHandle
}

fn fs_handle(endpoint: EndpointHandle<FsCodec>) -> FsServiceHandle {
    Arc::new(ServiceAdapter::new(endpoint)) as FsServiceHandle
}

fn gpu_handle(endpoint: EndpointHandle<GpuCodec>) -> GpuServiceHandle {
    Arc::new(ServiceAdapter::new(endpoint)) as GpuServiceHandle
}

fn audio_handle(endpoint: EndpointHandle<AudioCodec>) -> AudioServiceHandle {
    Arc::new(ServiceAdapter::new(endpoint)) as AudioServiceHandle
}
