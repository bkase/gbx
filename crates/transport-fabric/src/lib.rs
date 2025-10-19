#![cfg_attr(target_arch = "wasm32", feature(stdarch_wasm_atomic_wait))]
#![allow(missing_docs)]

mod builder;
mod codec;
mod endpoint;
mod error;
pub mod layout;
mod port;
mod runtime;
mod span;

pub use builder::{build_service, MailboxSpec, RingSpec, ServiceSpec, SlotPoolSpec};
pub use codec::{Codec, Encoded};
pub use endpoint::{EndpointHandle, ServiceAdapter, WorkerEndpoint};
pub use error::{FabricError, FabricResult};
pub use layout::{EndpointLayout, FabricLayout, PortLayout, PortRole};
pub use runtime::{ServiceEngine, WorkerRuntime};
pub use span::SlotSpan;
