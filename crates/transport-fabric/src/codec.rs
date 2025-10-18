use hub::SubmitPolicy;
use transport::Envelope;

use crate::error::FabricResult;

pub struct Encoded {
    pub policy: SubmitPolicy,
    pub envelope: Envelope,
    pub payload: Vec<u8>,
}

impl Encoded {
    pub fn new(policy: SubmitPolicy, envelope: Envelope, payload: Vec<u8>) -> Self {
        Self {
            policy,
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
