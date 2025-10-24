//! Mooneye timer and interrupt acceptance ROMs checked via the Fibonacci oracle.

use kernel_core::{BusScalar, Core, Exec, Model, Scalar};

const PASS_PATTERN: [u8; 6] = [3, 5, 8, 13, 21, 34];
const FAIL_PATTERN: [u8; 6] = [0x42; 6];
const MAX_STEPS: usize = 5_000_000;
const MAX_CYCLES: u64 = 25_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pass {
        cycles: u64,
        serial_len: usize,
    },
    Fail {
        cycles: u64,
        serial: [u8; 6],
        pc: u16,
        tima: u8,
        tac: u8,
    },
    Timeout {
        cycles: u64,
        serial_len: usize,
        pc: u16,
        tima: u8,
        tac: u8,
        div: u16,
        regs: [u8; 6],
    },
}

fn run_mooneye(path: &str) -> Outcome {
    let rom = testdata::bytes(path);
    let mut core = Core::<Scalar, BusScalar>::from_rom(rom);
    core.reset_post_boot(Model::Dmg);

    let mut cycles = 0u64;
    for _ in 0..MAX_STEPS {
        let (step_cycles, _) = core.step_instruction();
        cycles = cycles.saturating_add(u64::from(step_cycles));

        let regs = [
            Scalar::to_u8(core.cpu.b),
            Scalar::to_u8(core.cpu.c),
            Scalar::to_u8(core.cpu.d),
            Scalar::to_u8(core.cpu.e),
            Scalar::to_u8(core.cpu.h),
            Scalar::to_u8(core.cpu.l),
        ];

        if regs == PASS_PATTERN {
            return Outcome::Pass {
                cycles,
                serial_len: core.bus.serial_out.len(),
            };
        }

        if regs == FAIL_PATTERN {
            let mut serial = [0u8; 6];
            let take = core.bus.serial_out.len().min(6);
            if take > 0 {
                serial[..take]
                    .copy_from_slice(&core.bus.serial_out[core.bus.serial_out.len() - take..]);
            }
            let pc = Scalar::to_u16(core.cpu.pc);
            let tima = core.bus.io.tima();
            let tac = core.bus.io.tac();
            return Outcome::Fail {
                cycles,
                serial,
                pc,
                tima,
                tac,
            };
        }

        if core.bus.serial_out.len() >= PASS_PATTERN.len() {
            let tail = &core.bus.serial_out[core.bus.serial_out.len() - PASS_PATTERN.len()..];
            if tail == PASS_PATTERN {
                return Outcome::Pass {
                    cycles,
                    serial_len: core.bus.serial_out.len(),
                };
            }
            if tail == FAIL_PATTERN {
                let mut serial = [0u8; 6];
                serial.copy_from_slice(tail);
                let pc = Scalar::to_u16(core.cpu.pc);
                let tima = core.bus.io.tima();
                let tac = core.bus.io.tac();
                return Outcome::Fail {
                    cycles,
                    serial,
                    pc,
                    tima,
                    tac,
                };
            }
        }

        if cycles >= MAX_CYCLES {
            break;
        }
    }

    let pc = Scalar::to_u16(core.cpu.pc);
    let tima = core.bus.io.tima();
    let tac = core.bus.io.tac();
    let (div, _, _, _) = core.timers.debug_state();
    let regs = [
        Scalar::to_u8(core.cpu.b),
        Scalar::to_u8(core.cpu.c),
        Scalar::to_u8(core.cpu.d),
        Scalar::to_u8(core.cpu.e),
        Scalar::to_u8(core.cpu.h),
        Scalar::to_u8(core.cpu.l),
    ];
    if std::env::var_os("GBX_DUMP_TIMER_STATE").is_some() {
        let dump_addr = 0x0000usize;
        let dump_len = 128;
        if dump_addr + dump_len <= core.bus.wram.len() {
            let window = &core.bus.wram[dump_addr..dump_addr + dump_len];
            eprintln!(
                "WRAM[C{:04X}..]= {}",
                dump_addr + 0xC000,
                window
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    Outcome::Timeout {
        cycles,
        serial_len: core.bus.serial_out.len(),
        pc,
        tima,
        tac,
        div,
        regs,
    }
}

fn assert_mooneye(path: &str) {
    match run_mooneye(path) {
        Outcome::Pass { cycles, serial_len } => {
            eprintln!(
                "{} passed after {} cycles (serial bytes: {})",
                path, cycles, serial_len
            );
        }
        Outcome::Fail {
            cycles,
            serial,
            pc,
            tima,
            tac,
        } => {
            panic!(
                "Mooneye ROM {} reported failure after {} cycles (serial tail {:02X?}, pc={:#06X}, tima={:02X}, tac={:02X})",
                path, cycles, serial, pc, tima, tac
            );
        }
        Outcome::Timeout {
            cycles,
            serial_len,
            pc,
            tima,
            tac,
            div,
            regs,
        } => {
            panic!(
                "Mooneye ROM {} timed out after {} cycles (serial bytes: {}, pc={:#06X}, tima={:02X}, tac={:02X}, div={:04X}, regs={:02X?})",
                path, cycles, serial_len, pc, tima, tac, div, regs
            );
        }
    }
}

#[test]
fn mooneye_timer_suite() {
    const ROMS: &[&str] = &[
        "mooneye-test-suite/acceptance/timer/div_write.gb",
        "mooneye-test-suite/acceptance/timer/rapid_toggle.gb",
        "mooneye-test-suite/acceptance/timer/tim00.gb",
        "mooneye-test-suite/acceptance/timer/tim00_div_trigger.gb",
        "mooneye-test-suite/acceptance/timer/tim01.gb",
        "mooneye-test-suite/acceptance/timer/tim01_div_trigger.gb",
        "mooneye-test-suite/acceptance/timer/tim10.gb",
        "mooneye-test-suite/acceptance/timer/tim10_div_trigger.gb",
        "mooneye-test-suite/acceptance/timer/tim11.gb",
        "mooneye-test-suite/acceptance/timer/tim11_div_trigger.gb",
        "mooneye-test-suite/acceptance/timer/tima_reload.gb",
        "mooneye-test-suite/acceptance/timer/tima_write_reloading.gb",
        "mooneye-test-suite/acceptance/timer/tma_write_reloading.gb",
    ];

    for &rom in ROMS {
        assert_mooneye(rom);
    }
}

#[test]
#[ignore = "Mooneye timer suite (wilbertpol fork) under bring-up"]
fn mooneye_timer_suite_wilbertpol() {
    const ROMS: &[&str] = &[
        "mooneye-test-suite-wilbertpol/acceptance/timer/div_write.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/rapid_toggle.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim00.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim00_div_trigger.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim01.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim01_div_trigger.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim10.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim10_div_trigger.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim11.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tim11_div_trigger.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tima_reload.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tima_write_reloading.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/timer_if.gb",
        "mooneye-test-suite-wilbertpol/acceptance/timer/tma_write_reloading.gb",
    ];

    for &rom in ROMS {
        assert_mooneye(rom);
    }
}

#[test]
#[ignore = "Mooneye interrupt suite under bring-up"]
fn mooneye_interrupt_suite() {
    const ROMS: &[&str] = &["mooneye-test-suite/acceptance/interrupts/ie_push.gb"];
    for &rom in ROMS {
        assert_mooneye(rom);
    }
}
