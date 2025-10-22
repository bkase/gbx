#![cfg_attr(target_arch = "wasm32", feature(stdarch_wasm_atomic_wait))]
#![allow(missing_docs)]

mod builder;
mod codec;
mod endpoint;
mod error;
pub mod layout;
mod port;
mod runtime;
mod service;
mod span;

pub use builder::{build_service, MailboxSpec, RingSpec, ServiceSpec, SlotPoolSpec};
pub use codec::{Codec, Encoded, PortClass};
pub use endpoint::{EndpointHandle, ServiceAdapter, WorkerEndpoint};
pub use error::{FabricError, FabricResult};
pub use layout::{ArchivedFabricLayout, EndpointLayout, FabricLayout, PortLayout, PortRole};
pub use port::{make_port_pair_mailbox, make_port_pair_ring};
pub use runtime::{ServiceEngine, WorkerRuntime};
pub use service::{Service, SubmitOutcome};
pub use span::SlotSpan;
