use transport::Envelope;

use crate::error::FabricResult;

/// Port class determines which queue type to use for message delivery.
/// This is purely a transport-level concept with no app-level semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortClass {
    /// Lossless delivery via ring buffer (blocking on full)
    Lossless,
    /// Best-effort delivery via ring buffer (drops on full)
    BestEffort,
    /// Coalescing delivery via mailbox (replaces previous value)
    Coalesce,
}

pub struct Encoded {
    pub class: PortClass,
    pub envelope: Envelope,
    pub payload: Vec<u8>,
}

impl Encoded {
    pub fn new(class: PortClass, envelope: Envelope, payload: Vec<u8>) -> Self {
        Self {
            class,
            envelope,
            payload,
        }
    }
}

pub trait Codec: Clone + Send + Sync + 'static {
    type Cmd: Send + 'static;
    type Rep: Send + 'static;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded>;
    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd>;
    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded>;
    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep>;
}
