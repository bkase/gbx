#![feature(portable_simd)]
//! Blargg CPU acceptance tests executed against the SIMD core.

use kernel_core::SimdCore;
use std::sync::Arc;
use testdata::{self, Expected};

const LANES: usize = 4;
const STEP_BUDGET: u32 = 1_000_000;
const MAX_CYCLES: u64 = 150_000_000;

fn run_serial_rom(path: &str, max_cycles: u64) -> ([String; LANES], u64, [u16; LANES]) {
    let max_cycles = std::env::var("GBX_SERIAL_MAX_CYCLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(max_cycles);
    let rom = testdata::bytes(path);
    let mut core: SimdCore<LANES> = SimdCore::from_rom(Arc::clone(&rom));
    let mut serial: [String; LANES] = std::array::from_fn(|_| String::new());
    let mut elapsed = 0u64;

    while elapsed < max_cycles {
        let remaining = (max_cycles - elapsed).min(u64::from(STEP_BUDGET)) as u32;
        let consumed = core.step_cycles(remaining);
        if consumed == 0 {
            break;
        }
        elapsed += u64::from(consumed);

        for (lane, log) in serial.iter_mut().enumerate() {
            let chunk = core.bus.lane_mut(lane).take_serial();
            if !chunk.is_empty() {
                log.push_str(&chunk);
            }
        }

        let any_failed = serial.iter().any(|s| s.contains("Failed"));
        let all_done = serial.iter().all(|s| s.contains("Passed"));
        if any_failed || all_done {
            break;
        }
    }

    for (lane, log) in serial.iter_mut().enumerate() {
        let tail = core.bus.lane_mut(lane).take_serial();
        if !tail.is_empty() {
            log.push_str(&tail);
        }
    }

    let pcs = core.cpu.pc.to_array();

    (serial, elapsed, pcs)
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

    let (logs, cycles, pcs) = run_serial_rom(path, max_cycles);
    for (lane, log) in logs.iter().enumerate() {
        assert!(
            !log.contains("Failed"),
            "ROM {path} reported failure on lane {lane} after {cycles} cycles: {log:?}"
        );
        assert!(
            log.contains(expected.trim_end()),
            "ROM {path} did not emit expected serial marker {expected:?} on lane {lane} within {cycles} cycles; observed {log:?} pc={:#06X}",
            pcs[lane]
        );
    }
}

#[test]
#[ignore = "cpu_instrs aggregate suite still under implementation"]
fn blargg_cpu_instrs_simd_passes() {
    assert_serial_passes("blargg/cpu_instrs/cpu_instrs.gb");
}

#[test]
fn blargg_cpu_instrs_01_special_simd_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/01-special.gb", 40_000_000);
}

#[test]
fn blargg_cpu_instrs_02_interrupts_simd_passes() {
    assert_serial_passes_within("blargg/cpu_instrs/individual/02-interrupts.gb", 60_000_000);
}
