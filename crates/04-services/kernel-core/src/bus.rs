use crate::exec::{Exec, Scalar};
use crate::mmu;
use std::sync::Arc;

/// Memory and IO bus abstraction.
pub trait Bus<E: Exec> {
    /// Reads an 8-bit value from the provided address.
    fn read8(&mut self, addr: E::U16) -> E::U8;
    /// Writes an 8-bit value to the provided address.
    fn write8(&mut self, addr: E::U16, value: E::U8);
}

/// Scalar bus implementation backed by in-memory regions.
pub struct BusScalar {
    pub rom: Arc<[u8]>,
    pub vram: Box<[u8; 0x2000]>,
    pub wram: Box<[u8; 0x2000]>,
    pub oam: Box<[u8; 0xA0]>,
    pub hram: [u8; 0x7F],
    pub io: IoRegs,
    pub ie: u8,
}

impl BusScalar {
    /// Creates a bus using the supplied ROM bytes.
    pub fn new(rom: Arc<[u8]>) -> Self {
        Self {
            rom,
            vram: Box::new([0; 0x2000]),
            wram: Box::new([0; 0x2000]),
            oam: Box::new([0; 0xA0]),
            hram: [0; 0x7F],
            io: IoRegs::new(),
            ie: 0,
        }
    }

    /// Replaces the ROM contents.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.rom = rom;
    }
}

impl Bus<Scalar> for BusScalar {
    #[inline]
    fn read8(&mut self, addr: <Scalar as Exec>::U16) -> <Scalar as Exec>::U8 {
        mmu::read8_scalar(self, addr)
    }

    #[inline]
    fn write8(&mut self, addr: <Scalar as Exec>::U16, value: <Scalar as Exec>::U8) {
        mmu::write8_scalar(self, addr, value);
    }
}

/// IO register block for the scalar bus.
#[derive(Clone)]
pub struct IoRegs {
    regs: [u8; 0x80],
}

impl IoRegs {
    pub const JOYP: usize = 0x00;
    pub const DIV: usize = 0x04;
    pub const TIMA: usize = 0x05;
    pub const TMA: usize = 0x06;
    pub const TAC: usize = 0x07;
    pub const IF: usize = 0x0F;
    pub const LCDC: usize = 0x40;
    pub const STAT: usize = 0x41;
    pub const SCY: usize = 0x42;
    pub const SCX: usize = 0x43;
    pub const LY: usize = 0x44;
    pub const LYC: usize = 0x45;

    /// Creates zeroed IO registers.
    pub fn new() -> Self {
        Self { regs: [0; 0x80] }
    }

    /// Returns a mutable reference to the backing array.
    pub fn regs_mut(&mut self) -> &mut [u8; 0x80] {
        &mut self.regs
    }

    /// Returns an immutable reference to the array.
    pub fn regs(&self) -> &[u8; 0x80] {
        &self.regs
    }

    /// Reads an IO register.
    #[inline]
    pub fn read(&self, idx: usize) -> u8 {
        self.regs[idx]
    }

    /// Writes an IO register.
    #[inline]
    pub fn write(&mut self, idx: usize, value: u8) {
        self.regs[idx] = value;
    }

    /// Updates the divider register.
    #[inline]
    pub fn set_div(&mut self, value: u8) {
        self.regs[Self::DIV] = value;
    }

    /// Reads the divider register.
    #[inline]
    pub fn div(&self) -> u8 {
        self.regs[Self::DIV]
    }

    #[inline]
    pub fn tima(&self) -> u8 {
        self.regs[Self::TIMA]
    }

    #[inline]
    pub fn set_tima(&mut self, value: u8) {
        self.regs[Self::TIMA] = value;
    }

    #[inline]
    pub fn tma(&self) -> u8 {
        self.regs[Self::TMA]
    }

    #[inline]
    pub fn set_tma(&mut self, value: u8) {
        self.regs[Self::TMA] = value;
    }

    #[inline]
    pub fn tac(&self) -> u8 {
        self.regs[Self::TAC]
    }

    #[inline]
    pub fn set_tac(&mut self, value: u8) {
        self.regs[Self::TAC] = value;
    }

    #[inline]
    pub fn set_if(&mut self, value: u8) {
        self.regs[Self::IF] = value;
    }

    #[inline]
    pub fn if_reg(&self) -> u8 {
        self.regs[Self::IF]
    }

    #[inline]
    pub fn set_joyp(&mut self, value: u8) {
        self.regs[Self::JOYP] = value;
    }

    #[inline]
    pub fn joyp(&self) -> u8 {
        self.regs[Self::JOYP]
    }
}
