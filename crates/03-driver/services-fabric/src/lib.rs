#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
use service_abi::{AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle};
use transport::schema::{
    TAG_AUDIO_CMD, TAG_AUDIO_REP, TAG_FS_CMD, TAG_FS_REP, TAG_GPU_CMD, TAG_GPU_REP, TAG_KERNEL_CMD,
    TAG_KERNEL_REP,
};
use transport::SlotPoolConfig;
use transport_codecs::{AudioCodec, FsCodec, GpuCodec, KernelCodec};
use transport_fabric::{
    build_service, EndpointHandle, FabricLayout, MailboxSpec, PortClass, RingSpec, ServiceAdapter,
    ServiceSpec, SlotPoolSpec, WorkerEndpoint,
};

/// Aggregates transport-backed service endpoints and worker topology.
pub struct TransportServices {
    pub kernel: KernelServiceHandle,
    pub fs: FsServiceHandle,
    pub gpu: GpuServiceHandle,
    pub audio: AudioServiceHandle,
    pub worker: WorkerTopology,
    pub scheduler: SchedulerTopology,
}

pub struct WorkerTopology {
    pub kernel: WorkerEndpoint<KernelCodec>,
    pub fs: WorkerEndpoint<FsCodec>,
    pub gpu: WorkerEndpoint<GpuCodec>,
    pub audio: WorkerEndpoint<AudioCodec>,
    pub layout: FabricLayout,
}

pub struct SchedulerTopology {
    pub kernel: EndpointHandle<KernelCodec>,
    pub fs: EndpointHandle<FsCodec>,
    pub gpu: EndpointHandle<GpuCodec>,
    pub audio: EndpointHandle<AudioCodec>,
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
            reply_policy: PortClass::Lossless,
            slot_pools: vec![
                // Frame pool (index 0)
                SlotPoolSpec {
                    config: SlotPoolConfig {
                        slot_count: 8,
                        slot_size: 128 * 1024,
                    },
                },
                // Audio pool (index 1)
                SlotPoolSpec {
                    config: SlotPoolConfig {
                        slot_count: 6,
                        slot_size: 16 * 1024,
                    },
                },
            ],
        };
        let (kernel_endpoint, kernel_worker, _kernel_layout) = build_service(kernel_spec)?;
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
            reply_policy: PortClass::Lossless,
            slot_pools: vec![],
        };
        let (fs_endpoint, fs_worker, _fs_layout) = build_service(fs_spec)?;
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
            reply_policy: PortClass::Lossless,
            slot_pools: vec![],
        };
        let (gpu_endpoint, gpu_worker, _gpu_layout) = build_service(gpu_spec)?;
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
            reply_policy: PortClass::Lossless,
            slot_pools: vec![],
        };
        let (audio_endpoint, audio_worker, _audio_layout) = build_service(audio_spec)?;
        #[cfg(target_arch = "wasm32")]
        layout.add_endpoint(_audio_layout);

        let worker = WorkerTopology {
            kernel: kernel_worker,
            fs: fs_worker,
            gpu: gpu_worker,
            audio: audio_worker,
            layout,
        };

        let scheduler = SchedulerTopology {
            kernel: kernel_endpoint.clone(),
            fs: fs_endpoint.clone(),
            gpu: gpu_endpoint.clone(),
            audio: audio_endpoint.clone(),
        };

        Ok(Self {
            kernel: Arc::new(ServiceAdapter::new(kernel_endpoint)) as KernelServiceHandle,
            fs: Arc::new(ServiceAdapter::new(fs_endpoint)) as FsServiceHandle,
            gpu: Arc::new(ServiceAdapter::new(gpu_endpoint)) as GpuServiceHandle,
            audio: Arc::new(ServiceAdapter::new(audio_endpoint)) as AudioServiceHandle,
            worker,
            scheduler,
        })
    }
}
