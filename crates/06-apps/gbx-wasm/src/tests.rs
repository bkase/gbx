//! Browser-focused wasm-bindgen tests exercising the SharedArrayBuffer transport.

use gloo_timers::future::TimeoutFuture;
use inspector_vm::InspectorVM;
use runtime_web_harness::{
    verify_backpressure_run, verify_burst_run, verify_flood_run, BackpressureScenario,
    BurstScenario, TransportHarness,
};
use service_abi::{CpuVM, DebugRep, InspectorVMMinimal, MemSpace, PpuVM, StepKind, TimersVM};
use wasm_bindgen::prelude::*;

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(JsValue::from_str(&format!($($arg)*)));
        }
    };
}

#[wasm_bindgen]
pub fn wasm_smoke_test() -> Result<(), JsValue> {
    let sum: i32 = [1, 1].iter().copied().sum();
    ensure!(sum == 2, "smoke test sum mismatch: {}", sum);
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_flood_frames() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let ticket = harness.start_flood(10_000)?;
    let outcome = harness.consume_frames(10_000, None).await?;
    let result = ticket.wait().await?;
    ensure!(
        result.status == 0,
        "worker flood result status {}",
        result.status
    );
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing flood stats"))?;
    verify_flood_run(&outcome, &stats, 10_000)?;
    harness.assert_reconciliation()?;
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_burst_fairness() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let config = BurstScenario {
        bursts: 40,
        burst_size: 64,
        drain_budget: 8,
    };
    let ticket = harness.start_burst(&config)?;
    let outcome = harness
        .consume_frames(
            (config.bursts * config.burst_size) as usize,
            Some(config.drain_budget as usize),
        )
        .await?;
    let result = ticket.wait().await?;
    ensure!(result.status == 0, "burst status {}", result.status);
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing burst stats"))?;
    verify_burst_run(&outcome, &stats, &config, harness.frame_slot_count)?;
    harness.assert_reconciliation()?;
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_transport_worker_backpressure_recovery() -> Result<(), JsValue> {
    let mut harness = TransportHarness::new().await?;
    let cfg = BackpressureScenario {
        frames: 4_096,
        pause_ms: 25,
    };
    let ticket = harness.start_backpressure(&cfg)?;
    TimeoutFuture::new(cfg.pause_ms).await;
    let outcome = harness.consume_frames(cfg.frames as usize, None).await?;
    harness.assert_reconciliation()?;
    let result = ticket.wait().await?;
    ensure!(result.status == 0, "backpressure status {}", result.status);
    let stats = result
        .stats
        .ok_or_else(|| JsValue::from_str("missing backpressure stats"))?;
    verify_backpressure_run(&outcome, &stats, cfg.frames)?;
    Ok(())
}

#[wasm_bindgen]
pub async fn wasm_inspector_debug_smoke() -> Result<(), JsValue> {
    let mut vm = InspectorVM::default();
    let snapshot = DebugRep::Snapshot(InspectorVMMinimal {
        cpu: CpuVM {
            a: 0x12,
            f: 0xB0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0xC000,
            pc: 0x0100,
            ime: true,
            halted: false,
        },
        ppu: PpuVM {
            ly: 0,
            mode: 1,
            stat: 0x85,
            lcdc: 0x91,
            scx: 0,
            scy: 0,
            wy: 0,
            wx: 0,
            bgp: 0xFC,
            frame_ready: false,
        },
        timers: TimersVM {
            div: 0x12,
            tima: 0x34,
            tma: 0x56,
            tac: 0x05,
        },
        io: vec![0; 0x80],
    });
    vm.apply_debug_rep(&snapshot);
    ensure!(vm.cpu.pc == 0x0100, "Snapshot PC mismatch: {}", vm.cpu.pc);

    let mem = DebugRep::MemWindow {
        space: MemSpace::Vram,
        base: 0x8000,
        bytes: vec![1, 2, 3, 4].into(),
    };
    vm.apply_debug_rep(&mem);
    ensure!(
        vm.mem.vram_window.as_ref().map(|w| w.bytes.as_slice()) == Some(&[1, 2, 3, 4][..]),
        "VRAM window mismatch"
    );

    let stepped = DebugRep::Stepped {
        kind: StepKind::Instruction,
        cycles: 8,
        pc: 0x0102,
        disasm: Some("NOP".into()),
    };
    vm.apply_debug_rep(&stepped);
    ensure!(
        vm.disasm
            .as_ref()
            .map(|trace| (trace.last_pc, trace.disasm_line.as_str()))
            == Some((0x0102, "NOP")),
        "Trace data missing"
    );

    let line = vm
        .to_ndjson_line()
        .map_err(|err: serde_json::Error| JsValue::from_str(&err.to_string()))?;
    ensure!(line.contains("\"pc\":256"), "NDJSON missing PC field");

    Ok(())
}
