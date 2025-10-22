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

/// Trait exposing interrupt enable state.
pub trait InterruptCtrl {
    /// Returns the interrupt enable register.
    fn read_ie(&self) -> u8;
}

/// Scalar bus implementation backed by in-memory regions.
pub struct BusScalar {
    /// Entire cartridge ROM contents.
    pub rom: Arc<[u8]>,
    /// Video RAM region used by the PPU.
    pub vram: Box<[u8; 0x2000]>,
    /// Work RAM shared by the CPU.
    pub wram: Box<[u8; 0x2000]>,
    /// Object attribute memory backing sprites.
    pub oam: Box<[u8; 0xA0]>,
    /// High RAM window at `0xFF80`.
    pub hram: [u8; 0x7F],
    /// IO register block.
    pub io: IoRegs,
    /// Interrupt enable register at `0xFFFF`.
    pub ie: u8,
    /// Captured serial output emitted through the serial control register.
    pub serial_out: Vec<u8>,
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
            serial_out: Vec::new(),
        }
    }

    /// Replaces the ROM contents.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.rom = rom;
        self.serial_out.clear();
    }

    /// Returns the accumulated serial log as a string and clears the buffer.
    pub fn take_serial(&mut self) -> String {
        let bytes = std::mem::take(&mut self.serial_out);
        match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(err) => {
                let bytes = err.into_bytes();
                String::from_utf8_lossy(&bytes).into_owned()
            }
        }
    }

    /// Returns the IO register index for the serial data register.
    #[inline]
    pub(crate) fn io_sb_index() -> usize {
        IoRegs::SB
    }

    /// Returns the IO register index for the serial control register.
    #[inline]
    pub(crate) fn io_sc_index() -> usize {
        IoRegs::SC
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

impl InterruptCtrl for BusScalar {
    #[inline]
    fn read_ie(&self) -> u8 {
        self.ie
    }
}

/// IO register block for the scalar bus.
#[derive(Clone)]
pub struct IoRegs {
    regs: [u8; 0x80],
}

impl Default for IoRegs {
    fn default() -> Self {
        Self::new()
    }
}

impl IoRegs {
    /// JOYP register offset.
    pub const JOYP: usize = 0x00;
    /// Serial transfer data register.
    pub const SB: usize = 0x01;
    /// Serial transfer control register.
    pub const SC: usize = 0x02;
    /// Divider register offset.
    pub const DIV: usize = 0x04;
    /// Timer counter register offset.
    pub const TIMA: usize = 0x05;
    /// Timer modulo register offset.
    pub const TMA: usize = 0x06;
    /// Timer control register offset.
    pub const TAC: usize = 0x07;
    /// Interrupt flag register offset.
    pub const IF: usize = 0x0F;
    /// LCD control register offset.
    pub const LCDC: usize = 0x40;
    /// LCD status register offset.
    pub const STAT: usize = 0x41;
    /// Scroll Y register offset.
    pub const SCY: usize = 0x42;
    /// Scroll X register offset.
    pub const SCX: usize = 0x43;
    /// Current scanline register offset.
    pub const LY: usize = 0x44;
    /// Scanline compare register offset.
    pub const LYC: usize = 0x45;
    pub const BGP: usize = 0x47;
    pub const OBP0: usize = 0x48;
    pub const OBP1: usize = 0x49;
    pub const WY: usize = 0x4A;
    pub const WX: usize = 0x4B;

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

    /// Reads the timer counter.
    #[inline]
    pub fn tima(&self) -> u8 {
        self.regs[Self::TIMA]
    }

    /// Writes the timer counter.
    #[inline]
    pub fn set_tima(&mut self, value: u8) {
        self.regs[Self::TIMA] = value;
    }

    /// Reads the timer modulo.
    #[inline]
    pub fn tma(&self) -> u8 {
        self.regs[Self::TMA]
    }

    /// Writes the timer modulo.
    #[inline]
    pub fn set_tma(&mut self, value: u8) {
        self.regs[Self::TMA] = value;
    }

    /// Reads the timer control register.
    #[inline]
    pub fn tac(&self) -> u8 {
        self.regs[Self::TAC]
    }

    /// Writes the timer control register.
    #[inline]
    pub fn set_tac(&mut self, value: u8) {
        self.regs[Self::TAC] = value;
    }

    /// Writes the interrupt flag register.
    #[inline]
    pub fn set_if(&mut self, value: u8) {
        self.regs[Self::IF] = value;
    }

    /// Reads the interrupt flag register.
    #[inline]
    pub fn if_reg(&self) -> u8 {
        self.regs[Self::IF]
    }

    /// Writes the joypad register.
    #[inline]
    pub fn set_joyp(&mut self, value: u8) {
        self.regs[Self::JOYP] = value;
    }

    /// Reads the joypad register.
    #[inline]
    pub fn joyp(&self) -> u8 {
        self.regs[Self::JOYP]
    }
}
