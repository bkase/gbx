use hub::{Service, SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::sync::Arc;
use transport::SlotPool;

use crate::codec::Codec;
use crate::error::{FabricError, FabricResult};
use crate::port::{ConsumerPort, ProducerPort};

/// Handle exposed to the scheduler for submitting commands and draining reports.
#[derive(Clone)]
pub struct EndpointHandle<C: Codec> {
    pub(crate) lossless: Option<ProducerPort>,
    pub(crate) besteffort: Option<ProducerPort>,
    pub(crate) coalesce: Option<ProducerPort>,
    pub(crate) replies: ConsumerPort,
    pub(crate) slot_pools: Vec<Arc<Mutex<SlotPool>>>,
    pub(crate) codec: C,
}

impl<C: Codec> EndpointHandle<C> {
    pub fn submit(&self, cmd: &C::Cmd) -> FabricResult<SubmitOutcome> {
        let encoded = self.codec.encode_cmd(cmd)?;
        let port = match encoded.policy {
            SubmitPolicy::Must | SubmitPolicy::Lossless => self.lossless.as_ref(),
            SubmitPolicy::BestEffort => self.besteffort.as_ref(),
            SubmitPolicy::Coalesce => self.coalesce.as_ref(),
        }
        .ok_or(FabricError::InvalidConfig("missing port for submit policy"))?;
        port.try_send(encoded.envelope, encoded.payload.as_slice())
    }

    pub fn drain_reports(&self, max: usize) -> FabricResult<SmallVec<[C::Rep; 8]>> {
        let mut out = SmallVec::<[C::Rep; 8]>::new();
        self.replies.drain_records(max, |envelope, payload| {
            if out.len() >= max {
                return;
            }
            match self.codec.decode_rep(envelope, payload) {
                Ok(rep) => out.push(rep),
                Err(err) => {
                    tracing::error!("failed to decode report: {err}");
                }
            }
        })?;
        Ok(out)
    }

    /// Returns a slice of all configured slot pools.
    pub fn slot_pools(&self) -> &[Arc<Mutex<SlotPool>>] {
        &self.slot_pools
    }
}

/// Worker-facing endpoint used by service engines inside the runtime.
#[derive(Clone)]
pub struct WorkerEndpoint<C: Codec> {
    pub(crate) lossless: Option<ConsumerPort>,
    pub(crate) besteffort: Option<ConsumerPort>,
    pub(crate) coalesce: Option<ConsumerPort>,
    pub(crate) replies: ProducerPort,
    pub(crate) slot_pools: Vec<Arc<Mutex<SlotPool>>>,
    pub(crate) codec: C,
}

impl<C: Codec> WorkerEndpoint<C> {
    pub fn drain_commands<F>(&self, max: usize, mut f: F) -> FabricResult<usize>
    where
        F: FnMut(&C::Cmd),
    {
        let mut drained = 0;

        drained += self.drain_port(max, &self.lossless, &mut f)?;
        if drained >= max {
            return Ok(drained);
        }

        drained += self.drain_port(max - drained, &self.coalesce, &mut f)?;
        if drained >= max {
            return Ok(drained);
        }

        drained += self.drain_port(max - drained, &self.besteffort, &mut f)?;
        Ok(drained)
    }

    pub fn publish_report(&self, rep: &C::Rep) -> FabricResult<SubmitOutcome> {
        let encoded = self.codec.encode_rep(rep)?;
        self.replies
            .try_send(encoded.envelope, encoded.payload.as_slice())
    }

    /// Returns a slice of all configured slot pools.
    pub fn slot_pools(&self) -> &[Arc<Mutex<SlotPool>>] {
        &self.slot_pools
    }

    fn drain_port<F>(
        &self,
        max: usize,
        port: &Option<ConsumerPort>,
        f: &mut F,
    ) -> FabricResult<usize>
    where
        F: FnMut(&C::Cmd),
    {
        let Some(port) = port else {
            return Ok(0);
        };
        let codec = &self.codec;
        let mut decoded = Vec::new();
        let mut drained = 0;
        port.drain_records(max, |envelope, payload| {
            if drained >= max {
                return;
            }
            match codec.decode_cmd(envelope, payload) {
                Ok(cmd) => {
                    decoded.push(cmd);
                    drained += 1;
                }
                Err(err) => {
                    tracing::error!("failed to decode command: {err}");
                }
            }
        })?;

        for cmd in decoded.iter() {
            f(cmd);
        }

        Ok(drained)
    }
}

/// Generic service adapter bridging the `Service` trait onto a transport endpoint.
pub struct ServiceAdapter<C: Codec> {
    handle: EndpointHandle<C>,
}

impl<C: Codec> ServiceAdapter<C> {
    pub fn new(handle: EndpointHandle<C>) -> Self {
        Self { handle }
    }
}

impl<C: Codec + Send + Sync + 'static> Service for ServiceAdapter<C> {
    type Cmd = C::Cmd;
    type Rep = C::Rep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        match self.handle.submit(cmd) {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::error!("service submit failed: {err}");
                SubmitOutcome::Closed
            }
        }
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        match self.handle.drain_reports(max) {
            Ok(reps) => reps,
            Err(err) => {
                tracing::error!("service drain failed: {err}");
                SmallVec::new()
            }
        }
    }
}
