//! Top-level WASM module for GBX.
//!
//! This crate serves as the single WASM artifact that contains:
//! - Re-exported worker functions from transport-worker (reusable)
//! - Test orchestration code (GBX-specific)
//!
//! Following the single-module pattern to avoid __wbindgen_start deadlocks.

#![allow(missing_docs)]

// Re-export all worker functions from transport-worker
// These are the reusable, app-agnostic worker entry points

// Fabric worker functions (generalized runtime)
pub use transport_worker::{fabric_worker_init, fabric_worker_run};

// Test scenario worker functions (built on fabric layer)
pub use transport_worker::{worker_register_test, ScenarioStats, TestConfig};

// Re-export types for convenience
pub use transport_worker::{EndpointLayout, FabricLayout, PortLayout, PortRole};

// Test orchestration - GBX-specific integration tests
#[cfg(target_arch = "wasm32")]
pub mod tests;
