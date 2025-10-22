use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use transport::{Envelope, Mailbox, MailboxSend, MsgRing};

use crate::codec::PortClass;
use crate::error::{FabricError, FabricResult};
#[cfg(target_arch = "wasm32")]
use crate::layout::PortLayout;
use crate::service::SubmitOutcome;

enum Backend {
    MsgRing(Mutex<MsgRing>),
    Mailbox(Mutex<Mailbox>),
}

pub struct SharedPort {
    class: PortClass,
    backend: Backend,
    metrics: PortMetrics,
}

impl SharedPort {
    pub fn new_ring(class: PortClass, ring: MsgRing) -> Arc<Self> {
        Arc::new(Self {
            class,
            backend: Backend::MsgRing(Mutex::new(ring)),
            metrics: PortMetrics::new(),
        })
    }

    pub fn new_mailbox(mailbox: Mailbox) -> Arc<Self> {
        Arc::new(Self {
            class: PortClass::Coalesce,
            backend: Backend::Mailbox(Mutex::new(mailbox)),
            metrics: PortMetrics::new(),
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

    pub fn metrics(&self) -> PortMetricsSnapshot {
        self.metrics.snapshot()
    }

    fn record(&self, outcome: SubmitOutcome) {
        self.metrics.record(outcome);
    }
}

#[derive(Clone)]
pub struct ProducerPort {
    inner: Arc<SharedPort>,
}

impl ProducerPort {
    pub fn try_send(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<SubmitOutcome> {
        let result = match (&self.inner.backend, self.inner.class) {
            (Backend::MsgRing(ring), PortClass::Lossless) => {
                send_ring_lossless(&mut ring.lock(), envelope, payload)
            }
            (Backend::MsgRing(ring), PortClass::BestEffort) => {
                send_ring_besteffort(&mut ring.lock(), envelope, payload)
            }
            (Backend::Mailbox(mailbox), PortClass::Coalesce) => {
                send_mailbox(&mut mailbox.lock(), envelope, payload)
            }
            (Backend::MsgRing(_), PortClass::Coalesce) => Err(FabricError::InvalidConfig(
                "coalesce class requires mailbox backend",
            )),
            (Backend::Mailbox(_), _) => Err(FabricError::InvalidConfig(
                "mailbox backend only supports coalesce class",
            )),
        };

        if let Ok(outcome) = &result {
            self.inner.record(*outcome);
        }

        result
    }

    pub fn metrics(&self) -> PortMetricsSnapshot {
        self.inner.metrics()
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

    pub fn metrics(&self) -> PortMetricsSnapshot {
        self.inner.metrics()
    }
}

#[derive(Default)]
struct PortMetrics {
    accepted: AtomicU32,
    coalesced: AtomicU32,
    dropped: AtomicU32,
    would_block: AtomicU32,
}

impl PortMetrics {
    fn new() -> Self {
        Self::default()
    }

    fn record(&self, outcome: SubmitOutcome) {
        match outcome {
            SubmitOutcome::Accepted => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
            }
            SubmitOutcome::Coalesced => {
                self.coalesced.fetch_add(1, Ordering::Relaxed);
            }
            SubmitOutcome::Dropped => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            SubmitOutcome::WouldBlock => {
                self.would_block.fetch_add(1, Ordering::Relaxed);
            }
            SubmitOutcome::Closed => {}
        }
    }

    fn snapshot(&self) -> PortMetricsSnapshot {
        PortMetricsSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            would_block: self.would_block.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PortMetricsSnapshot {
    pub accepted: u32,
    pub coalesced: u32,
    pub dropped: u32,
    pub would_block: u32,
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

pub fn make_port_pair_ring(class: PortClass, ring: MsgRing) -> PortPair {
    let shared = SharedPort::new_ring(class, ring);
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
