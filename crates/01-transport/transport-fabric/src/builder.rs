use hub::SubmitPolicy;
use parking_lot::Mutex;
use std::sync::Arc;
use transport::{Envelope, Mailbox, MsgRing, SlotPool, SlotPoolConfig};

use crate::codec::Codec;
use crate::endpoint::{EndpointHandle, WorkerEndpoint};
use crate::error::FabricResult;
use crate::layout::EndpointLayout;
#[cfg(target_arch = "wasm32")]
use crate::layout::PortRole;
use crate::port::{make_port_pair_mailbox, make_port_pair_ring};
#[cfg(target_arch = "wasm32")]
use crate::port::{shared_from_consumer, shared_from_producer};

const SCHEMA_VER: u8 = transport::schema::SCHEMA_VERSION_V1;

/// Specification for a ring-based transport channel.
pub struct RingSpec {
    pub capacity_bytes: usize,
    pub envelope_tag: u8,
}

/// Specification for a mailbox-based transport channel (coalescing).
pub struct MailboxSpec {
    pub payload_bytes: usize,
    pub envelope_tag: u8,
}

/// Specification for a slot pool (frame/audio/bulk data).
pub struct SlotPoolSpec {
    pub config: SlotPoolConfig,
}

/// Complete specification for building a service endpoint pair.
pub struct ServiceSpec<C: Codec> {
    pub codec: C,
    pub lossless: Option<RingSpec>,
    pub besteffort: Option<RingSpec>,
    pub coalesce: Option<MailboxSpec>,
    pub replies: RingSpec,
    pub reply_policy: SubmitPolicy,
    pub slot_pools: Vec<SlotPoolSpec>,
}

/// Builds a bidirectional service endpoint pair from a specification.
///
/// Returns the scheduler-side handle, worker-side handle, and layout metadata.
pub fn build_service<C: Codec>(
    spec: ServiceSpec<C>,
) -> FabricResult<(EndpointHandle<C>, WorkerEndpoint<C>, EndpointLayout)> {
    #[allow(unused_mut)]
    let mut layout = EndpointLayout::default();

    let lossless_pair = if let Some(ring_spec) = spec.lossless {
        let ring = MsgRing::new(
            ring_spec.capacity_bytes,
            Envelope::new(ring_spec.envelope_tag, SCHEMA_VER),
        )?;
        let pair = make_port_pair_ring(SubmitPolicy::Lossless, ring);
        #[cfg(target_arch = "wasm32")]
        {
            let role = PortRole::CmdLossless;
            layout.push_port(role, shared_from_producer(&pair.producer).wasm_layout());
        }
        Some(pair)
    } else {
        None
    };

    let besteffort_pair = if let Some(ring_spec) = spec.besteffort {
        let ring = MsgRing::new(
            ring_spec.capacity_bytes,
            Envelope::new(ring_spec.envelope_tag, SCHEMA_VER),
        )?;
        let pair = make_port_pair_ring(SubmitPolicy::BestEffort, ring);
        #[cfg(target_arch = "wasm32")]
        {
            let role = PortRole::CmdBestEffort;
            layout.push_port(role, shared_from_producer(&pair.producer).wasm_layout());
        }
        Some(pair)
    } else {
        None
    };

    let coalesce_pair = if let Some(mailbox_spec) = spec.coalesce {
        let mailbox = Mailbox::new(
            mailbox_spec.payload_bytes,
            Envelope::new(mailbox_spec.envelope_tag, SCHEMA_VER),
        )?;
        let pair = make_port_pair_mailbox(mailbox);
        #[cfg(target_arch = "wasm32")]
        {
            let role = PortRole::CmdMailbox;
            layout.push_port(role, shared_from_producer(&pair.producer).wasm_layout());
        }
        Some(pair)
    } else {
        None
    };

    let replies_ring = MsgRing::new(
        spec.replies.capacity_bytes,
        Envelope::new(spec.replies.envelope_tag, SCHEMA_VER),
    )?;
    let replies_pair = make_port_pair_ring(spec.reply_policy, replies_ring);
    #[cfg(target_arch = "wasm32")]
    {
        let role = PortRole::Replies;
        layout.push_port(
            role,
            shared_from_consumer(&replies_pair.consumer).wasm_layout(),
        );
    }

    // Allocate slot pools
    let mut slot_pools = Vec::with_capacity(spec.slot_pools.len());
    #[cfg(target_arch = "wasm32")]
    let pool_specs_iter = spec.slot_pools.into_iter().enumerate();
    #[cfg(not(target_arch = "wasm32"))]
    let pool_specs_iter = spec.slot_pools.into_iter();

    for pool_item in pool_specs_iter {
        #[cfg(target_arch = "wasm32")]
        let (idx, pool_spec) = pool_item;
        #[cfg(not(target_arch = "wasm32"))]
        let pool_spec = pool_item;

        let pool = SlotPool::new(pool_spec.config)?;
        #[cfg(target_arch = "wasm32")]
        {
            use crate::layout::PortLayout;
            let role = PortRole::SlotPool(idx);
            layout.push_port(role, PortLayout::SlotPool(pool.wasm_layout()));
        }
        slot_pools.push(Arc::new(Mutex::new(pool)));
    }

    let endpoint = EndpointHandle {
        lossless: lossless_pair.as_ref().map(|p| p.producer.clone()),
        besteffort: besteffort_pair.as_ref().map(|p| p.producer.clone()),
        coalesce: coalesce_pair.as_ref().map(|p| p.producer.clone()),
        replies: replies_pair.consumer.clone(),
        slot_pools: slot_pools.clone(),
        codec: spec.codec.clone(),
    };

    let worker_endpoint = WorkerEndpoint {
        lossless: lossless_pair.as_ref().map(|p| p.consumer.clone()),
        besteffort: besteffort_pair.as_ref().map(|p| p.consumer.clone()),
        coalesce: coalesce_pair.as_ref().map(|p| p.consumer.clone()),
        replies: replies_pair.producer.clone(),
        slot_pools,
        codec: spec.codec,
    };

    Ok((endpoint, worker_endpoint, layout))
}
