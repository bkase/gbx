//! Blargg Game Boy CPU instruction acceptance tests executed against the scalar core.

use kernel_core::{Core, Exec, Scalar};
use testdata::{self, Expected};

const STEP_BUDGET: u32 = 1_000_000;
const MAX_CYCLES: u64 = 150_000_000;

fn run_serial_rom(path: &str, max_cycles: u64) -> (String, u64) {
    let max_cycles = std::env::var("GBX_SERIAL_MAX_CYCLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(max_cycles);
    let rom = testdata::bytes(path);
    let mut core: Core<Scalar, _> = Core::from_rom(rom);
    let mut serial = String::new();
    let mut elapsed = 0u64;

    while elapsed < max_cycles {
        let remaining = (max_cycles - elapsed).min(u64::from(STEP_BUDGET)) as u32;
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
    }

    let tail = core.bus.take_serial();
    if !tail.is_empty() {
        serial.push_str(&tail);
    }

    if std::env::var_os("GBX_DUMP_SERIAL_STATE").is_some() {
        let cpu = &core.cpu;
        eprintln!(
            "GBX_DUMP_SERIAL_STATE: cycles={} status={} pc={:#06X} a={:02X} f={:02X} bc={:#06X} de={:#06X} hl={:#06X} sp={:#06X} ime={} halted={}",
            elapsed,
            if serial.contains("Passed") {
                "passed"
            } else if serial.contains("Failed") {
                "failed"
            } else {
                "in-progress"
            },
            Scalar::to_u16(cpu.pc),
            Scalar::to_u8(cpu.a),
            cpu.f.to_byte(),
            Scalar::to_u16(cpu.bc()),
            Scalar::to_u16(cpu.de()),
            Scalar::to_u16(cpu.hl()),
            Scalar::to_u16(cpu.sp),
            cpu.ime,
            cpu.halted,
        );
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
#[ignore = "cpu_instrs aggregate suite still under implementation"]
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
