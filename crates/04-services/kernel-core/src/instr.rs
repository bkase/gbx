//! Instruction metadata helpers and opcode implementations shared across core milestones.
//!
//! The scalar M1 core currently executes through a large `match` in
//! [`Core::execute_opcode`](crate::core::Core::execute_opcode). This module hosts
//! helper routines to keep that match manageable while preserving a scalar-friendly
//! control flow.

use crate::bus::Bus;
use crate::core::Core;
use crate::exec::{Exec, Flags};
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
    /// 20-cycle operations.
    Clocks20 = 20,
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

/// High-level ALU operation categories used by opcode helpers.
#[derive(Clone, Copy)]
pub enum AluOp {
    /// 8-bit addition.
    Add,
    /// 8-bit addition with carry.
    Adc,
    /// 8-bit subtraction.
    Sub,
    /// 8-bit subtraction with borrow.
    Sbc,
    /// Bitwise AND.
    And,
    /// Bitwise XOR.
    Xor,
    /// Bitwise OR.
    Or,
    /// Compare (subtraction without storing the result).
    Cp,
}

/// Implements the `NOP` instruction.
#[inline(always)]
pub fn op_nop() -> u32 {
    CycleCost::Clocks4.as_u32()
}

/// Placeholder handler used for unimplemented opcodes.
#[inline(always)]
pub fn op_unimplemented() -> u32 {
    CycleCost::Clocks4.as_u32()
}

/// Reads an 8-bit operand referred to by the register selector encoding.
#[inline(always)]
pub fn read_r8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, code: u8) -> (E::U8, u32) {
    match code & 0x07 {
        0 => (core.cpu.b, 0),
        1 => (core.cpu.c, 0),
        2 => (core.cpu.d, 0),
        3 => (core.cpu.e, 0),
        4 => (core.cpu.h, 0),
        5 => (core.cpu.l, 0),
        6 => {
            let value = core.bus.read8(core.cpu.hl());
            (value, CycleCost::Clocks4.as_u32())
        }
        7 => (core.cpu.a, 0),
        _ => unreachable!(),
    }
}

/// Writes an 8-bit operand to the register or memory target specified by the encoding.
#[inline(always)]
pub fn write_r8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, code: u8, value: E::U8) -> u32 {
    match code & 0x07 {
        0 => {
            core.cpu.b = value;
            0
        }
        1 => {
            core.cpu.c = value;
            0
        }
        2 => {
            core.cpu.d = value;
            0
        }
        3 => {
            core.cpu.e = value;
            0
        }
        4 => {
            core.cpu.h = value;
            0
        }
        5 => {
            core.cpu.l = value;
            0
        }
        6 => {
            let addr = core.cpu.hl();
            core.bus.write8(addr, value);
            CycleCost::Clocks4.as_u32()
        }
        7 => {
            core.cpu.a = value;
            0
        }
        _ => unreachable!(),
    }
}

/// Evaluates a conditional branch code against the current flag register.
#[inline(always)]
pub fn cond<E: Exec>(flags: &Flags<<E as Exec>::Mask>, cc: u8) -> bool {
    match cc & 0x03 {
        0 => !flags.z(), // NZ
        1 => flags.z(),  // Z
        2 => !flags.c(), // NC
        3 => flags.c(),  // C
        _ => unreachable!(),
    }
}

#[inline(always)]
fn alu_assign<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, operand: E::U8, op: AluOp) {
    match op {
        AluOp::Add => {
            let result = E::add8(core.cpu.a, operand, false, &mut core.cpu.f);
            core.cpu.a = result;
        }
        AluOp::Adc => {
            let result = E::add8(core.cpu.a, operand, core.cpu.f.c(), &mut core.cpu.f);
            core.cpu.a = result;
        }
        AluOp::Sub => {
            let result = E::sub8(core.cpu.a, operand, false, &mut core.cpu.f);
            core.cpu.a = result;
        }
        AluOp::Sbc => {
            let result = E::sub8(core.cpu.a, operand, core.cpu.f.c(), &mut core.cpu.f);
            core.cpu.a = result;
        }
        AluOp::And => {
            let result = E::and(core.cpu.a, operand);
            core.cpu.a = result;
            core.cpu.f.set_z(E::to_u8(result) == 0);
            core.cpu.f.set_n(false);
            core.cpu.f.set_h(true);
            core.cpu.f.set_c(false);
        }
        AluOp::Xor => {
            let result = E::xor(core.cpu.a, operand);
            core.cpu.a = result;
            core.cpu.f.set_z(E::to_u8(result) == 0);
            core.cpu.f.set_n(false);
            core.cpu.f.set_h(false);
            core.cpu.f.set_c(false);
        }
        AluOp::Or => {
            let result = E::or(core.cpu.a, operand);
            core.cpu.a = result;
            core.cpu.f.set_z(E::to_u8(result) == 0);
            core.cpu.f.set_n(false);
            core.cpu.f.set_h(false);
            core.cpu.f.set_c(false);
        }
        AluOp::Cp => {
            let _ = E::sub8(core.cpu.a, operand, false, &mut core.cpu.f);
        }
    }
}

/// Loads a 16-bit immediate into one of the register pairs.
#[inline(always)]
pub fn op_ld_rr_d16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let imm = core.cpu.fetch16(&mut core.bus);
    match rp & 0x03 {
        0 => core.cpu.set_bc(imm),
        1 => core.cpu.set_de(imm),
        2 => core.cpu.set_hl(imm),
        3 => core.cpu.sp = imm,
        _ => unreachable!(),
    }
    CycleCost::Clocks12.as_u32()
}

/// Stores accumulator into the memory location pointed by a register pair.
#[inline(always)]
pub fn op_ld_mem_rr_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let addr = match rp & 0x03 {
        0 => core.cpu.bc(),
        1 => core.cpu.de(),
        _ => unreachable!(),
    };
    let value = core.cpu.a;
    core.bus.write8(addr, value);
    CycleCost::Clocks8.as_u32()
}

/// Loads the accumulator from the memory location pointed by a register pair.
#[inline(always)]
pub fn op_ld_a_mem_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let addr = match rp & 0x03 {
        0 => core.cpu.bc(),
        1 => core.cpu.de(),
        _ => unreachable!(),
    };
    core.cpu.a = core.bus.read8(addr);
    CycleCost::Clocks8.as_u32()
}

/// Loads an immediate byte into a register or `(HL)` target.
#[inline(always)]
pub fn op_ld_r_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, dst: u8) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    let extra = write_r8(core, dst, imm);
    CycleCost::Clocks8.as_u32() + extra
}

/// Stores an immediate byte into `(HL)`.
#[inline(always)]
pub fn op_ld_hl_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    let addr = core.cpu.hl();
    core.bus.write8(addr, imm);
    CycleCost::Clocks12.as_u32()
}

/// Stores the stack pointer to an absolute address.
#[inline(always)]
pub fn op_ld_mem_a16_sp<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    let sp = core.cpu.sp;
    let (hi, lo) = E::split_u16(sp);
    core.bus.write8(addr, lo);
    let next_addr = E::from_u16(E::to_u16(addr).wrapping_add(1));
    core.bus.write8(next_addr, hi);
    CycleCost::Clocks20.as_u32()
}

/// Writes the accumulator to the high-memory IO space using an immediate offset.
#[inline(always)]
pub fn op_ldh_a8_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let offset = core.cpu.fetch8(&mut core.bus);
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(offset)));
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks12.as_u32()
}

/// Reads the accumulator from the high-memory IO space using an immediate offset.
#[inline(always)]
pub fn op_ldh_a_a8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let offset = core.cpu.fetch8(&mut core.bus);
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(offset)));
    let value = core.bus.read8(addr);
    core.cpu.a = value;
    CycleCost::Clocks12.as_u32()
}

/// Writes the accumulator to the high-memory IO space using register `C`.
#[inline(always)]
pub fn op_ldh_c_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(core.cpu.c)));
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks8.as_u32()
}

/// Reads the accumulator from the high-memory IO space using register `C`.
#[inline(always)]
pub fn op_ldh_a_c<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = E::from_u16(0xFF00 | u16::from(E::to_u8(core.cpu.c)));
    core.cpu.a = core.bus.read8(addr);
    CycleCost::Clocks8.as_u32()
}

/// Loads the accumulator from an absolute 16-bit address.
#[inline(always)]
pub fn op_ld_a_a16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    core.cpu.a = core.bus.read8(addr);
    CycleCost::Clocks16.as_u32()
}

/// Stores the accumulator to an absolute 16-bit address.
#[inline(always)]
pub fn op_ld_a16_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    core.bus.write8(addr, core.cpu.a);
    CycleCost::Clocks16.as_u32()
}

/// Copies a register value to another register or `(HL)`.
#[inline(always)]
pub fn op_ld_r_r<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, opcode: u8) -> u32 {
    let dst = (opcode >> 3) & 0x07;
    let src = opcode & 0x07;
    let (value, read_cycles) = read_r8(core, src);
    let write_cycles = write_r8(core, dst, value);
    CycleCost::Clocks4.as_u32() + read_cycles + write_cycles
}

/// Increments an 8-bit register or memory location.
#[inline(always)]
pub fn op_inc_r<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, idx: u8) -> u32 {
    match idx & 0x07 {
        6 => {
            let addr = core.cpu.hl();
            let value = core.bus.read8(addr);
            let (result, _) = core.inc_reg(value);
            core.bus.write8(addr, result);
            CycleCost::Clocks12.as_u32()
        }
        0 => {
            let (value, cycles) = core.inc_reg(core.cpu.b);
            core.cpu.b = value;
            cycles
        }
        1 => {
            let (value, cycles) = core.inc_reg(core.cpu.c);
            core.cpu.c = value;
            cycles
        }
        2 => {
            let (value, cycles) = core.inc_reg(core.cpu.d);
            core.cpu.d = value;
            cycles
        }
        3 => {
            let (value, cycles) = core.inc_reg(core.cpu.e);
            core.cpu.e = value;
            cycles
        }
        4 => {
            let (value, cycles) = core.inc_reg(core.cpu.h);
            core.cpu.h = value;
            cycles
        }
        5 => {
            let (value, cycles) = core.inc_reg(core.cpu.l);
            core.cpu.l = value;
            cycles
        }
        7 => {
            let (value, cycles) = core.inc_reg(core.cpu.a);
            core.cpu.a = value;
            cycles
        }
        _ => unreachable!(),
    }
}

/// Decrements an 8-bit register or memory location.
#[inline(always)]
pub fn op_dec_r<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, idx: u8) -> u32 {
    match idx & 0x07 {
        6 => {
            let addr = core.cpu.hl();
            let value = core.bus.read8(addr);
            let (result, _) = core.dec_reg(value);
            core.bus.write8(addr, result);
            CycleCost::Clocks12.as_u32()
        }
        0 => {
            let (value, cycles) = core.dec_reg(core.cpu.b);
            core.cpu.b = value;
            cycles
        }
        1 => {
            let (value, cycles) = core.dec_reg(core.cpu.c);
            core.cpu.c = value;
            cycles
        }
        2 => {
            let (value, cycles) = core.dec_reg(core.cpu.d);
            core.cpu.d = value;
            cycles
        }
        3 => {
            let (value, cycles) = core.dec_reg(core.cpu.e);
            core.cpu.e = value;
            cycles
        }
        4 => {
            let (value, cycles) = core.dec_reg(core.cpu.h);
            core.cpu.h = value;
            cycles
        }
        5 => {
            let (value, cycles) = core.dec_reg(core.cpu.l);
            core.cpu.l = value;
            cycles
        }
        7 => {
            let (value, cycles) = core.dec_reg(core.cpu.a);
            core.cpu.a = value;
            cycles
        }
        _ => unreachable!(),
    }
}

/// Increments a 16-bit register pair.
#[inline(always)]
pub fn op_inc_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    match rp & 0x03 {
        0 => {
            let (value, cycles) = core.inc16(core.cpu.bc());
            core.cpu.set_bc(value);
            cycles
        }
        1 => {
            let (value, cycles) = core.inc16(core.cpu.de());
            core.cpu.set_de(value);
            cycles
        }
        2 => {
            let (value, cycles) = core.inc16(core.cpu.hl());
            core.cpu.set_hl(value);
            cycles
        }
        3 => {
            let (value, cycles) = core.inc16(core.cpu.sp);
            core.cpu.sp = value;
            cycles
        }
        _ => unreachable!(),
    }
}

/// Decrements a 16-bit register pair.
#[inline(always)]
pub fn op_dec_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    match rp & 0x03 {
        0 => {
            let (value, cycles) = core.dec16(core.cpu.bc());
            core.cpu.set_bc(value);
            cycles
        }
        1 => {
            let (value, cycles) = core.dec16(core.cpu.de());
            core.cpu.set_de(value);
            cycles
        }
        2 => {
            let (value, cycles) = core.dec16(core.cpu.hl());
            core.cpu.set_hl(value);
            cycles
        }
        3 => {
            let (value, cycles) = core.dec16(core.cpu.sp);
            core.cpu.sp = value;
            cycles
        }
        _ => unreachable!(),
    }
}

/// Adds a register pair to HL.
#[inline(always)]
pub fn op_add_hl_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let rhs = match rp & 0x03 {
        0 => core.cpu.bc(),
        1 => core.cpu.de(),
        2 => core.cpu.hl(),
        3 => core.cpu.sp,
        _ => unreachable!(),
    };
    core.add16_hl(rhs)
}

/// Adds a signed immediate value to SP.
#[inline(always)]
pub fn op_add_sp_e8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    let (result, h, c) = core.add_sp_e8(imm);
    core.cpu.sp = result;
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(h);
    core.cpu.f.set_c(c);
    CycleCost::Clocks16.as_u32()
}

/// Computes `HL = SP + e8` while updating flags.
#[inline(always)]
pub fn op_ld_hl_sp_plus_e8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    let (result, h, c) = core.add_sp_e8(imm);
    core.cpu.set_hl(result);
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(h);
    core.cpu.f.set_c(c);
    CycleCost::Clocks12.as_u32()
}

/// Copies HL into SP.
#[inline(always)]
pub fn op_ld_sp_hl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.sp = core.cpu.hl();
    CycleCost::Clocks8.as_u32()
}

/// Stores `A` to `(HL)` and increments HL.
#[inline(always)]
pub fn op_ldi_hl_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.bus.write8(addr, core.cpu.a);
    let next = E::from_u16(E::to_u16(addr).wrapping_add(1));
    core.cpu.set_hl(next);
    CycleCost::Clocks8.as_u32()
}

/// Loads `A` from `(HL)` and increments HL.
#[inline(always)]
pub fn op_ldi_a_hl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    let value = core.bus.read8(addr);
    let next = E::from_u16(E::to_u16(addr).wrapping_add(1));
    core.cpu.set_hl(next);
    core.cpu.a = value;
    CycleCost::Clocks8.as_u32()
}

/// Stores `A` to `(HL)` and then decrements HL.
#[inline(always)]
pub fn op_ldd_hl_a<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.bus.write8(addr, core.cpu.a);
    let next = E::from_u16(E::to_u16(addr).wrapping_sub(1));
    core.cpu.set_hl(next);
    CycleCost::Clocks8.as_u32()
}

/// Loads `A` from `(HL)` and then decrements HL.
#[inline(always)]
pub fn op_ldd_a_hl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    let value = core.bus.read8(addr);
    let next = E::from_u16(E::to_u16(addr).wrapping_sub(1));
    core.cpu.set_hl(next);
    core.cpu.a = value;
    CycleCost::Clocks8.as_u32()
}

/// Performs an unconditional relative jump.
#[inline(always)]
pub fn op_jr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let off = core.cpu.fetch8(&mut core.bus);
    core.jump_relative(off, true);
    CycleCost::Clocks12.as_u32()
}

/// Performs a conditional relative jump.
#[inline(always)]
pub fn op_jr_cc<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, opcode: u8) -> u32 {
    let off = core.cpu.fetch8(&mut core.bus);
    let cc = (opcode >> 3) & 0x03;
    let taken = cond::<E>(&core.cpu.f, cc);
    core.jump_relative(off, taken);
    if taken {
        CycleCost::Clocks12.as_u32()
    } else {
        CycleCost::Clocks8.as_u32()
    }
}

/// Executes an ALU operation on `A` with a register or `(HL)` operand.
#[inline(always)]
pub fn op_alu_a_r<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, opcode: u8, op: AluOp) -> u32 {
    let (operand, read_cycles) = read_r8(core, opcode);
    alu_assign(core, operand, op);
    CycleCost::Clocks4.as_u32() + read_cycles
}

/// Executes an ALU operation on `A` with an immediate operand.
#[inline(always)]
pub fn op_alu_a_d8<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, op: AluOp) -> u32 {
    let imm = core.cpu.fetch8(&mut core.bus);
    alu_assign(core, imm, op);
    CycleCost::Clocks8.as_u32()
}

/// Rotates `A` left without carry and clears `Z`.
#[inline(always)]
pub fn op_rlca<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::to_u8(core.cpu.a);
    let carry = value >> 7;
    let result = (value << 1) | carry;
    core.cpu.a = E::from_u8(result);
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry != 0);
    CycleCost::Clocks4.as_u32()
}

/// Rotates `A` left through the carry flag.
#[inline(always)]
pub fn op_rla<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::to_u8(core.cpu.a);
    let carry_in = core.cpu.f.c() as u8;
    let carry_out = value >> 7;
    let result = (value << 1) | carry_in;
    core.cpu.a = E::from_u8(result);
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry_out != 0);
    CycleCost::Clocks4.as_u32()
}

/// Rotates `A` right without carry and clears `Z`.
#[inline(always)]
pub fn op_rrca<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::to_u8(core.cpu.a);
    let carry = value & 0x01;
    let result = (value >> 1) | (carry << 7);
    core.cpu.a = E::from_u8(result);
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry != 0);
    CycleCost::Clocks4.as_u32()
}

/// Rotates `A` right through the carry flag.
#[inline(always)]
pub fn op_rra<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::to_u8(core.cpu.a);
    let carry_in = if core.cpu.f.c() { 1 } else { 0 };
    let carry_out = value & 0x01;
    let result = (value >> 1) | (carry_in << 7);
    core.cpu.a = E::from_u8(result);
    core.cpu.f.set_z(false);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry_out != 0);
    CycleCost::Clocks4.as_u32()
}

/// Adjusts `A` for binary-coded decimal representation.
#[inline(always)]
pub fn op_daa<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let mut a = E::to_u8(core.cpu.a);
    let mut carry = core.cpu.f.c();

    if !core.cpu.f.n() {
        if carry || a > 0x99 {
            a = a.wrapping_add(0x60);
            carry = true;
        }
        if core.cpu.f.h() || (a & 0x0F) > 0x09 {
            a = a.wrapping_add(0x06);
        }
    } else {
        if carry {
            a = a.wrapping_sub(0x60);
        }
        if core.cpu.f.h() {
            a = a.wrapping_sub(0x06);
        }
    }

    core.cpu.a = E::from_u8(a);
    core.cpu.f.set_z(a == 0);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry);
    CycleCost::Clocks4.as_u32()
}

/// Complements the accumulator.
#[inline(always)]
pub fn op_cpl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let value = E::to_u8(core.cpu.a) ^ 0xFF;
    core.cpu.a = E::from_u8(value);
    core.cpu.f.set_n(true);
    core.cpu.f.set_h(true);
    CycleCost::Clocks4.as_u32()
}

/// Sets the carry flag and clears `N`/`H`.
#[inline(always)]
pub fn op_scf<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(true);
    CycleCost::Clocks4.as_u32()
}

/// Complements the carry flag while clearing `N`/`H`.
#[inline(always)]
pub fn op_ccf<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(!core.cpu.f.c());
    CycleCost::Clocks4.as_u32()
}

/// Disables interrupts immediately.
#[inline(always)]
pub fn op_di<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.ime = false;
    core.cpu.enable_ime_pending = false;
    CycleCost::Clocks4.as_u32()
}

/// Schedules interrupts to enable after the next instruction.
#[inline(always)]
pub fn op_ei<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.enable_ime_pending = true;
    CycleCost::Clocks4.as_u32()
}

/// Enters the HALT low-power state.
#[inline(always)]
pub fn op_halt<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.halted = true;
    CycleCost::Clocks4.as_u32()
}

/// Enters the STOP low-power state (treated as HALT).
#[inline(always)]
pub fn op_stop<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    core.cpu.halted = true;
    CycleCost::Clocks4.as_u32()
}

/// Jumps to the address stored in HL.
#[inline(always)]
pub fn op_jp_hl<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.hl();
    core.cpu.pc = addr;
    CycleCost::Clocks4.as_u32()
}

/// Pushes a register pair onto the stack.
#[inline(always)]
pub fn op_push_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let value = match rp & 0x03 {
        0 => core.cpu.bc(),
        1 => core.cpu.de(),
        2 => core.cpu.hl(),
        3 => core.cpu.af(),
        _ => unreachable!(),
    };
    core.cpu.push16(&mut core.bus, value);
    CycleCost::Clocks16.as_u32()
}

/// Pops a register pair from the stack.
#[inline(always)]
pub fn op_pop_rr<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, rp: u8) -> u32 {
    let value = core.cpu.pop16(&mut core.bus);
    match rp & 0x03 {
        0 => core.cpu.set_bc(value),
        1 => core.cpu.set_de(value),
        2 => core.cpu.set_hl(value),
        3 => core.cpu.set_af(value),
        _ => unreachable!(),
    }
    CycleCost::Clocks12.as_u32()
}

/// Implements the CB-prefixed `RLC r` mnemonic.
#[inline(always)]
fn cb_rlc(value: u8) -> (u8, bool) {
    let carry = (value & 0x80) != 0;
    (value.rotate_left(1), carry)
}

/// Implements the CB-prefixed `RRC r` mnemonic.
#[inline(always)]
fn cb_rrc(value: u8) -> (u8, bool) {
    let carry = (value & 0x01) != 0;
    (value.rotate_right(1), carry)
}

/// Implements the CB-prefixed `RL r` mnemonic.
#[inline(always)]
fn cb_rl(value: u8, carry_in: bool) -> (u8, bool) {
    let carry = (value & 0x80) != 0;
    let result = (value << 1) | u8::from(carry_in);
    (result, carry)
}

/// Implements the CB-prefixed `RR r` mnemonic.
#[inline(always)]
fn cb_rr(value: u8, carry_in: bool) -> (u8, bool) {
    let carry = (value & 0x01) != 0;
    let result = (value >> 1) | (u8::from(carry_in) << 7);
    (result, carry)
}

/// Implements the CB-prefixed `SLA r` mnemonic.
#[inline(always)]
fn cb_sla(value: u8) -> (u8, bool) {
    let carry = (value & 0x80) != 0;
    (value << 1, carry)
}

/// Implements the CB-prefixed `SRA r` mnemonic.
#[inline(always)]
fn cb_sra(value: u8) -> (u8, bool) {
    let carry = (value & 0x01) != 0;
    ((value >> 1) | (value & 0x80), carry)
}

/// Implements the CB-prefixed `SWAP r` mnemonic.
#[inline(always)]
fn cb_swap(value: u8) -> (u8, bool) {
    (value.rotate_left(4), false)
}

/// Implements the CB-prefixed `SRL r` mnemonic.
#[inline(always)]
fn cb_srl(value: u8) -> (u8, bool) {
    let carry = (value & 0x01) != 0;
    (value >> 1, carry)
}

/// Executes the CB-prefixed rotate/shift group (`RLC`..`SRL`).
#[inline(always)]
fn cb_exec_rotate<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, variant: u8, z: u8) -> (u32, u32) {
    let (value, read_cycles) = read_r8(core, z);
    let value_u8 = E::to_u8(value);
    let carry_in = core.cpu.f.c();
    let (result_u8, carry) = match variant {
        0 => cb_rlc(value_u8),
        1 => cb_rrc(value_u8),
        2 => cb_rl(value_u8, carry_in),
        3 => cb_rr(value_u8, carry_in),
        4 => cb_sla(value_u8),
        5 => cb_sra(value_u8),
        6 => cb_swap(value_u8),
        7 => cb_srl(value_u8),
        _ => unreachable!(),
    };
    let result = E::from_u8(result_u8);
    let write_cycles = write_r8(core, z, result);
    core.cpu.f.set_z(result_u8 == 0);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(false);
    core.cpu.f.set_c(carry);
    (read_cycles, write_cycles)
}

/// Executes the CB-prefixed `BIT b, r` mnemonics.
#[inline(always)]
fn cb_exec_bit<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, bit: u8, z: u8) -> u32 {
    let (value, read_cycles) = read_r8(core, z);
    let result = E::to_u8(value);
    let zero = result & (1 << bit) == 0;
    core.cpu.f.set_z(zero);
    core.cpu.f.set_n(false);
    core.cpu.f.set_h(true);
    read_cycles
}

/// Executes the CB-prefixed `RES b, r` mnemonics.
#[inline(always)]
fn cb_exec_res<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, bit: u8, z: u8) -> (u32, u32) {
    let (value, read_cycles) = read_r8(core, z);
    let mut result = E::to_u8(value);
    result &= !(1 << bit);
    let write_cycles = write_r8(core, z, E::from_u8(result));
    (read_cycles, write_cycles)
}

/// Executes the CB-prefixed `SET b, r` mnemonics.
#[inline(always)]
fn cb_exec_set<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, bit: u8, z: u8) -> (u32, u32) {
    let (value, read_cycles) = read_r8(core, z);
    let mut result = E::to_u8(value);
    result |= 1 << bit;
    let write_cycles = write_r8(core, z, E::from_u8(result));
    (read_cycles, write_cycles)
}

/// Executes a CB-prefixed extended opcode.
#[inline(always)]
pub fn op_cb<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, sub: u8) -> u32 {
    let x = sub >> 6;
    let y = (sub >> 3) & 0x07;
    let z = sub & 0x07;

    match x {
        0 => {
            let (read_cycles, write_cycles) = cb_exec_rotate(core, y, z);
            CycleCost::Clocks8.as_u32() + read_cycles + write_cycles
        }
        1 => {
            let read_cycles = cb_exec_bit(core, y, z);
            CycleCost::Clocks8.as_u32() + read_cycles
        }
        2 => {
            let (read_cycles, write_cycles) = cb_exec_res(core, y, z);
            CycleCost::Clocks8.as_u32() + read_cycles + write_cycles
        }
        3 => {
            let (read_cycles, write_cycles) = cb_exec_set(core, y, z);
            CycleCost::Clocks8.as_u32() + read_cycles + write_cycles
        }
        _ => unreachable!(),
    }
}

/// Performs an absolute jump.
#[inline(always)]
pub fn op_jp_a16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    core.cpu.pc = addr;
    CycleCost::Clocks16.as_u32()
}

/// Returns from a subroutine.
#[inline(always)]
pub fn op_ret<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.pop16(&mut core.bus);
    core.cpu.pc = addr;
    CycleCost::Clocks16.as_u32()
}

/// Conditionally returns from a subroutine.
#[inline(always)]
pub fn op_ret_cc<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, take: bool) -> u32 {
    if take {
        let addr = core.cpu.pop16(&mut core.bus);
        core.cpu.pc = addr;
        CycleCost::Clocks20.as_u32()
    } else {
        CycleCost::Clocks8.as_u32()
    }
}

/// Calls a subroutine at an absolute address.
#[inline(always)]
pub fn op_call_a16<E: Exec, B: Bus<E>>(core: &mut Core<E, B>) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    let ret = core.cpu.pc;
    core.cpu.push16(&mut core.bus, ret);
    core.cpu.pc = addr;
    CycleCost::Clocks24.as_u32()
}

/// Conditionally calls a subroutine at an absolute address.
#[inline(always)]
pub fn op_call_cc<E: Exec, B: Bus<E>>(core: &mut Core<E, B>, take: bool) -> u32 {
    let addr = core.cpu.fetch16(&mut core.bus);
    if take {
        let ret = core.cpu.pc;
        core.cpu.push16(&mut core.bus, ret);
        core.cpu.pc = addr;
        CycleCost::Clocks24.as_u32()
    } else {
        CycleCost::Clocks12.as_u32()
    }
}
