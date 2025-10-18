use std::sync::Arc;

use hub::{SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use transport::{Envelope, Mailbox, MailboxSend, MsgRing};

use crate::error::{FabricError, FabricResult};
#[cfg(target_arch = "wasm32")]
use crate::layout::PortLayout;

enum Backend {
    MsgRing(Mutex<MsgRing>),
    Mailbox(Mutex<Mailbox>),
}

pub struct SharedPort {
    policy: SubmitPolicy,
    backend: Backend,
}

impl SharedPort {
    pub fn new_ring(policy: SubmitPolicy, ring: MsgRing) -> Arc<Self> {
        Arc::new(Self {
            policy,
            backend: Backend::MsgRing(Mutex::new(ring)),
        })
    }

    pub fn new_mailbox(mailbox: Mailbox) -> Arc<Self> {
        Arc::new(Self {
            policy: SubmitPolicy::Coalesce,
            backend: Backend::Mailbox(Mutex::new(mailbox)),
        })
    }

    pub fn producer(self: &Arc<Self>) -> ProducerPort {
        ProducerPort {
            inner: Arc::clone(self),
        }
    }

    pub fn consumer(self: &Arc<Self>) -> ConsumerPort {
        ConsumerPort {
            inner: Arc::clone(self),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn wasm_layout(&self) -> PortLayout {
        match &self.backend {
            Backend::MsgRing(ring) => PortLayout::MsgRing(ring.lock().wasm_layout()),
            Backend::Mailbox(mailbox) => PortLayout::Mailbox(mailbox.lock().wasm_layout()),
        }
    }
}

#[derive(Clone)]
pub struct ProducerPort {
    inner: Arc<SharedPort>,
}

impl ProducerPort {
    pub fn try_send(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<SubmitOutcome> {
        match (&self.inner.backend, self.inner.policy) {
            (Backend::MsgRing(ring), SubmitPolicy::Lossless | SubmitPolicy::Must) => {
                send_ring_lossless(&mut ring.lock(), envelope, payload)
            }
            (Backend::MsgRing(ring), SubmitPolicy::BestEffort) => {
                send_ring_besteffort(&mut ring.lock(), envelope, payload)
            }
            (Backend::Mailbox(mailbox), SubmitPolicy::Coalesce) => {
                send_mailbox(&mut mailbox.lock(), envelope, payload)
            }
            (Backend::MsgRing(_), SubmitPolicy::Coalesce) => Err(FabricError::InvalidConfig(
                "coalesce policy requires mailbox backend",
            )),
            (Backend::Mailbox(_), _) => Err(FabricError::InvalidConfig(
                "mailbox backend only supports coalesce policy",
            )),
        }
    }
}

#[derive(Clone)]
pub struct ConsumerPort {
    inner: Arc<SharedPort>,
}

impl ConsumerPort {
    pub fn drain_records<F>(&self, max: usize, mut f: F) -> FabricResult<usize>
    where
        F: FnMut(Envelope, &[u8]),
    {
        if max == 0 {
            return Ok(0);
        }

        match &self.inner.backend {
            Backend::MsgRing(ring) => {
                let mut ring = ring.lock();
                let mut drained = 0;
                while drained < max {
                    if let Some(record) = ring.consumer_peek() {
                        f(record.envelope, record.payload);
                        ring.consumer_pop_advance();
                        drained += 1;
                    } else {
                        break;
                    }
                }
                Ok(drained)
            }
            Backend::Mailbox(mailbox) => {
                let mut mailbox = mailbox.lock();
                if let Some(record) = mailbox.take_latest() {
                    f(record.envelope, record.payload);
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
        }
    }
}

fn send_ring_lossless(
    ring: &mut MsgRing,
    envelope: Envelope,
    payload: &[u8],
) -> FabricResult<SubmitOutcome> {
    match reserve_and_commit(ring, envelope, payload) {
        Some(()) => Ok(SubmitOutcome::Accepted),
        None => Ok(SubmitOutcome::WouldBlock),
    }
}

fn send_ring_besteffort(
    ring: &mut MsgRing,
    envelope: Envelope,
    payload: &[u8],
) -> FabricResult<SubmitOutcome> {
    match reserve_and_commit(ring, envelope, payload) {
        Some(()) => Ok(SubmitOutcome::Accepted),
        None => Ok(SubmitOutcome::Dropped),
    }
}

fn reserve_and_commit(ring: &mut MsgRing, envelope: Envelope, payload: &[u8]) -> Option<()> {
    let mut grant = ring.try_reserve_with(envelope, payload.len())?;
    let buf = grant.payload();
    if payload.len() > buf.len() {
        return None;
    }
    buf[..payload.len()].copy_from_slice(payload);
    grant.commit(payload.len());
    Some(())
}

fn send_mailbox(
    mailbox: &mut Mailbox,
    envelope: Envelope,
    payload: &[u8],
) -> FabricResult<SubmitOutcome> {
    match mailbox.try_send(payload, Some(envelope))? {
        MailboxSend::Accepted => Ok(SubmitOutcome::Accepted),
        MailboxSend::Coalesced => Ok(SubmitOutcome::Coalesced),
    }
}

pub struct PortPair {
    pub producer: ProducerPort,
    pub consumer: ConsumerPort,
}

pub fn make_port_pair_ring(policy: SubmitPolicy, ring: MsgRing) -> PortPair {
    let shared = SharedPort::new_ring(policy, ring);
    PortPair {
        producer: shared.producer(),
        consumer: shared.consumer(),
    }
}

pub fn make_port_pair_mailbox(mailbox: Mailbox) -> PortPair {
    let shared = SharedPort::new_mailbox(mailbox);
    PortPair {
        producer: shared.producer(),
        consumer: shared.consumer(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn shared_from_producer(port: &ProducerPort) -> Arc<SharedPort> {
    Arc::clone(&port.inner)
}

#[cfg(target_arch = "wasm32")]
pub fn shared_from_consumer(port: &ConsumerPort) -> Arc<SharedPort> {
    Arc::clone(&port.inner)
}
