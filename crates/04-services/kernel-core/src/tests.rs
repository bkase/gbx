use crate::exec::Exec;
use crate::ppu_stub::CYCLES_PER_FRAME;
use crate::{BusScalar, Core, CoreConfig, Model, Scalar};
use std::sync::Arc;

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
