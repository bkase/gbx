use crate::bus::Bus;
use crate::exec::Exec;
use crate::ppu_stub::CYCLES_PER_FRAME;
use crate::{BusScalar, Core, CoreConfig, Model, Scalar};
use proptest::prelude::*;
use std::sync::Arc;

/// Builds a scalar core with the provided program bytes at reset address 0x0100.
fn core_with_program(bytes: &[u8]) -> Core<Scalar, BusScalar> {
    let mut rom = vec![0u8; 0x8000];
    let start = 0x0100;
    rom[start..start + bytes.len()].copy_from_slice(bytes);
    Core::new(
        BusScalar::new(Arc::from(rom.into_boxed_slice())),
        CoreConfig::default(),
        Model::Dmg,
    )
}

/// Verifies `ADD A,r` updates flags according to fixture cases.
#[test]
fn cpu_add8_flags() {
    let cases = [
        (0x0F, 0x01, 0x10, false, false, true, false),
        (0xFF, 0x01, 0x00, true, false, true, true),
        (0x80, 0x80, 0x00, true, false, false, true),
    ];

    for (a, b, expected, z, n, h, c) in cases {
        let mut core = core_with_program(&[0x80, 0x76]); // ADD A,B; HALT
        core.cpu.a = Scalar::from_u8(a);
        core.cpu.b = Scalar::from_u8(b);
        core.cpu.f.from_byte(0);

        let consumed = core.step_cycles(8);
        assert!(
            consumed >= 4,
            "instruction should consume at least 4 cycles"
        );
        assert_eq!(Scalar::to_u8(core.cpu.a), expected, "ADD result mismatch");
        assert_eq!(core.cpu.f.z(), z, "Z flag mismatch");
        assert_eq!(core.cpu.f.n(), n, "N flag mismatch");
        assert_eq!(core.cpu.f.h(), h, "H flag mismatch");
        assert_eq!(core.cpu.f.c(), c, "C flag mismatch");
    }
}

/// Verifies `ADC` respects the incoming carry and sets flags correctly.
#[test]
fn cpu_adc8_flags() {
    let cases = [
        (0x0F, 0x01, true, 0x11, false, false, true, false),
        (0xFF, 0x00, true, 0x00, true, false, true, true),
        (0x7F, 0x00, true, 0x80, false, false, true, false),
    ];

    for (a, imm, carry_in, expected, z, n, h, c) in cases {
        let mut core = core_with_program(&[0xCE, imm, 0x76]); // ADC A,d8; HALT
        core.cpu.a = Scalar::from_u8(a);
        core.cpu.f.from_byte(0);
        core.cpu.f.set_c(carry_in);

        core.step_cycles(16);
        assert_eq!(Scalar::to_u8(core.cpu.a), expected, "ADC result mismatch");
        assert_eq!(core.cpu.f.z(), z, "ADC Z flag mismatch");
        assert_eq!(core.cpu.f.n(), n, "ADC N flag mismatch");
        assert_eq!(core.cpu.f.h(), h, "ADC H flag mismatch");
        assert_eq!(core.cpu.f.c(), c, "ADC C flag mismatch");
    }
}

/// Ensures `SBC` incorporates carry-in and produces expected flag values.
#[test]
fn cpu_sbc8_flags() {
    let cases = [
        (0x10, 0x01, false, 0x0F, false, true, true, false),
        (0x10, 0x01, true, 0x0E, false, true, true, false),
        (0x00, 0x01, false, 0xFF, false, true, true, true),
    ];

    for (a, imm, carry_in, expected, z, n, h, c) in cases {
        let mut core = core_with_program(&[0xDE, imm, 0x76]); // SBC A,d8; HALT
        core.cpu.a = Scalar::from_u8(a);
        core.cpu.f.from_byte(0);
        core.cpu.f.set_c(carry_in);

        core.step_cycles(16);
        assert_eq!(Scalar::to_u8(core.cpu.a), expected, "SBC result mismatch");
        assert_eq!(core.cpu.f.z(), z, "SBC Z flag mismatch");
        assert_eq!(core.cpu.f.n(), n, "SBC N flag mismatch");
        assert_eq!(core.cpu.f.h(), h, "SBC H flag mismatch");
        assert_eq!(core.cpu.f.c(), c, "SBC C flag mismatch");
    }
}

/// Confirms logical immediates (`AND`, `OR`, `XOR`) follow flag semantics.
#[test]
fn cpu_immediate_logic_flags() {
    // AND
    let mut core = core_with_program(&[0xE6, 0x0F, 0x76]);
    core.cpu.a = Scalar::from_u8(0x33);
    core.cpu.f.from_byte(0xFF);
    core.step_cycles(16);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x03);
    assert!(core.cpu.f.h());
    assert!(!core.cpu.f.n());
    assert!(!core.cpu.f.c());

    // OR
    let mut core = core_with_program(&[0xF6, 0x0F, 0x76]);
    core.cpu.a = Scalar::from_u8(0x30);
    core.cpu.f.from_byte(0);
    core.step_cycles(16);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x3F);
    assert!(!core.cpu.f.h());
    assert!(!core.cpu.f.n());
    assert!(!core.cpu.f.c());

    // XOR
    let mut core = core_with_program(&[0xEE, 0xFF, 0x76]);
    core.cpu.a = Scalar::from_u8(0xFF);
    core.cpu.f.from_byte(0xFF);
    core.step_cycles(16);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x00);
    assert!(core.cpu.f.z());
    assert!(!core.cpu.f.h());
    assert!(!core.cpu.f.n());
    assert!(!core.cpu.f.c());
}

/// Checks `CP` preserves accumulator and matches `SUB` flags.
#[test]
fn cpu_cp_preserves_a_and_flags_match_sub() {
    let mut core_sub = core_with_program(&[0xD6, 0x10, 0x76]); // SUB d8
    core_sub.cpu.a = Scalar::from_u8(0x20);
    core_sub.cpu.f.from_byte(0);
    core_sub.step_cycles(16);
    let flags_sub = core_sub.cpu.f.to_byte();

    let mut core_cp = core_with_program(&[0xFE, 0x10, 0x76]); // CP d8
    core_cp.cpu.a = Scalar::from_u8(0x20);
    core_cp.cpu.f.from_byte(0);
    core_cp.step_cycles(16);

    assert_eq!(Scalar::to_u8(core_cp.cpu.a), 0x20, "CP must not mutate A");
    assert_eq!(
        core_cp.cpu.f.to_byte(),
        flags_sub,
        "CP flags should mirror SUB flags"
    );
}

/// Exercises `INC` / `DEC` on `(HL)` while ensuring carry preservation.
#[test]
fn inc_dec_hl_preserves_carry() {
    let mut core = core_with_program(&[0x21, 0x00, 0xC0, 0x36, 0x0F, 0x34, 0x35, 0x76]);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_c(true);
    core.step_cycles(64);

    let addr = Scalar::from_u16(0xC000);
    let value = core.bus.read8(addr);
    assert_eq!(Scalar::to_u8(value), 0x0F);
    assert!(
        core.cpu.f.c(),
        "Carry flag must be preserved across INC/DEC"
    );
    assert!(core.cpu.f.n(), "DEC should set N");
    assert!(
        !core.cpu.f.z(),
        "Value should not be zero after INC then DEC"
    );
}

/// Validates `DAA` behaviour for both addition and subtraction contexts.
#[test]
fn daa_adjusts_after_add_and_sub() {
    // Addition path: expect carry and zero after adjustment.
    let mut core = core_with_program(&[0x27, 0x76]);
    core.cpu.a = Scalar::from_u8(0x9A);
    core.cpu.f.from_byte(0);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x00);
    assert!(core.cpu.f.z());
    assert!(core.cpu.f.c());
    assert!(!core.cpu.f.h());

    // Subtraction path: expect carry retained.
    let mut core = core_with_program(&[0x27, 0x76]);
    core.cpu.a = Scalar::from_u8(0x15);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_n(true);
    core.cpu.f.set_c(true);
    core.cpu.f.set_h(true);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0xAF);
    assert!(core.cpu.f.c());
    assert!(!core.cpu.f.h());
    assert!(core.cpu.f.n());
}

/// Captures serial transfers triggered via writes to the SC register.
#[test]
fn serial_transfer_accumulates_output() {
    let mut core = core_with_program(&[0x76]);
    let data_addr = Scalar::from_u16(0xFF01);
    let ctrl_addr = Scalar::from_u16(0xFF02);

    core.bus.write8(data_addr, Scalar::from_u8(b'O'));
    core.bus.write8(ctrl_addr, Scalar::from_u8(0x81));
    core.bus.write8(data_addr, Scalar::from_u8(b'K'));
    core.bus.write8(ctrl_addr, Scalar::from_u8(0x81));

    let mut expected = String::new();
    expected.push('O');
    expected.push('K');

    assert_eq!(core.bus.take_serial(), expected);
}

/// Checks all four accumulator rotate opcodes for proper carry handling.
#[test]
fn rotate_a_variants() {
    // RLCA
    let mut core = core_with_program(&[0x07, 0x76]);
    core.cpu.a = Scalar::from_u8(0x81);
    core.cpu.f.from_byte(0xFF);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x03);
    assert!(core.cpu.f.c());
    assert!(!core.cpu.f.z());

    // RLA
    let mut core = core_with_program(&[0x17, 0x76]);
    core.cpu.a = Scalar::from_u8(0x80);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_c(true);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x01);
    assert!(core.cpu.f.c());

    // RRCA
    let mut core = core_with_program(&[0x0F, 0x76]);
    core.cpu.a = Scalar::from_u8(0x01);
    core.cpu.f.from_byte(0xFF);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x80);
    assert!(core.cpu.f.c());
    assert!(!core.cpu.f.z());

    // RRA
    let mut core = core_with_program(&[0x1F, 0x76]);
    core.cpu.a = Scalar::from_u8(0x01);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_c(true);
    core.step_cycles(8);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0x80);
    assert!(core.cpu.f.c());
}

/// Ensures `ADD HL,rr` updates carry/half-carry without touching zero.
#[test]
fn add_hl_rr_flag_behavior() {
    let mut core = core_with_program(&[0x09, 0x76]); // ADD HL,BC; HALT
    core.cpu.set_hl(Scalar::from_u16(0x0FFF));
    core.cpu.set_bc(Scalar::from_u16(0x0001));
    core.cpu.f.from_byte(0);
    core.cpu.f.set_z(true);

    core.step_cycles(12);
    assert_eq!(Scalar::to_u16(core.cpu.hl()), 0x1000);
    assert!(core.cpu.f.h());
    assert!(!core.cpu.f.c());
    assert!(!core.cpu.f.n());
    assert!(core.cpu.f.z(), "ADD HL should not touch Z");
}

/// Validates `ADD SP,e8` sets flags per hardware rules.
#[test]
fn add_sp_e8_flag_behavior() {
    let mut core = core_with_program(&[0xE8, 0x08, 0x76]);
    core.cpu.sp = Scalar::from_u16(0xFFF8);
    core.cpu.f.from_byte(0xFF);
    core.step_cycles(20);
    assert_eq!(Scalar::to_u16(core.cpu.sp), 0x0000);
    assert!(!core.cpu.f.z());
    assert!(!core.cpu.f.n());
    assert!(core.cpu.f.h());
    assert!(core.cpu.f.c());
}

/// Ensures `LD HL,SP+e8` writes flags identically to `ADD SP,e8`.
#[test]
fn ld_hl_sp_plus_e8_sets_flags() {
    let mut core = core_with_program(&[0xF8, 0xF8, 0x76]);
    core.cpu.sp = Scalar::from_u16(0xFFF8);
    core.cpu.f.from_byte(0);
    core.step_cycles(16);
    assert_eq!(Scalar::to_u16(core.cpu.hl()), 0xFFF0);
    assert!(!core.cpu.f.z());
    assert!(!core.cpu.f.n());
    assert!(core.cpu.f.h());
    assert!(core.cpu.f.c());
}

/// Confirms `PUSH/POP AF` masks lower flag bits during restore.
#[test]
fn push_pop_af_masks_low_bits() {
    let mut core = core_with_program(&[0x31, 0x00, 0xD0, 0xF5, 0x3E, 0xFF, 0xF1, 0x76]);
    core.cpu.a = Scalar::from_u8(0xAB);
    core.cpu.f.from_byte(0xF0);
    core.step_cycles(64);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0xAB);
    assert_eq!(core.cpu.f.to_byte(), 0xF0);
}

/// Measures cycles for taken vs. untaken `JR` conditions.
#[test]
fn jr_condition_cycles() {
    // Not taken (Z = 1)
    let mut core = core_with_program(&[0x20, 0x02, 0x3E, 0xAA, 0x76]);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_z(true);
    let consumed = core.step_cycles(20);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0xAA);
    assert_eq!(consumed, 20);

    // Taken (Z = 0)
    let mut core = core_with_program(&[0x20, 0x02, 0x3E, 0xAA, 0x3E, 0xBB, 0x76]);
    core.cpu.f.from_byte(0);
    core.cpu.f.set_z(false);
    let consumed = core.step_cycles(24);
    assert_eq!(Scalar::to_u8(core.cpu.a), 0xBB);
    assert_eq!(consumed, 24);
}

/// Checks timer overflow reloads TIMA and raises the interrupt flag.
#[test]
fn timers_wrap() {
    let rom = vec![0x76u8; 0x8000]; // HALT-filled ROM keeps CPU idle.
    let mut core = Core::new(
        BusScalar::new(Arc::from(rom.into_boxed_slice())),
        CoreConfig::default(),
        Model::Dmg,
    );
    core.bus.io.set_tac(0b101); // enable timer, 16-cycle frequency
    core.bus.io.set_tma(0xAC);
    core.bus.io.set_tima(0xFF);
    let initial_if = core.bus.io.if_reg();

    core.timers.step(16, &mut core.bus);

    assert_eq!(core.bus.io.tima(), 0xAC, "TIMA should reload from TMA");
    assert_eq!(
        core.bus.io.if_reg(),
        initial_if | 0x04,
        "timer interrupt flag must be raised"
    );
}

/// Confirms the PPU stub signals a frame boundary after one frame of cycles.
#[test]
fn frame_boundary() {
    let rom = vec![0x00u8; 0x8000]; // NOP-filled ROM.
    let mut core = Core::new(
        BusScalar::new(Arc::from(rom.into_boxed_slice())),
        CoreConfig::default(),
        Model::Dmg,
    );

    let consumed = core.step_cycles(CYCLES_PER_FRAME);
    assert!(
        core.frame_ready(),
        "frame boundary should be flagged after one frame"
    );
    assert!(
        consumed >= CYCLES_PER_FRAME.min(consumed),
        "core should account for consumed cycles"
    );
}

/// Verifies consecutive `EI` instructions enable IME after the second opcode.
#[test]
fn double_ei_sets_ime_after_second_instruction() {
    let mut core = core_with_program(&[0xFB, 0xFB, 0x76]);
    core.cpu.ime = false;
    core.cpu.enable_ime_pending = false;

    // Execute first EI.
    core.step_cycles(4);
    assert!(
        !core.cpu.ime,
        "IME should remain disabled immediately after EI"
    );
    assert!(
        core.cpu.enable_ime_pending,
        "EI must schedule IME enable on the next instruction"
    );

    // Execute second EI; IME should enable now and pending should stay latched.
    core.step_cycles(4);
    assert!(
        core.cpu.ime,
        "IME should enable after the instruction following EI"
    );
    assert!(
        core.cpu.enable_ime_pending,
        "Second EI must re-arm the pending flag for the next instruction"
    );
}

/// Ensures `EI` followed by a non-EI clears the pending flag after enabling IME.
#[test]
fn ei_followed_by_nop_clears_pending() {
    let mut core = core_with_program(&[0xFB, 0x00, 0x76]);
    core.cpu.ime = false;
    core.cpu.enable_ime_pending = false;

    // First EI schedules IME enable.
    core.step_cycles(4);
    assert!(core.cpu.enable_ime_pending);

    // NOP should trigger IME enable and clear pending.
    core.step_cycles(4);
    assert!(
        core.cpu.ime,
        "IME should enable after the instruction following EI"
    );
    assert!(
        !core.cpu.enable_ime_pending,
        "Pending flag must clear when a non-EI instruction runs"
    );
}

// Property-based tests verifying ALU/carry behaviour.
proptest! {
    #[test]
    fn prop_adc_matches_reference(a in any::<u8>(), b in any::<u8>(), carry_in in any::<bool>()) {
        let mut core = core_with_program(&[0xCE, b, 0x76]);
        core.cpu.a = Scalar::from_u8(a);
        core.cpu.f.from_byte(0);
        core.cpu.f.set_c(carry_in);
        core.step_cycles(16);

        let carry = if carry_in { 1 } else { 0 };
        let sum = a as u16 + b as u16 + carry as u16;
        let expected = (sum & 0xFF) as u8;
        let h = ((a & 0x0F) + (b & 0x0F) + carry) > 0x0F;
        let c = sum > 0xFF;

        assert_eq!(Scalar::to_u8(core.cpu.a), expected);
        assert_eq!(core.cpu.f.z(), expected == 0);
        assert!(!core.cpu.f.n());
        assert_eq!(core.cpu.f.h(), h);
        assert_eq!(core.cpu.f.c(), c);
    }

    #[test]
    fn prop_cp_matches_sub_flags(a in any::<u8>(), b in any::<u8>()) {
        let mut core_sub = core_with_program(&[0xD6, b, 0x76]);
        core_sub.cpu.a = Scalar::from_u8(a);
        core_sub.cpu.f.from_byte(0);
        core_sub.step_cycles(16);
        let flags_sub = core_sub.cpu.f.to_byte();

        let mut core_cp = core_with_program(&[0xFE, b, 0x76]);
        core_cp.cpu.a = Scalar::from_u8(a);
        core_cp.cpu.f.from_byte(0);
        core_cp.step_cycles(16);

        assert_eq!(Scalar::to_u8(core_cp.cpu.a), a);
        assert_eq!(core_cp.cpu.f.to_byte(), flags_sub);
    }
}
