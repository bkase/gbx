//! Browser-focused wasm-bindgen tests exercising the SharedArrayBuffer transport.

use gloo_timers::future::TimeoutFuture;
use runtime_web_harness::{
    verify_backpressure_run, verify_burst_run, verify_flood_run, BackpressureScenario,
    BurstScenario, TransportHarness,
};
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
