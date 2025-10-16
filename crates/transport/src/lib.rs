//! Core transport primitives shared by native and web backends.
//!
//! This module exposes the foundational pieces described in the transport spec:
//! * [`SharedRegion`] – contiguous, aligned memory slices which back the rings.
//! * [`MsgRing`] – single-producer/single-consumer command/report queue encoded with rkyv.
//! * [`ProducerGrant`] / [`Record`] – ergonomic producer/consumer views that avoid callbacks.
//! * [`TransportError`] – lightweight error surface for allocation/config failures.

mod error;
mod msg_ring;
mod region;
pub mod schema;
mod slot_pool;
pub mod wasm;

pub use error::{TransportError, TransportResult};
pub use msg_ring::{Envelope, MsgRing, ProducerGrant, Record};
pub use region::{SharedRegion, Uninit, Zeroed};
pub use schema::*;
pub use slot_pool::{SlotPool, SlotPoolConfig, SlotPop, SlotPush, SLOT_ALIGNMENT};
