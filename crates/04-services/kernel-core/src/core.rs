use crate::bus::{Bus, BusScalar};
use crate::cpu::Cpu;
use crate::exec::{Exec, Scalar};
use crate::instr;
use crate::ppu_stub::PpuStub;
use crate::timers::{TimerIo, Timers};
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
    pub frame_width: u16,
    pub frame_height: u16,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            frame_width: 160,
            frame_height: 144,
        }
    }
}

/// Combined CPU + bus + timing core parameterised by execution backend.
pub struct Core<E: Exec, B: Bus<E>> {
    pub cpu: Cpu<E>,
    pub bus: B,
    pub timers: Timers,
    pub ppu: PpuStub,
    pub cycles_this_frame: u32,
    config: CoreConfig,
    model: Model,
}

impl<E: Exec, B: Bus<E>> Core<E, B> {
    /// Creates a core with the supplied bus and configuration.
    pub fn new(bus: B, config: CoreConfig, model: Model) -> Self {
        Self {
            cpu: Cpu::new(),
            bus,
            timers: Timers::new(),
            ppu: PpuStub::new(),
            cycles_this_frame: 0,
            config,
            model,
        }
    }

    /// Resets CPU and peripheral state to post-boot defaults.
    pub fn reset_post_boot(&mut self, model: Model) {
        self.model = model;
        self.cpu = Cpu::new();
        self.timers.reset();
        self.ppu.reset();
        self.cycles_this_frame = 0;
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
            }
        }
    }

    /// Returns whether the PPU reported a frame boundary.
    #[inline]
    pub fn frame_ready(&self) -> bool {
        self.ppu.frame_ready()
    }

    /// Renders the current frame into `out_rgba`.
    ///
    /// The stub implementation fills a simple gradient to surface determinism.
    pub fn take_frame(&mut self, out_rgba: &mut [u8]) {
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

        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                let shade = (((x + y) & 0xFF) as u8).saturating_add(32);
                out_rgba[idx] = shade;
                out_rgba[idx + 1] = shade;
                out_rgba[idx + 2] = shade;
                out_rgba[idx + 3] = 0xFF;
            }
        }

        self.ppu.clear_frame_ready();
        self.cycles_this_frame = 0;
    }

    /// Executes instructions until the cycle budget is exhausted or a frame boundary is hit.
    pub fn step_cycles(&mut self, mut budget: u32) -> u32
    where
        B: TimerIo,
    {
        if budget == 0 {
            return 0;
        }

        let mut consumed = 0u32;
        while budget > 0 {
            let cycles = if self.cpu.halted {
                4u32.min(budget)
            } else {
                self.execute_opcode()
            };

            let cycles = cycles.min(budget);
            self.timers.step(cycles, &mut self.bus);
            self.ppu.step(cycles);
            self.cycles_this_frame = self.cycles_this_frame.wrapping_add(cycles);

            consumed = consumed.wrapping_add(cycles);
            budget = budget.saturating_sub(cycles);

            if self.ppu.frame_ready() {
                break;
            }
        }

        consumed
    }

    fn execute_opcode(&mut self) -> u32 {
        let opcode = self.cpu.fetch8(&mut self.bus);
        let opcode_u8 = E::to_u8(opcode);
        match opcode_u8 {
            0x00 => instr::op_nop(),
            0x01 => instr::op_ld_bc_d16(self),
            0x02 => instr::op_ld_bc_a(self),
            0x03 => instr::op_inc_bc(self),
            0x04 => instr::op_inc_b(self),
            0x05 => instr::op_dec_b(self),
            0x06 => instr::op_ld_b_d8(self),
            0x0E => instr::op_ld_c_d8(self),
            0x11 => instr::op_ld_de_d16(self),
            0x13 => instr::op_inc_de(self),
            0x18 => instr::op_jr(self),
            0x1A => instr::op_ld_a_de(self),
            0x1E => instr::op_ld_e_d8(self),
            0x20 => instr::op_jr_nz(self),
            0x21 => instr::op_ld_hl_d16(self),
            0x22 => instr::op_ldi_hl_a(self),
            0x23 => instr::op_inc_hl(self),
            0x24 => instr::op_inc_h(self),
            0x26 => instr::op_ld_h_d8(self),
            0x2E => instr::op_ld_l_d8(self),
            0x31 => instr::op_ld_sp_d16(self),
            0x32 => instr::op_ldd_hl_a(self),
            0x3E => instr::op_ld_a_d8(self),
            0x76 => instr::op_halt(self),
            0x77 => instr::op_ld_hl_a(self),
            0x80..=0x87 => instr::op_add_a_reg(self, opcode_u8 & 0x07),
            0xAF => instr::op_xor_a(self),
            0xC3 => instr::op_jp_a16(self),
            0xC9 => instr::op_ret(self),
            0xCD => instr::op_call_a16(self),
            0xE0 => instr::op_ldh_a8_a(self),
            0xEA => instr::op_ld_a16_a(self),
            0xF0 => instr::op_ldh_a_a8(self),
            _ => instr::op_unimplemented(),
        }
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
