//! Canonical world state primitives and message types.
//!
//! The `world` crate intentionally stays small.  It defines the shared enums and
//! structs that frontends, schedulers, and services compile against, along with
//! a minimal `World` container used by tests and early reducers.

#![deny(missing_docs)]

/// Core message and policy types for the emulator world.
pub mod types;
/// Minimal world state container used by early reducers and tests.
pub mod world;

pub use crate::types::{
    AudioCmd, AudioRep, AudioSpan, AvCmd, FollowUps, FrameSpan, FsCmd, FsRep, GpuCmd, GpuRep,
    Intent, KernelCmd, KernelRep, Report, SubmitOutcome, SubmitPolicy, TickPurpose, WorkCmd,
};
pub use crate::world::{World, WorldHealth, WorldPerf};
