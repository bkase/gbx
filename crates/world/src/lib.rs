//! Canonical world state primitives and message types.
//!
//! The `world` crate intentionally stays small.  It defines the shared enums and
//! structs that frontends, schedulers, and services compile against, along with
//! a minimal `World` container used by tests and early reducers.

/// Pure intent reducer for Wave B scaffolding.
pub mod reduce_intent;
/// Pure report reducer for Wave B scaffolding.
pub mod reduce_report;
/// Non-blocking service abstractions and hub coordination.
pub mod services;
/// Core message and policy types for the emulator world.
pub mod types;
/// Minimal world state container used by early reducers and tests.
pub mod world;

pub use crate::reduce_intent::IntentReducer;
pub use crate::reduce_report::ReportReducer;
pub use crate::services::{
    AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle, Service,
    ServicesHub, ServicesHubBuilder,
};
pub use crate::types::{
    AudioCmd, AudioRep, AudioSpan, AvCmd, FollowUps, FrameSpan, FsCmd, FsRep, GpuCmd, GpuRep,
    Intent, IntentPriority, KernelCmd, KernelRep, Report, SubmitOutcome, SubmitPolicy, TickPurpose,
    WorkCmd,
};
pub use crate::world::{World, WorldHealth, WorldPerf};

/// Default budget for processing intents per scheduler tick.
pub const DEFAULT_INTENT_BUDGET: usize = 3;
/// Default budget for draining reports per scheduler tick.
pub const DEFAULT_REPORT_BUDGET: usize = 32;
