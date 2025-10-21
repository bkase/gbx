//! Instruction metadata helpers.
//!
//! The scalar M1 core drives execution through direct match arms within
//! [`Core::execute_opcode`](crate::core::Core::execute_opcode). This module
//! currently houses minimal helpers but will grow into a table-driven decoder
//! shared by scalar and SIMD backends in follow-up milestones.

/// Execution cost for common instruction classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleCost {
    /// 4-cycle operations.
    Clocks4 = 4,
    /// 8-cycle operations.
    Clocks8 = 8,
    /// 12-cycle operations.
    Clocks12 = 12,
    /// 16-cycle operations.
    Clocks16 = 16,
    /// 24-cycle operations.
    Clocks24 = 24,
}

impl CycleCost {
    /// Returns the cycle count as `u32`.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

use crate::bus::Bus;
use crate::core::Core;
use crate::exec::Exec;

#[inline(always)]
pub fn op_nop() -> u32 {
    CycleCost::Clocks4.as_u32()
}

#[inline(always)]
pub fn op_ld_bc_d16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch16(&mut core.bus);
    core.cpu.set_bc(imm);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_ld_bc_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.bc();
    let value = core.cpu.a;
    core.bus.write8(addr, value);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_inc_bc<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::from_u16(E::to_u16(core.cpu.bc()).wrapping_add(1));
    core.cpu.set_bc(value);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_inc_b<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let (value, cycles) = core.inc_reg(core.cpu.b);
    core.cpu.b = value;
    cycles
}

#[inline(always)]
pub fn op_dec_b<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let (value, cycles) = core.dec_reg(core.cpu.b);
    core.cpu.b = value;
    cycles
}

#[inline(always)]
pub fn op_ld_b_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.b = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_c_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.c = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_de_d16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch16(&mut core.bus);
    core.cpu.set_de(imm);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_inc_de<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::from_u16(E::to_u16(core.cpu.de()).wrapping_add(1));
    core.cpu.set_de(value);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_jr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let off = core.cpu.fetch8(&mut core.bus);
    core.jump_relative(off, true);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_ld_a_de<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.de();
    core.cpu.a = core.bus.read8(addr);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_e_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.e = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_jr_nz<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let off = core.cpu.fetch8(&mut core.bus);
    let cond = !core.cpu.f.z();
    core.jump_relative(off, cond);
    if cond {
        CycleCost::Clocks12.as_u32()
    } else {
        CycleCost::Clocks8.as_u32()
    }
}

#[inline(always)]
pub fn op_ld_hl_d16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch16(&mut core.bus);
    core.cpu.set_hl(imm);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_ldi_hl_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.bus.write8(addr, core.cpu.a);
    let next = E::from_u16(E::to_u16(addr).wrapping_add(1));
    core.cpu.set_hl(next);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_inc_hl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::from_u16(E::to_u16(core.cpu.hl()).wrapping_add(1));
    core.cpu.set_hl(value);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_inc_h<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let (value, cycles) = core.inc_reg(core.cpu.h);
    core.cpu.h = value;
    cycles
}

#[inline(always)]
pub fn op_ld_h_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.h = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_l_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.l = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_sp_d16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch16(&mut core.bus);
    core.cpu.sp = imm;
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_ldd_hl_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.bus.write8(addr, core.cpu.a);
    let next = E::from_u16(E::to_u16(addr).wrapping_sub(1));
    core.cpu.set_hl(next);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_ld_a_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    core.cpu.a = imm;
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_halt<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.halted = true;
    CycleCost::Clocks4.as_u32()
}

#[inline(always)]
pub fn op_ld_hl_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks8.as_u32()
}

#[inline(always)]
pub fn op_add_a_reg<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, reg_idx: u8) -> u32 {
    let (operand, cycles) = match reg_idx {
        0x00 => (core.cpu.b, CycleCost::Clocks4.as_u32()),
        0x01 => (core.cpu.c, CycleCost::Clocks4.as_u32()),
        0x02 => (core.cpu.d, CycleCost::Clocks4.as_u32()),
        0x03 => (core.cpu.e, CycleCost::Clocks4.as_u32()),
        0x04 => (core.cpu.h, CycleCost::Clocks4.as_u32()),
        0x05 => (core.cpu.l, CycleCost::Clocks4.as_u32()),
        0x06 => {
            let addr = core.cpu.hl();
            (core.bus.read8(addr), CycleCost::Clocks8.as_u32())
        }
        0x07 => (core.cpu.a, CycleCost::Clocks4.as_u32()),
        _ => (core.cpu.a, CycleCost::Clocks4.as_u32()),
    };
    let result = E::add8(core.cpu.a, operand, false, &mut core.cpu.f);
    core.cpu.a = result;
    cycles
}

#[inline(always)]
pub fn op_xor_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::xor(core.cpu.a, core.cpu.a);
    core.cpu.a = value;
    core.cpu.f.set_z(true);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(false);
    CycleCost::Clocks4.as_u32()
}

#[inline(always)]
pub fn op_jp_a16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    core.cpu.pc = addr;
    CycleCost::Clocks16.as_u32()
}

#[inline(always)]
pub fn op_ret<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.pop16(&mut core.bus);
    core.cpu.pc = addr;
    CycleCost::Clocks16.as_u32()
}

#[inline(always)]
pub fn op_call_a16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    let ret = core.cpu.pc;
    core.cpu.push16(&mut core.bus, ret);
    core.cpu.pc = addr;
    CycleCost::Clocks24.as_u32()
}

#[inline(always)]
pub fn op_ldh_a8_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let offset = core.cpu.fetch8(&mut core.bus);
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(offset)));
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_ld_a16_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks16.as_u32()
}

#[inline(always)]
pub fn op_ldh_a_a8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let offset = core.cpu.fetch8(&mut core.bus);
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(offset)));
    core.cpu.a = core.bus.read8(addr);
    CycleCost::Clocks12.as_u32()
}

#[inline(always)]
pub fn op_unimplemented() -> u32 {
    CycleCost::Clocks4.as_u32()
}
