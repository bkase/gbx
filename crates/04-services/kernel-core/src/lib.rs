#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
#![feature(portable_simd)]

//! Kernel execution core for the GBX emulator.
//!
//! The crate exposes scalar-first CPU, bus, and scheduler primitives that can
//! later be specialised for SIMD execution without touching instruction logic.

/// Bus traits and scalar IO implementations.
pub mod bus;
/// SIMD bus wrapper combining multiple scalar lanes.
pub mod bus_simd;
/// Central CPU + scheduler core implementation.
pub mod core;
/// CPU register file representation.
pub mod cpu;
/// Execution backend abstractions.
pub mod exec;
/// SIMD execution backend built on `std::simd`.
pub mod exec_simd;
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

/// Re-export of bus traits and IO structures.
pub use bus::{Bus, BusScalar, IoRegs};
/// Re-export of the SIMD bus implementation.
pub use bus_simd::BusSimd;
/// Re-export of the high-level core types.
pub use core::{Core, CoreConfig, Model};
/// Re-export of execution backend traits and scalar implementation.
pub use exec::{Exec, Flags, MaskValue, Scalar};
/// Re-export of the SIMD execution backend.
pub use exec_simd::{LaneMask, SimdExec};

/// Convenience alias for a SIMD-configured core.
pub type SimdCore<const LANES: usize> = Core<SimdExec<LANES>, bus_simd::BusSimd<LANES>>;

#[cfg(test)]
mod tests;
