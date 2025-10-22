#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]

//! Kernel execution core for the GBX emulator.
//!
//! The crate exposes scalar-first CPU, bus, and scheduler primitives that can
//! later be specialised for SIMD execution without touching instruction logic.

/// Bus traits and scalar IO implementations.
pub mod bus;
/// Central CPU + scheduler core implementation.
pub mod core;
/// CPU register file representation.
pub mod cpu;
/// Execution backend abstractions.
pub mod exec;
/// Instruction helpers and opcode implementations.
pub mod instr;
/// Memory management unit helpers.
pub mod mmu;
/// Placeholder PPU implementation used for tests.
pub mod ppu_stub;
/// Serialization helpers for saving and restoring core state.
pub mod state;
/// Timer block abstraction shared across services.
pub mod timers;

/// Re-export of bus traits and scalar IO structures.
pub use bus::{Bus, BusScalar, IoRegs};
/// Re-export of the high-level core types.
pub use core::{Core, CoreConfig, Model};
/// Re-export of execution backend traits and scalar implementation.
pub use exec::{Exec, Flags, MaskValue, Scalar};

#[cfg(test)]
mod tests;
