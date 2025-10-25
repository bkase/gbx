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

/// Trait exposing serial transfer ticking.
pub trait SerialIo {
    /// Advances the serial link by `cycles` CPU ticks.
    fn step_serial(&mut self, cycles: u32);
}

/// Scalar bus implementation backed by in-memory regions.
pub struct BusScalar {
    /// Entire cartridge ROM contents.
    pub rom: Arc<[u8]>,
    /// Currently selected switchable ROM bank (bank 0 fixed).
    pub rom_bank: usize,
    /// Optional bootstrap ROM overlay.
    boot_rom: Option<Arc<[u8]>>,
    /// Tracks whether the bootstrap overlay is active.
    boot_rom_enabled: bool,
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
    /// Remaining cycles before the current serial transfer completes.
    pub serial_counter: u32,
    /// Indicates whether a serial transfer is currently active.
    pub serial_active: bool,
    /// Latched serial byte scheduled for capture when the transfer completes.
    pub serial_pending_data: u8,
    /// Shift register used while clocking the current serial transfer.
    pub serial_shift_reg: u8,
    /// Number of bits remaining in the active transfer.
    pub serial_bits_remaining: u8,
    /// Tracks whether the transfer uses the internal serial clock.
    pub serial_internal_clock: bool,
    /// Optional override for the LY register used by lockstep tests to mirror the oracle.
    pub lockstep_ly_override: Option<u8>,
    /// Latched JOYP column select bits (P14/P15).
    pub joyp_select: u8,
    /// Latched button inputs (A, B, Select, Start), active-low.
    pub joyp_buttons: u8,
    /// Latched d-pad inputs (Right, Left, Up, Down), active-low.
    pub joyp_dpad: u8,
    /// Tracks whether DIV was written since the last timer step.
    pub timer_div_reset: bool,
    /// Captures the most recent TIMA write until timers acknowledge it.
    pub timer_tima_write: Option<u8>,
    /// Captures the most recent TMA write until timers acknowledge it.
    pub timer_tma_write: Option<u8>,
    /// Captures the most recent TAC write (old, new) until timers acknowledge it.
    pub timer_tac_write: Option<(u8, u8)>,
}

impl BusScalar {
    /// Creates a bus using the supplied ROM bytes.
    pub fn new(rom: Arc<[u8]>, boot_rom: Option<Arc<[u8]>>) -> Self {
        let boot_rom_enabled = boot_rom.is_some();
        let mut bus = Self {
            rom,
            rom_bank: 1,
            boot_rom,
            boot_rom_enabled,
            vram: Box::new([0; 0x2000]),
            wram: Box::new([0; 0x2000]),
            oam: Box::new([0; 0xA0]),
            hram: [0; 0x7F],
            io: IoRegs::new(),
            ie: 0,
            serial_out: Vec::new(),
            serial_counter: 0,
            serial_active: false,
            serial_pending_data: 0,
            serial_shift_reg: 0,
            serial_bits_remaining: 0,
            serial_internal_clock: true,
            lockstep_ly_override: None,
            joyp_select: 0x30,
            joyp_buttons: 0x0F,
            joyp_dpad: 0x0F,
            timer_div_reset: false,
            timer_tima_write: None,
            timer_tma_write: None,
            timer_tac_write: None,
        };
        bus.set_rom_bank(1);
        bus
    }

    /// Replaces the ROM contents.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.rom = rom;
        self.boot_rom_enabled = self.boot_rom.is_some();
        self.serial_out.clear();
        self.reset_serial_state();
        self.joyp_select = 0x30;
        self.joyp_buttons = 0x0F;
        self.joyp_dpad = 0x0F;
        self.timer_div_reset = false;
        self.timer_tima_write = None;
        self.timer_tma_write = None;
        self.timer_tac_write = None;
        self.set_rom_bank(1);
    }

    /// Resets VRAM, WRAM, and IO state to their power-on defaults.
    pub fn reset_memory(&mut self) {
        self.vram.fill(0);
        self.wram.fill(0);
        self.oam.fill(0);
        self.hram = [0; 0x7F];
        self.io = IoRegs::new();
        self.ie = 0;
        self.serial_out.clear();
        self.reset_serial_state();
        self.serial_pending_data = 0;
        self.serial_counter = 0;
        self.serial_bits_remaining = 0;
        self.serial_internal_clock = true;
        self.lockstep_ly_override = None;
        self.joyp_select = 0x30;
        self.joyp_buttons = 0x0F;
        self.joyp_dpad = 0x0F;
        self.timer_div_reset = false;
        self.timer_tima_write = None;
        self.timer_tma_write = None;
        self.timer_tac_write = None;
        self.boot_rom_enabled = self.boot_rom.is_some();
        self.set_rom_bank(1);
    }

    /// Enables or disables the bootstrap overlay.
    pub fn set_boot_rom_enabled(&mut self, enabled: bool) {
        if self.boot_rom.is_some() {
            self.boot_rom_enabled = enabled;
        }
    }

    /// Returns whether a bootstrap overlay is present.
    pub fn has_boot_rom(&self) -> bool {
        self.boot_rom.is_some()
    }

    /// Returns whether the bootstrap overlay is currently active.
    pub fn boot_rom_enabled(&self) -> bool {
        self.boot_rom_enabled
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

    fn rom_bank_count(&self) -> usize {
        self.rom.len().div_ceil(0x4000)
    }

    pub(crate) fn set_rom_bank(&mut self, bank: usize) {
        let total = self.rom_bank_count().max(1);
        let mut bank = bank % total;
        if total > 1 && bank == 0 {
            bank = 1;
        }
        self.rom_bank = bank;
    }

    pub(crate) fn boot_rom_byte(&self, addr: u16) -> Option<u8> {
        if !self.boot_rom_enabled {
            return None;
        }
        self.boot_rom
            .as_ref()
            .and_then(|boot| boot.get(addr as usize).copied())
    }

    pub(crate) fn disable_boot_rom(&mut self) {
        self.boot_rom_enabled = false;
    }

    const SERIAL_CYCLES_PER_BIT: u32 = 512;
    /// Accounts for the CPU cycles that elapse between the write to SC and the moment the link
    /// hardware starts driving the internal clock. Adjusted to match DMG timing (verified against
    /// SameBoy) so the serial interrupt fires on the same boundary.
    const SERIAL_STARTUP_LATENCY: u32 = 0;
    const SERIAL_FIRST_BIT_CYCLES: u32 = Self::SERIAL_CYCLES_PER_BIT - Self::SERIAL_STARTUP_LATENCY;

    fn reset_serial_state(&mut self) {
        self.serial_active = false;
        self.serial_counter = 0;
        self.serial_bits_remaining = 0;
    }

    pub(crate) fn write_serial_control(&mut self, value: u8) {
        let prev_raw = self.io.sc_raw();
        let raw = value & 0x83;
        self.io.write(Self::io_sc_index(), raw);

        let start_requested = (raw & 0x80) != 0;
        if !start_requested {
            self.reset_serial_state();
            return;
        }

        let started_now = (prev_raw & 0x80) == 0 || !self.serial_active;
        self.serial_internal_clock = (raw & 0x01) != 0;

        if !started_now && self.serial_active {
            return;
        }

        self.reset_serial_state();

        self.serial_pending_data = self.io.read(Self::io_sb_index());
        self.serial_shift_reg = self.serial_pending_data;
        self.serial_bits_remaining = 8;
        self.serial_active = true;

        if self.serial_internal_clock {
            self.serial_counter = Self::SERIAL_FIRST_BIT_CYCLES;
        }
    }

    pub(crate) fn advance_serial(&mut self, mut cycles: u32) {
        if !self.serial_active || !self.serial_internal_clock {
            return;
        }

        while self.serial_active && cycles > 0 {
            if self.serial_counter == 0 {
                self.serial_counter = Self::SERIAL_CYCLES_PER_BIT;
            }

            if cycles >= self.serial_counter {
                cycles -= self.serial_counter;
                self.serial_counter = 0;
                self.clock_serial_bit();
            } else {
                self.serial_counter -= cycles;
                cycles = 0;
            }
        }
    }

    fn clock_serial_bit(&mut self) {
        let incoming = self.serial_input_bit();
        self.serial_shift_reg = (self.serial_shift_reg << 1) | incoming;
        self.serial_bits_remaining = self.serial_bits_remaining.saturating_sub(1);
        self.io.write(Self::io_sb_index(), self.serial_shift_reg);

        if self.serial_bits_remaining == 0 {
            self.finish_serial_transfer();
        }
    }

    fn serial_input_bit(&self) -> u8 {
        0x01
    }

    fn finish_serial_transfer(&mut self) {
        self.reset_serial_state();

        let clock_bit = self.io.sc_raw() & 0x01;
        self.io.write(Self::io_sc_index(), clock_bit);

        let mut if_reg = self.io.if_reg();
        if_reg |= 0x08;
        self.io.set_if(if_reg);

        self.serial_out.push(self.serial_pending_data);
    }

    /// Updates the latched joypad inputs using active-low semantics per Game Boy hardware.
    #[inline]
    pub fn set_inputs(&mut self, joypad: u8) {
        self.joyp_buttons = (joypad >> 4) & 0x0F;
        self.joyp_dpad = joypad & 0x0F;
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

impl SerialIo for BusScalar {
    #[inline]
    fn step_serial(&mut self, cycles: u32) {
        self.advance_serial(cycles);
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
    /// Background palette register offset.
    pub const BGP: usize = 0x47;
    /// Object palette 0 register offset.
    pub const OBP0: usize = 0x48;
    /// Object palette 1 register offset.
    pub const OBP1: usize = 0x49;
    /// Window Y position register offset.
    pub const WY: usize = 0x4A;
    /// Window X position register offset.
    pub const WX: usize = 0x4B;
    /// Speed switch register (CGB only; reads as 0xFF on DMG hardware).
    pub const KEY1: usize = 0x4D;

    /// Creates zeroed IO registers.
    pub fn new() -> Self {
        let mut regs = [0; 0x80];
        regs[Self::IF] = 0xE0;
        regs[Self::KEY1] = 0xFF;
        Self { regs }
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
        match idx {
            Self::SC => self.sc(),
            Self::TAC => self.tac(),
            Self::KEY1 => 0xFF,
            _ => self.regs[idx],
        }
    }

    /// Writes an IO register.
    #[inline]
    pub fn write(&mut self, idx: usize, value: u8) {
        match idx {
            Self::SC => self.set_sc(value),
            Self::KEY1 => {
                // DMG hardware ignores writes; keep the register reading as 0xFF so
                // detection code recognises the model correctly.
                self.regs[idx] = 0xFF;
            }
            _ => {
                self.regs[idx] = value;
            }
        }
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
        self.regs[Self::TAC] | 0xF8
    }

    /// Writes the timer control register.
    #[inline]
    pub fn set_tac(&mut self, value: u8) {
        self.regs[Self::TAC] = value;
    }

    /// Writes the serial control register (FF02).
    #[inline]
    pub fn set_sc(&mut self, value: u8) {
        self.regs[Self::SC] = value & 0x81;
    }

    /// Returns the raw serial control bits (bit7 start, bit0 clock).
    #[inline]
    pub fn sc_raw(&self) -> u8 {
        self.regs[Self::SC] & 0x81
    }

    /// Reads the serial control register with unused bits pulled high.
    #[inline]
    pub fn sc(&self) -> u8 {
        self.sc_raw() | 0x7E
    }

    /// Writes the interrupt flag register.
    #[inline]
    pub fn set_if(&mut self, value: u8) {
        // IF bits 5-7 read back as 1 on hardware; mask writes to the lower
        // interrupt bits so pending flags match Pan Docs semantics.
        self.regs[Self::IF] = (value & 0x1F) | 0xE0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const SERIAL_STEP_CYCLES: u32 = 512;
    const FIRST_BIT_CYCLES: u32 = SERIAL_STEP_CYCLES - BusScalar::SERIAL_STARTUP_LATENCY;

    fn make_bus() -> BusScalar {
        let rom = Arc::<[u8]>::from(vec![0u8; 0x8000]);
        BusScalar::new(rom, None)
    }

    #[test]
    fn serial_internal_transfer_ticks_every_512_cycles() {
        let mut bus = make_bus();
        bus.io.write(BusScalar::io_sb_index(), b'A');
        bus.write_serial_control(0x81);

        assert!(bus.serial_active);
        assert!(bus.serial_internal_clock);
        assert_eq!(bus.serial_bits_remaining, 8);

        bus.advance_serial(FIRST_BIT_CYCLES - 1);
        assert_eq!(bus.serial_bits_remaining, 8);
        assert_eq!(bus.io.read(BusScalar::io_sb_index()), b'A');
        assert_eq!(bus.io.sc_raw() & 0x80, 0x80);

        bus.advance_serial(1);
        assert_eq!(bus.serial_bits_remaining, 7);
        assert_eq!(bus.io.sc_raw() & 0x80, 0x80);

        for _ in 0..6 {
            bus.advance_serial(SERIAL_STEP_CYCLES);
        }
        assert_eq!(bus.serial_bits_remaining, 1);

        bus.advance_serial(SERIAL_STEP_CYCLES);

        assert!(!bus.serial_active);
        assert_eq!(bus.io.sc_raw() & 0x80, 0);
        assert_ne!(bus.io.if_reg() & 0x08, 0);
        assert_eq!(bus.io.read(BusScalar::io_sb_index()), 0xFF);
        assert_eq!(bus.take_serial(), "A");
    }

    #[test]
    fn serial_external_clock_waits_for_remote_partner() {
        let mut bus = make_bus();
        bus.io.write(BusScalar::io_sb_index(), b'B');
        bus.write_serial_control(0x80);

        assert!(bus.serial_active);
        assert!(!bus.serial_internal_clock);
        assert_eq!(bus.serial_bits_remaining, 8);

        bus.advance_serial(SERIAL_STEP_CYCLES * 8);

        assert!(bus.serial_active);
        assert_eq!(bus.serial_bits_remaining, 8);
        assert_eq!(bus.io.sc_raw() & 0x80, 0x80);
        assert_eq!(bus.io.if_reg() & 0x08, 0);
        assert!(bus.serial_out.is_empty());
        assert_eq!(bus.io.read(BusScalar::io_sb_index()), b'B');
    }

    #[test]
    fn serial_retrigger_during_active_transfer_is_ignored() {
        let mut bus = make_bus();
        bus.io.write(BusScalar::io_sb_index(), b'C');
        bus.write_serial_control(0x81);
        bus.advance_serial(SERIAL_STEP_CYCLES);

        assert_eq!(bus.serial_bits_remaining, 7);

        bus.write_serial_control(0x81);

        assert_eq!(bus.serial_bits_remaining, 7);
        assert!(bus.serial_active);
    }
}
