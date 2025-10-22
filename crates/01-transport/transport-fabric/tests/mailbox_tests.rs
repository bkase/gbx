//! Mailbox semantics integration tests.
//! This suite exercises coalescing behaviour, lossless prioritisation,
//! concurrency, and (optionally) property-based checks. Adjust imports
//! if your crate layout differs.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use transport::schema::SCHEMA_VERSION_V1;
use transport::Envelope;
use transport_fabric::{
    build_service, Codec, Encoded, EndpointHandle, FabricError, FabricResult, MailboxSpec,
    PortClass, RingSpec, ServiceSpec, SubmitOutcome, WorkerEndpoint, WorkerRuntime,
};

const CMD_TAG: u8 = 0xE1;
const REP_TAG: u8 = 0xE2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrainEvent {
    Lossless(u32),
    Coalesce(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MbxCmd {
    Set(u32),
    SetMany(u32),
    Lossless(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MbxRep {
    Ack(u32),
}

#[derive(Clone, Default)]
struct MbxCodec;

impl Codec for MbxCodec {
    type Cmd = MbxCmd;
    type Rep = MbxRep;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded> {
        let (variant, value, class) = match *cmd {
            MbxCmd::Set(x) => (0u8, x, PortClass::Coalesce),
            MbxCmd::SetMany(x) => (1, x, PortClass::Coalesce),
            MbxCmd::Lossless(x) => (2, x, PortClass::Lossless),
        };
        let mut payload = Vec::with_capacity(1 + std::mem::size_of::<u32>());
        payload.push(variant);
        payload.extend_from_slice(&value.to_le_bytes());
        Ok(Encoded::new(
            class,
            Envelope::new(CMD_TAG, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd> {
        if envelope.tag != CMD_TAG || envelope.ver != SCHEMA_VERSION_V1 {
            return Err(FabricError::codec("unexpected command envelope"));
        }
        if payload.len() != 1 + std::mem::size_of::<u32>() {
            return Err(FabricError::codec("invalid command payload length"));
        }
        let variant = payload[0];
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&payload[1..]);
        let value = u32::from_le_bytes(buf);
        match variant {
            0 => Ok(MbxCmd::Set(value)),
            1 => Ok(MbxCmd::SetMany(value)),
            2 => Ok(MbxCmd::Lossless(value)),
            _ => Err(FabricError::codec("unknown command variant")),
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let MbxRep::Ack(value) = *rep;
        let mut payload = Vec::with_capacity(std::mem::size_of::<u32>());
        payload.extend_from_slice(&value.to_le_bytes());
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(REP_TAG, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep> {
        if envelope.tag != REP_TAG || envelope.ver != SCHEMA_VERSION_V1 {
            return Err(FabricError::codec("unexpected report envelope"));
        }
        if payload.len() != std::mem::size_of::<u32>() {
            return Err(FabricError::codec("invalid report payload length"));
        }
        let mut buf = [0u8; 4];
        buf.copy_from_slice(payload);
        Ok(MbxRep::Ack(u32::from_le_bytes(buf)))
    }
}

struct Harness {
    handle: EndpointHandle<MbxCodec>,
    runtime: WorkerRuntime,
    drain_log: Arc<Mutex<Vec<DrainEvent>>>,
}

impl Harness {
    fn new() -> Self {
        let spec = ServiceSpec {
            codec: MbxCodec,
            lossless: Some(RingSpec {
                capacity_bytes: 1024,
                envelope_tag: CMD_TAG,
            }),
            besteffort: None,
            coalesce: Some(MailboxSpec {
                payload_bytes: 64,
                envelope_tag: CMD_TAG,
            }),
            replies: RingSpec {
                capacity_bytes: 1024,
                envelope_tag: REP_TAG,
            },
            reply_policy: PortClass::Lossless,
            slot_pools: Vec::new(),
        };
        let (handle, worker_endpoint, _layout) =
            build_service(spec).expect("build_service succeeded");
        let drain_log = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = WorkerRuntime::new();
        runtime.register(AckEngine {
            endpoint: worker_endpoint,
            drain_log: Arc::clone(&drain_log),
        });
        Self {
            handle,
            runtime,
            drain_log,
        }
    }

    fn tick_worker(&mut self) -> usize {
        self.runtime.run_tick()
    }

    fn drain_reports(&self, max: usize) -> Vec<MbxRep> {
        self.handle
            .drain_reports(max)
            .expect("drain reports")
            .into_iter()
            .collect()
    }

    fn drain_log(&self) -> Vec<DrainEvent> {
        self.drain_log.lock().unwrap().clone()
    }
}

struct AckEngine {
    endpoint: WorkerEndpoint<MbxCodec>,
    drain_log: Arc<Mutex<Vec<DrainEvent>>>,
}

impl transport_fabric::ServiceEngine for AckEngine {
    fn poll(&mut self) -> usize {
        let mut work = 0usize;
        let mut last_coalesce = None;
        let drain_log = Arc::clone(&self.drain_log);
        let drained = self
            .endpoint
            .drain_commands(1024, |cmd| {
                let mut log = drain_log.lock().unwrap();
                match cmd {
                    MbxCmd::Set(v) | MbxCmd::SetMany(v) => {
                        log.push(DrainEvent::Coalesce(*v));
                        last_coalesce = Some(*v);
                    }
                    MbxCmd::Lossless(v) => {
                        log.push(DrainEvent::Lossless(*v));
                        self.endpoint
                            .publish_report(&MbxRep::Ack(*v))
                            .expect("publish lossless ack");
                        work += 1;
                    }
                }
            })
            .expect("drain commands");
        work += drained;
        if let Some(value) = last_coalesce {
            self.endpoint
                .publish_report(&MbxRep::Ack(value))
                .expect("publish mailbox ack");
            work += 1;
        }
        work
    }

    fn name(&self) -> &'static str {
        "mailbox-test-engine"
    }
}

/// Submitting a single mailbox value should be accepted, yield one ack, and leave
/// the queue empty until the next write.
#[test]
fn mailbox_accept_then_drain_once() {
    let mut h = Harness::new();

    let o1 = h.handle.submit(&MbxCmd::Set(7)).expect("submit");
    assert!(matches!(o1, SubmitOutcome::Accepted));

    assert!(h.tick_worker() >= 1);
    let reps = h.drain_reports(8);
    assert_eq!(reps, vec![MbxRep::Ack(7)]);

    let reps = h.drain_reports(8);
    assert!(reps.is_empty());
}

/// Rapid writes should coalesce so the worker only acknowledges the final value.
#[test]
fn mailbox_coalesces_many_writes() {
    let mut h = Harness::new();

    let mut outcomes = Vec::new();
    for x in 1..=10 {
        outcomes.push(h.handle.submit(&MbxCmd::Set(x)).expect("submit"));
    }

    assert!(matches!(outcomes[0], SubmitOutcome::Accepted));
    assert!(outcomes
        .iter()
        .skip(1)
        .all(|o| matches!(o, SubmitOutcome::Coalesced)));

    assert!(h.tick_worker() >= 1);
    let reps = h.drain_reports(8);
    assert_eq!(reps, vec![MbxRep::Ack(10)]);
}

/// Mailbox submissions must never surface WouldBlock or Dropped since writes overwrite in place.
#[test]
fn mailbox_never_blocks_never_drops() {
    let h = Harness::new();

    for x in 0..1000u32 {
        let outcome = h.handle.submit(&MbxCmd::Set(x)).expect("submit");
        assert!(
            !matches!(outcome, SubmitOutcome::WouldBlock | SubmitOutcome::Dropped),
            "unexpected outcome {outcome:?}"
        );
    }
}

/// Lossless commands should be processed before mailbox work when both arrive together.
#[test]
fn mailbox_interleave_with_lossless_ring_priority() {
    let mut h = Harness::new();

    h.handle
        .submit(&MbxCmd::Set(1))
        .expect("submit mailbox value");
    h.handle
        .submit(&MbxCmd::Lossless(99))
        .expect("submit lossless value");

    assert!(h.tick_worker() >= 2);

    let reps = h.drain_reports(8);
    assert_eq!(reps, vec![MbxRep::Ack(99), MbxRep::Ack(1)]);

    let log = h.drain_log();
    assert_eq!(log, vec![DrainEvent::Lossless(99), DrainEvent::Coalesce(1)]);
}

/// Concurrent writers should see monotonic acknowledgements even if intermediate values are skipped.
#[test]
fn mailbox_concurrency_writer_fast_reader_slow() {
    let mut h = Harness::new();
    let handle = h.handle.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_write = Arc::clone(&stop);

    let writer = thread::spawn(move || {
        let start = Instant::now();
        let mut x = 0u32;
        while start.elapsed() < Duration::from_millis(100) {
            let _ = handle.submit(&MbxCmd::Set(x)).expect("submit");
            x = x.wrapping_add(1);
        }
        stop_write.store(true, Ordering::SeqCst);
    });

    let mut seen = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        if h.tick_worker() > 0 {
            let reps = h.drain_reports(64);
            seen.extend(reps.into_iter().map(|r| {
                let MbxRep::Ack(v) = r;
                v
            }));
        }
    }
    writer.join().unwrap();

    // Flush any remaining acknowledgements.
    h.tick_worker();
    let reps = h.drain_reports(64);
    seen.extend(reps.into_iter().map(|r| {
        let MbxRep::Ack(v) = r;
        v
    }));

    assert!(!seen.is_empty(), "expected at least one acknowledgement");
    for window in seen.windows(2) {
        assert!(
            window[1] >= window[0],
            "ack sequence must be non-decreasing: {window:?}"
        );
    }
}

/// The worker must only observe the latest mailbox value and emit no duplicate acknowledgements.
#[test]
fn mailbox_visibility_and_ordering() {
    let mut h = Harness::new();

    h.handle
        .submit(&MbxCmd::Set(111))
        .expect("submit first value");
    h.handle
        .submit(&MbxCmd::Set(222))
        .expect("submit second value");

    assert!(h.tick_worker() >= 1);
    let reps = h.drain_reports(8);
    assert_eq!(reps, vec![MbxRep::Ack(222)]);

    let reps = h.drain_reports(8);
    assert!(reps.is_empty());
}

#[cfg(feature = "proptest")]
mod prop {
    use super::*;
    use proptest::collection;
    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum Op {
        Write(u32),
        Drain,
    }

    proptest! {
        /// Random write/drain sequences must uphold last-write-wins semantics at every drain point.
        #[test]
        fn mailbox_last_write_wins_prop(seq in collection::vec(0u32..1000, 1..200)) {
            let mut h = Harness::new();
            let ops: Vec<Op> = seq.into_iter().flat_map(|x| {
                if x % 3 == 0 {
                    vec![Op::Write(x), Op::Drain]
                } else {
                    vec![Op::Write(x)]
                }
            }).collect::<Vec<Op>>();

            let mut delivered = Vec::<u32>::new();
            let mut last_before_drain: Option<u32> = None;

            for op in ops {
                match op {
                    Op::Write(x) => {
                        h.handle.submit(&MbxCmd::Set(x)).expect("submit");
                        last_before_drain = Some(x);
                    }
                    Op::Drain => {
                        let _ = h.tick_worker();
                        let reps = h.drain_reports(64);
                        if !reps.is_empty() {
                            prop_assert_eq!(reps.len(), 1);
                            let MbxRep::Ack(v) = reps[0];
                            delivered.push(v);
                            prop_assert_eq!(Some(v), last_before_drain);
                            last_before_drain = None;
                        }
                    }
                }
            }
        }
    }
}
