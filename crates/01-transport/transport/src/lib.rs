#![cfg_attr(
    all(target_arch = "wasm32", not(feature = "loom")),
    feature(stdarch_wasm_atomic_wait)
)]
//! Core transport primitives shared by native and web backends.
//!
//! This module exposes the foundational pieces described in the transport spec:
//! * [`SharedRegion`] – contiguous, aligned memory slices which back the rings.
//! * [`MsgRing`] – single-producer/single-consumer command/report queue encoded with rkyv.
//! * [`ProducerGrant`] / [`Record`] – ergonomic producer/consumer views that avoid callbacks.
//! * [`TransportError`] – lightweight error surface for allocation/config failures.

mod error;
mod mailbox;
mod msg_ring;
mod region;
pub mod schema;
mod slot_pool;
pub mod wait;
pub mod wasm;

pub use error::{TransportError, TransportResult};
pub use mailbox::{Mailbox, MailboxRecord, MailboxSend};
pub use msg_ring::{Envelope, MsgRing, ProducerGrant, Record};
pub use region::{SharedRegion, Uninit, Zeroed};
pub use schema::*;
pub use slot_pool::{SlotPool, SlotPoolConfig, SlotPoolHandle, SlotPop, SlotPush, SLOT_ALIGNMENT};
