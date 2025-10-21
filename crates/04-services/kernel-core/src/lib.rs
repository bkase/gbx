#![deny(unsafe_op_in_unsafe_fn)]
#![allow(missing_docs)]

//! Kernel execution core for the GBX emulator.
//!
//! The crate exposes scalar-first CPU, bus, and scheduler primitives that can
//! later be specialised for SIMD execution without touching instruction logic.

pub mod bus;
pub mod core;
pub mod cpu;
pub mod exec;
pub mod instr;
pub mod mmu;
pub mod ppu_stub;
pub mod state;
pub mod timers;

pub use bus::{Bus, BusScalar, IoRegs};
pub use core::{Core, CoreConfig, Model};
pub use exec::{Exec, Flags, MaskValue, Scalar};

#[cfg(test)]
mod tests;
