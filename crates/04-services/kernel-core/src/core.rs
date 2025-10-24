use crate::bus::{Bus, BusScalar, InterruptCtrl, SerialIo};
use crate::bus_simd::BusSimd;
use crate::cpu::Cpu;
use crate::exec::{Exec, Scalar};
use crate::exec_simd::SimdExec;
use crate::instr::{self, AluOp};
use crate::ppu_stub::{PpuFrameSource, PpuIo, PpuStub};
use crate::timers::{TimerIo, Timers};

/// Bundles the bus traits required by the scalar core for timing-sensitive peripherals.
pub trait CoreBus<E: Exec>: Bus<E> + TimerIo + InterruptCtrl + PpuIo + SerialIo {}
impl<E: Exec, B> CoreBus<E> for B where B: Bus<E> + TimerIo + InterruptCtrl + PpuIo + SerialIo {}
use core::simd::{LaneCount, SupportedLaneCount};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Hardware model variants supported by the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// Original DMG model.
    Dmg,
}

/// Core configuration shared across backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreConfig {
    /// Width of the output frame in pixels.
    pub frame_width: u16,
    /// Height of the output frame in pixels.
    pub frame_height: u16,
    /// Number of SIMD lanes the core should drive.
    pub lanes: NonZeroUsize,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            frame_width: 160,
            frame_height: 144,
            lanes: NonZeroUsize::new(1).expect("non-zero"),
        }
    }
}

/// Combined CPU + bus + timing core parameterised by execution backend.
pub struct Core<E: Exec, B: CoreBus<E>> {
    /// CPU register file and scheduler state.
    pub cpu: Cpu<E>,
    /// Memory and IO bus implementation.
    pub bus: B,
    /// Timer block shared with the scheduler.
    pub timers: Timers,
    /// Stub PPU used for frame pacing.
    pub ppu: PpuStub,
    /// Cycle budget accumulated within the current frame.
    pub cycles_this_frame: u32,
    inline_cycle_credit: u32,
    ppu_enabled: bool,
    config: CoreConfig,
    model: Model,
}

impl<E: Exec, B: CoreBus<E>> Core<E, B> {
    /// Creates a core with the supplied bus and configuration.
    pub fn new(bus: B, config: CoreConfig, model: Model) -> Self {
        Self {
            cpu: Cpu::new(),
            bus,
            timers: Timers::new(),
            ppu: PpuStub::new(),
            cycles_this_frame: 0,
            inline_cycle_credit: 0,
            ppu_enabled: true,
            config,
            model,
        }
    }

    /// Resets CPU and peripheral state to post-boot defaults.
    pub fn reset_post_boot(&mut self, model: Model)
    where
        B: TimerIo,
    {
        self.model = model;
        self.cpu = Cpu::new();
        self.timers.reset();
        self.ppu.reset();
        self.cycles_this_frame = 0;
        self.inline_cycle_credit = 0;
        self.ppu_enabled = true;
        self.cpu.halted = false;
        self.cpu.enable_ime_pending = false;
        match model {
            Model::Dmg => {
                // Classic DMG register defaults per Pan Docs reference.
                self.cpu.a = E::from_u8(0x01);
                self.cpu.f.from_byte(0xB0);
                self.cpu.b = E::from_u8(0x00);
                self.cpu.c = E::from_u8(0x13);
                self.cpu.d = E::from_u8(0x00);
                self.cpu.e = E::from_u8(0xD8);
                self.cpu.h = E::from_u8(0x01);
                self.cpu.l = E::from_u8(0x4D);
                self.cpu.sp = E::from_u16(0xFFFE);
                self.cpu.pc = E::from_u16(0x0100);
                self.cpu.ime = true;

                // PPU post-boot defaults so the LCD begins in the enabled state.
                self.bus.write8(E::from_u16(0xFF40), E::from_u8(0x91)); // LCDC
                self.bus.write8(E::from_u16(0xFF41), E::from_u8(0x85)); // STAT
                self.bus.write8(E::from_u16(0xFF42), E::from_u8(0x00)); // SCY
                self.bus.write8(E::from_u16(0xFF43), E::from_u8(0x00)); // SCX
                self.bus.write8(E::from_u16(0xFF44), E::from_u8(0x00)); // LY
                self.bus.write8(E::from_u16(0xFF47), E::from_u8(0xFC)); // BGP
                self.timers.initialize_post_boot(&mut self.bus);
            }
        }
    }

    fn consume_cycles(&mut self, cycles: u32)
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        if cycles == 0 {
            return;
        }
        self.timers.step(cycles, &mut self.bus);
        if self.ppu_enabled {
            self.ppu.step(cycles, &mut self.bus);
        }
        self.bus.step_serial(cycles);
        self.cycles_this_frame = self.cycles_this_frame.wrapping_add(cycles);
    }

    pub(crate) fn fetch_imm8(&mut self) -> E::U8
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        let byte = self.cpu.fetch8(&mut self.bus);
        self.consume_cycles(4);
        self.inline_cycle_credit = self.inline_cycle_credit.wrapping_add(4);
        byte
    }

    pub(crate) fn fetch_imm16(&mut self) -> E::U16
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        let lo = self.fetch_imm8();
        let hi = self.fetch_imm8();
        E::combine_u16(hi, lo)
    }

    pub(crate) fn read8(&mut self, addr: E::U16) -> E::U8
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        let byte = self.bus.read8(addr);
        self.consume_cycles(4);
        self.inline_cycle_credit = self.inline_cycle_credit.wrapping_add(4);
        byte
    }

    pub(crate) fn write8(&mut self, addr: E::U16, value: E::U8)
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        self.bus.write8(addr, value);
        self.consume_cycles(4);
        self.inline_cycle_credit = self.inline_cycle_credit.wrapping_add(4);
    }

    /// Returns whether the PPU reported a frame boundary.
    #[inline]
    pub fn frame_ready(&self) -> bool {
        self.ppu.frame_ready()
    }

    /// Enables or disables PPU stepping.
    #[inline]
    pub fn set_ppu_enabled(&mut self, enabled: bool) {
        self.ppu_enabled = enabled;
    }

    /// Renders the current frame into `out_rgba`.
    ///
    /// The stub implementation fills a simple gradient to surface determinism.
    pub fn take_frame(&mut self, out_rgba: &mut [u8])
    where
        B: PpuFrameSource,
    {
        let width = usize::from(self.config.frame_width);
        let height = usize::from(self.config.frame_height);
        let expected_len = width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .expect("frame dimensions should not overflow");
        assert!(
            out_rgba.len() >= expected_len,
            "frame buffer too small (have {}, need {})",
            out_rgba.len(),
            expected_len
        );

        self.ppu.render_frame(
            self.bus.ppu_io(),
            self.bus.ppu_vram(),
            self.bus.ppu_oam(),
            out_rgba,
            self.config.frame_width,
            self.config.frame_height,
        );

        self.ppu.clear_frame_ready();
        self.cycles_this_frame = 0;
    }

    /// Executes instructions until the cycle budget is exhausted or a frame boundary is hit.
    pub fn step_cycles(&mut self, mut budget: u32) -> u32
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        if budget == 0 {
            return 0;
        }

        let mut consumed = 0u32;
        while budget > 0 {
            let frame_before = self.cycles_this_frame;

            if self.service_interrupts() == 0 {
                if self.cpu.halted {
                    let step = budget.min(4);
                    self.consume_cycles(step);
                } else {
                    self.execute_opcode();
                }
            }

            let delta = self.cycles_this_frame.wrapping_sub(frame_before);
            if delta == 0 {
                break;
            }

            consumed = consumed.wrapping_add(delta);
            budget = budget.saturating_sub(delta);

            if self.ppu.frame_ready() {
                break;
            }
        }

        consumed
    }

    /// Executes exactly one instruction or interrupt service and returns cycles + new PC.
    pub fn step_instruction(&mut self) -> (u32, u16)
    where
        B: TimerIo + InterruptCtrl + PpuIo + SerialIo,
    {
        let prev_pc = E::to_u16(self.cpu.pc);
        let frame_before = self.cycles_this_frame;

        if self.service_interrupts() == 0 {
            if self.cpu.halted {
                self.consume_cycles(4);
            } else {
                self.execute_opcode();
            }
        }

        let cycles = self.cycles_this_frame.wrapping_sub(frame_before);
        let pc = if cycles == 0 {
            prev_pc
        } else {
            E::to_u16(self.cpu.pc)
        };
        (cycles, pc)
    }

    fn execute_opcode(&mut self) -> u32
    where
        B: TimerIo + InterruptCtrl,
    {
        self.inline_cycle_credit = 0;
        let pending_before = self.cpu.enable_ime_pending;
        let opcode = self.cpu.fetch8(&mut self.bus);
        let opcode_u8 = E::to_u8(opcode);
        self.consume_cycles(4);
        self.inline_cycle_credit = self.inline_cycle_credit.wrapping_add(4);
        let was_ei = opcode_u8 == 0xFB;
        let cycles = match opcode_u8 {
            0x00 => instr::op_nop(),
            0x01 => instr::op_ld_rr_d16(self, 0),
            0x02 => instr::op_ld_mem_rr_a(self, 0),
            0x03 => instr::op_inc_rr(self, 0),
            0x04 => instr::op_inc_r(self, 0),
            0x05 => instr::op_dec_r(self, 0),
            0x06 => instr::op_ld_r_d8(self, 0),
            0x07 => instr::op_rlca(self),
            0x08 => instr::op_ld_mem_a16_sp(self),
            0x09 => instr::op_add_hl_rr(self, 0),
            0x0A => instr::op_ld_a_mem_rr(self, 0),
            0x0B => instr::op_dec_rr(self, 0),
            0x0C => instr::op_inc_r(self, 1),
            0x0D => instr::op_dec_r(self, 1),
            0x0E => instr::op_ld_r_d8(self, 1),
            0x0F => instr::op_rrca(self),
            0x10 => instr::op_stop(self),
            0x11 => instr::op_ld_rr_d16(self, 1),
            0x12 => instr::op_ld_mem_rr_a(self, 1),
            0x13 => instr::op_inc_rr(self, 1),
            0x14 => instr::op_inc_r(self, 2),
            0x15 => instr::op_dec_r(self, 2),
            0x16 => instr::op_ld_r_d8(self, 2),
            0x17 => instr::op_rla(self),
            0x18 => instr::op_jr(self),
            0x19 => instr::op_add_hl_rr(self, 1),
            0x1A => instr::op_ld_a_mem_rr(self, 1),
            0x1B => instr::op_dec_rr(self, 1),
            0x1C => instr::op_inc_r(self, 3),
            0x1D => instr::op_dec_r(self, 3),
            0x1E => instr::op_ld_r_d8(self, 3),
            0x1F => instr::op_rra(self),
            0x20 | 0x28 | 0x30 | 0x38 => instr::op_jr_cc(self, opcode_u8),
            0x21 => instr::op_ld_rr_d16(self, 2),
            0x22 => instr::op_ldi_hl_a(self),
            0x23 => instr::op_inc_rr(self, 2),
            0x24 => instr::op_inc_r(self, 4),
            0x25 => instr::op_dec_r(self, 4),
            0x26 => instr::op_ld_r_d8(self, 4),
            0x27 => instr::op_daa(self),
            0x29 => instr::op_add_hl_rr(self, 2),
            0x2A => instr::op_ldi_a_hl(self),
            0x2B => instr::op_dec_rr(self, 2),
            0x2C => instr::op_inc_r(self, 5),
            0x2D => instr::op_dec_r(self, 5),
            0x2E => instr::op_ld_r_d8(self, 5),
            0x2F => instr::op_cpl(self),
            0x31 => instr::op_ld_rr_d16(self, 3),
            0x32 => instr::op_ldd_hl_a(self),
            0x33 => instr::op_inc_rr(self, 3),
            0x34 => instr::op_inc_r(self, 6),
            0x35 => instr::op_dec_r(self, 6),
            0x36 => instr::op_ld_hl_d8(self),
            0x37 => instr::op_scf(self),
            0x39 => instr::op_add_hl_rr(self, 3),
            0x3A => instr::op_ldd_a_hl(self),
            0x3B => instr::op_dec_rr(self, 3),
            0x3C => instr::op_inc_r(self, 7),
            0x3D => instr::op_dec_r(self, 7),
            0x3E => instr::op_ld_r_d8(self, 7),
            0x3F => instr::op_ccf(self),
            0x40..=0x7F => {
                if opcode_u8 == 0x76 {
                    instr::op_halt(self)
                } else {
                    instr::op_ld_r_r(self, opcode_u8)
                }
            }
            0x80..=0x87 => instr::op_alu_a_r(self, opcode_u8, AluOp::Add),
            0x88..=0x8F => instr::op_alu_a_r(self, opcode_u8, AluOp::Adc),
            0x90..=0x97 => instr::op_alu_a_r(self, opcode_u8, AluOp::Sub),
            0x98..=0x9F => instr::op_alu_a_r(self, opcode_u8, AluOp::Sbc),
            0xA0..=0xA7 => instr::op_alu_a_r(self, opcode_u8, AluOp::And),
            0xA8..=0xAF => instr::op_alu_a_r(self, opcode_u8, AluOp::Xor),
            0xB0..=0xB7 => instr::op_alu_a_r(self, opcode_u8, AluOp::Or),
            0xB8..=0xBF => instr::op_alu_a_r(self, opcode_u8, AluOp::Cp),
            0xC0 | 0xC8 | 0xD0 | 0xD8 => instr::op_ret_cc(self, opcode_u8),
            0xC1 => instr::op_pop_rr(self, 0),
            0xC2 | 0xCA | 0xD2 | 0xDA => instr::op_jp_cc_a16(self, opcode_u8),
            0xC3 => instr::op_jp_a16(self),
            0xC4 | 0xCC | 0xD4 | 0xDC => instr::op_call_cc_a16(self, opcode_u8),
            0xC5 => instr::op_push_rr(self, 0),
            0xC6 => instr::op_alu_a_d8(self, AluOp::Add),
            0xC9 => instr::op_ret(self),
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => instr::op_rst(self, opcode_u8),
            0xCB => {
                let sub = self.fetch_imm8();
                instr::op_cb(self, E::to_u8(sub))
            }
            0xCD => instr::op_call_a16(self),
            0xCE => instr::op_alu_a_d8(self, AluOp::Adc),
            0xD1 => instr::op_pop_rr(self, 1),
            0xD9 => instr::op_reti(self),
            0xD5 => instr::op_push_rr(self, 1),
            0xD6 => instr::op_alu_a_d8(self, AluOp::Sub),
            0xDE => instr::op_alu_a_d8(self, AluOp::Sbc),
            0xE0 => instr::op_ldh_a8_a(self),
            0xE1 => instr::op_pop_rr(self, 2),
            0xE2 => instr::op_ldh_c_a(self),
            0xE5 => instr::op_push_rr(self, 2),
            0xE6 => instr::op_alu_a_d8(self, AluOp::And),
            0xE8 => instr::op_add_sp_e8(self),
            0xE9 => instr::op_jp_hl(self),
            0xEA => instr::op_ld_a16_a(self),
            0xEE => instr::op_alu_a_d8(self, AluOp::Xor),
            0xF0 => instr::op_ldh_a_a8(self),
            0xF1 => instr::op_pop_rr(self, 3),
            0xF2 => instr::op_ldh_a_c(self),
            0xF3 => instr::op_di(self),
            0xF5 => instr::op_push_rr(self, 3),
            0xF6 => instr::op_alu_a_d8(self, AluOp::Or),
            0xF8 => instr::op_ld_hl_sp_plus_e8(self),
            0xF9 => instr::op_ld_sp_hl(self),
            0xFA => instr::op_ld_a_a16(self),
            0xFB => instr::op_ei(self),
            0xFE => instr::op_alu_a_d8(self, AluOp::Cp),
            _ => instr::op_unimplemented(),
        };
        if pending_before {
            self.cpu.ime = true;
            self.cpu.enable_ime_pending = was_ei;
        }
        let accounted = self.inline_cycle_credit;
        if cycles > accounted {
            self.consume_cycles(cycles - accounted);
        }
        self.inline_cycle_credit = 0;
        cycles
    }

    pub(crate) fn inc_reg(&mut self, value: E::U8) -> (E::U8, u32) {
        let carry = self.cpu.f.c();
        let val = E::to_u8(value);
        let res = val.wrapping_add(1);
        let result = E::from_u8(res);

        self.cpu.f.set_z(res == 0);
        self.cpu.f.set_n(false);
        self.cpu.f.set_h(((val ^ res) & 0x10) != 0);
        self.cpu.f.set_c(carry);

        (result, 4)
    }

    pub(crate) fn dec_reg(&mut self, value: E::U8) -> (E::U8, u32) {
        let carry = self.cpu.f.c();
        let val = E::to_u8(value);
        let res = val.wrapping_sub(1);
        let result = E::from_u8(res);

        self.cpu.f.set_z(res == 0);
        self.cpu.f.set_n(true);
        self.cpu.f.set_h(((val ^ res) & 0x10) != 0);
        self.cpu.f.set_c(carry);

        (result, 4)
    }

    pub(crate) fn jump_relative(&mut self, offset: E::U8, condition: bool) {
        if condition {
            let pc = E::to_u16(self.cpu.pc);
            let delta = E::to_u8(offset) as i8;
            let next = pc.wrapping_add(delta as i16 as u16);
            self.cpu.pc = E::from_u16(next);
        }
    }

    pub(crate) fn add16_hl(&mut self, rhs: E::U16) -> u32 {
        let lhs = E::to_u16(self.cpu.hl());
        let rhs = E::to_u16(rhs);
        let result = lhs.wrapping_add(rhs);
        self.cpu.set_hl(E::from_u16(result));
        self.cpu.f.set_n(false);
        self.cpu.f.set_h(((lhs & 0x0FFF) + (rhs & 0x0FFF)) > 0x0FFF);
        self.cpu.f.set_c((lhs as u32 + rhs as u32) > 0xFFFF);
        8
    }

    pub(crate) fn inc16(&self, value: E::U16) -> (E::U16, u32) {
        let result = E::from_u16(E::to_u16(value).wrapping_add(1));
        (result, 8)
    }

    pub(crate) fn dec16(&self, value: E::U16) -> (E::U16, u32) {
        let result = E::from_u16(E::to_u16(value).wrapping_sub(1));
        (result, 8)
    }

    pub(crate) fn add_sp_e8(&self, offset: E::U8) -> (E::U16, bool, bool) {
        let sp = E::to_u16(self.cpu.sp);
        let off = E::to_u8(offset) as i8 as i16;
        let result = sp.wrapping_add(off as u16);
        let offset_u16 = off as u16;
        let half = ((sp ^ offset_u16 ^ result) & 0x0010) != 0;
        let carry = ((sp ^ offset_u16 ^ result) & 0x0100) != 0;
        (E::from_u16(result), half, carry)
    }

    fn service_interrupts(&mut self) -> u32
    where
        B: TimerIo + InterruptCtrl,
    {
        let pending = self.bus.read_ie() & TimerIo::read_if(&self.bus);
        if pending == 0 {
            return 0;
        }

        if self.cpu.ime {
            self.cpu.halted = false;
            self.cpu.ime = false;
            let bit = pending.trailing_zeros() as usize;
            let mask = 1u8 << bit;
            let mut if_reg = TimerIo::read_if(&self.bus);
            if_reg &= !mask;
            TimerIo::write_if(&mut self.bus, if_reg);
            let pc = self.cpu.pc;
            self.cpu.push16(&mut self.bus, pc);
            const VECTORS: [u16; 5] = [0x40, 0x48, 0x50, 0x58, 0x60];
            self.cpu.pc = E::from_u16(VECTORS[bit]);
            self.consume_cycles(20);
            20
        } else {
            if self.cpu.halted {
                self.cpu.halted = false;
            }
            0
        }
    }
}

impl Core<Scalar, BusScalar> {
    /// Convenience constructor using the scalar bus.
    pub fn from_rom(rom: Arc<[u8]>) -> Self {
        let bus = BusScalar::new(rom);
        Self::new(bus, CoreConfig::default(), Model::Dmg)
    }

    /// Replaces the cartridge ROM.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.bus.load_rom(rom);
        self.cpu.pc = <Scalar as Exec>::from_u16(0x0100);
    }
}

impl<const LANES: usize> Core<SimdExec<LANES>, BusSimd<LANES>>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    /// Convenience constructor using the SIMD bus.
    pub fn from_rom(rom: Arc<[u8]>) -> Self {
        let config = CoreConfig {
            lanes: NonZeroUsize::new(LANES).expect("SIMD lanes must be non-zero"),
            ..CoreConfig::default()
        };
        let bus = BusSimd::new(rom);
        Self::new(bus, config, Model::Dmg)
    }

    /// Replaces the cartridge ROM for every SIMD lane.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.bus.load_rom(rom);
        self.cpu.pc = <SimdExec<LANES> as Exec>::from_u16(0x0100);
    }
}
