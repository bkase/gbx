use thiserror::Error;

use transport::TransportError;

pub type FabricResult<T> = Result<T, FabricError>;

#[derive(Debug, Error)]
pub enum FabricError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("codec error: {0}")]
    Codec(String),

    #[error("invalid fabric configuration: {0}")]
    InvalidConfig(&'static str),

    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
}

impl FabricError {
    pub fn codec(msg: impl Into<String>) -> Self {
        FabricError::Codec(msg.into())
    }
}
