//! Top-level WASM module for GBX.
//!
//! This crate serves as the single WASM artifact that contains:
//! - Re-exported worker functions from transport-worker (reusable)
//! - Test orchestration code (GBX-specific)
//!
//! Following the single-module pattern to avoid __wbindgen_start deadlocks.

#![allow(missing_docs)]

// Re-export all worker functions from fabric-worker-wasm
// These are the reusable, app-agnostic worker entry points

// Fabric worker functions (generalized runtime)
pub use fabric_worker_wasm::{fabric_worker_init, fabric_worker_run};

// Test scenario worker functions (built on fabric layer)
pub use fabric_worker_wasm::{worker_register_test, ScenarioStats, TestConfig};

// Service registration (GBX-specific, layer 06)
#[cfg(target_arch = "wasm32")]
mod worker_services;
#[cfg(target_arch = "wasm32")]
pub use worker_services::worker_register_services;

// Re-export types for convenience
pub use fabric_worker_wasm::{EndpointLayout, FabricLayout, PortLayout, PortRole};

// Test orchestration - GBX-specific integration tests
#[cfg(target_arch = "wasm32")]
pub mod tests;

// GBX UI exports
#[cfg(target_arch = "wasm32")]
mod ui;

#[cfg(target_arch = "wasm32")]
pub use ui::{gbx_consume_frame, gbx_debug_state, gbx_init, gbx_load_rom, gbx_tick};
