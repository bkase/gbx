//! Blargg Game Boy CPU instruction acceptance tests executed against the scalar core.

use kernel_core::{BusScalar, Core, Exec, IoRegs, Scalar};
use testdata::{self, Expected};

const STEP_BUDGET: u32 = 1_000_000;
const MAX_CYCLES: u64 = 300_000_000;

fn run_serial_rom(path: &str, max_cycles: u64) -> (String, u64) {
    let max_cycles = std::env::var("GBX_SERIAL_MAX_CYCLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(max_cycles);
    let rom = testdata::bytes(path);
    let mut core = Core::<Scalar, BusScalar>::from_rom(rom);
    let mut serial = String::new();
    let mut elapsed = 0u64;
    let break_on_fail = std::env::var_os("GBX_BREAK_ON_TEST_FAIL").is_some();
    let step_budget = if break_on_fail { 1_024 } else { STEP_BUDGET };

    let mut saw_init = false;
    let mut saw_start = false;
    while elapsed < max_cycles {
        let remaining = (max_cycles - elapsed).min(u64::from(step_budget)) as u32;
        let consumed = core.step_cycles(remaining);
        if consumed == 0 {
            break;
        }
        elapsed += u64::from(consumed);

        let chunk = core.bus.take_serial();
        if !chunk.is_empty() {
            serial.push_str(&chunk);
        }

        if serial.contains("Passed") || serial.contains("Failed") {
            break;
        }
        if break_on_fail {
            let result_idx = usize::from(0xD800u16 - 0xC000u16);
            if let Some(&result_byte) = core.bus.wram.get(result_idx) {
                if !saw_init && result_byte == 0xFF {
                    saw_init = true;
                } else if saw_init && !saw_start && result_byte == 0 {
                    saw_start = true;
                } else if saw_start && result_byte != 0 {
                    eprintln!(
                        "GBX_BREAK_ON_TEST_FAIL: result={:02X} cycles={}",
                        result_byte, elapsed
                    );
                    break;
                }
            }
        }
    }

    let tail = core.bus.take_serial();
    if !tail.is_empty() {
        serial.push_str(&tail);
    }

    if std::env::var_os("GBX_DUMP_SERIAL_STATE").is_some() {
        let cpu = &core.cpu;
        let status = if serial.contains("Passed") {
            "passed"
        } else if serial.contains("Failed") {
            "failed"
        } else {
            "in-progress"
        };
        let pc = Scalar::to_u16(cpu.pc);
        let ie = core.bus.ie;
        let if_reg = core.bus.io.if_reg();
        let tima = core.bus.io.read(IoRegs::TIMA);
        let tac = core.bus.io.read(IoRegs::TAC);
        let lcdc = core.bus.io.read(IoRegs::LCDC);
        let stat = core.bus.io.read(IoRegs::STAT);
        eprintln!(
            "GBX_DUMP_SERIAL_STATE: cycles={} status={} pc={:#06X} a={:02X} f={:02X} bc={:#06X} de={:#06X} hl={:#06X} sp={:#06X} ime={} halted={} ie={:02X} if={:02X} tima={:02X} tac={:02X} lcdc={:02X} stat={:02X}",
            elapsed,
            status,
            pc,
            Scalar::to_u8(cpu.a),
            cpu.f.to_byte(),
            Scalar::to_u16(cpu.bc()),
            Scalar::to_u16(cpu.de()),
            Scalar::to_u16(cpu.hl()),
            Scalar::to_u16(cpu.sp),
            cpu.ime,
            cpu.halted,
            ie,
            if_reg,
            tima,
            tac,
            lcdc,
            stat,
        );
        if (0xC000..=0xDFFF).contains(&pc) {
            let offset = usize::from(pc - 0xC000);
            let start = offset.saturating_sub(16);
            let end = (offset + 16).min(core.bus.wram.len());
            eprintln!("WRAM @pc:");
            for chunk in (start..end).step_by(8) {
                let addr = 0xC000u16 + chunk as u16;
                let mut line = String::new();
                use std::fmt::Write as _;
                let _ = write!(line, "{addr:04X}: ");
                for idx in 0..8 {
                    if let Some(byte) = core.bus.wram.get(chunk + idx) {
                        let _ = write!(line, "{byte:02X} ");
                    }
                }
                eprintln!("{line}");
            }
        }
        let dbg_ptr_base = 0xD604u16;
        if let Some(slice) = core
            .bus
            .wram
            .get((dbg_ptr_base - 0xC000) as usize..)
            .map(|s| &s[..3])
        {
            eprintln!(
                "WRAM[D604..D606]={:02X} {:02X} {:02X}",
                slice[0], slice[1], slice[2]
            );
        }
        let test_state_base = 0xD800u16;
        if let Some(slice) = core
            .bus
            .wram
            .get((test_state_base - 0xC000) as usize..)
            .map(|s| &s[..3])
        {
            let test_ptr = u16::from(slice[1]) | (u16::from(slice[2]) << 8);
            eprintln!(
                "WRAM[D800..D802]={:02X} {:02X} {:02X}",
                slice[0], slice[1], slice[2]
            );
            if test_ptr != 0 {
                let offset = usize::from(test_ptr.saturating_sub(0xC000));
                if let Some(window) = core.bus.wram.get(offset..offset + 16) {
                    let printable: String = window
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| {
                            if (0x20..=0x7E).contains(&b) {
                                b as char
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    let bytes: Vec<String> = window
                        .iter()
                        .take_while(|&&b| b != 0)
                        .map(|&b| format!("{:02X}", b))
                        .collect();
                    eprintln!(
                        "test_name[{:04X}] ~ \"{}\" ({})",
                        test_ptr,
                        printable,
                        bytes.join(" ")
                    );
                }
            }
        }
    }

    (serial, elapsed)
}

fn assert_serial_passes(path: &str) {
    assert_serial_passes_within(path, MAX_CYCLES);
}

fn assert_serial_passes_within(path: &str, max_cycles: u64) {
    let meta =
        testdata::metadata(path).unwrap_or_else(|| panic!("missing metadata entry for {path}"));
    let expected = match meta.expected {
        Expected::SerialAscii(text) => text,
        other => panic!("ROM {path} does not declare serial ASCII expectation (found {other:?})"),
    };

    let (log, cycles) = run_serial_rom(path, max_cycles);
    assert!(
        !log.contains("Failed"),
        "ROM {path} reported failure over serial after {cycles} cycles: {log:?}"
    );
    assert!(
        log.contains(expected.trim_end()),
        "ROM {path} did not emit expected serial marker {expected:?} within {cycles} cycles; observed {log:?}"
    );
}

#[test]
fn blargg_cpu_instrs_emits_progress() {
    let (log, cycles) = run_serial_rom("blargg/cpu_instrs/cpu_instrs.gb", 5_000_000);
    assert!(
        !log.is_empty(),
        "expected serial output within {cycles} cycles"
    );
}

#[test]
#[ignore = "run on demand while bringing up CPU coverage"]
fn blargg_cpu_instrs_individual_suite() {
    const ROMS: &[(&str, u64)] = &[
        ("blargg/cpu_instrs/individual/01-special.gb", 40_000_000),
        ("blargg/cpu_instrs/individual/02-interrupts.gb", 60_000_000),
        ("blargg/cpu_instrs/individual/03-op sp,hl.gb", 40_000_000),
        ("blargg/cpu_instrs/individual/04-op r,imm.gb", 40_000_000),
        ("blargg/cpu_instrs/individual/05-op rp.gb", 40_000_000),
        ("blargg/cpu_instrs/individual/06-ld r,r.gb", 40_000_000),
        (
            "blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb",
            60_000_000,
        ),
        ("blargg/cpu_instrs/individual/08-misc instrs.gb", 60_000_000),
        ("blargg/cpu_instrs/individual/09-op r,r.gb", 40_000_000),
        ("blargg/cpu_instrs/individual/10-bit ops.gb", 60_000_000),
        ("blargg/cpu_instrs/individual/11-op a,(hl).gb", 60_000_000),
    ];

    for &(path, budget) in ROMS {
        let (log, cycles) = run_serial_rom(path, budget);
        assert!(
            !log.contains("Failed"),
            "ROM {path} reported failure after {cycles} cycles: {log:?}"
        );
        assert!(
            log.contains("Passed"),
            "ROM {path} did not emit 'Passed' within {budget} cycles; observed serial log: {log:?}"
        );
    }
}

#[test]
fn blargg_cpu_instrs_passes() {
    assert_serial_passes("blargg/cpu_instrs/cpu_instrs.gb");
}

#[test]
fn blargg_cpu_instrs_01_special_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/01-special.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_02_interrupts_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/02-interrupts.gb", 60_000_000);
}

#[test]
fn blargg_cpu_instrs_03_op_sp_hl_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/03-op sp,hl.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_04_op_r_imm_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/04-op r,imm.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_05_op_rp_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/05-op rp.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_06_ld_r_r_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/06-ld r,r.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_07_jr_jp_call_ret_rst_passes() {
    assert_serial_passes_within(
        "blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb",
        60_000_000,
    );
}

#[test]
fn blargg_cpu_instrs_08_misc_instrs_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/08-misc instrs.gb", 60_000_000);
}

#[test]
fn blargg_cpu_instrs_09_op_r_r_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/09-op r,r.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_10_bit_ops_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/10-bit ops.gb", 60_000_000);
}

#[test]
fn blargg_cpu_instrs_11_op_a_hl_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/11-op a,(hl).gb", 80_000_000);
}

#[test]
fn blargg_instr_timing_passes() {
    assert_serial_passes_within("blargg/instr_timing/instr_timing.gb", 80_000_000);
}

#[test]
fn blargg_mem_timing_passes() {
    assert_serial_passes_within("blargg/mem_timing/mem_timing.gb", 120_000_000);
}
