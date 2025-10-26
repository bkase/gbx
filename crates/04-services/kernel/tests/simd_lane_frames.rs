//! Integration tests for multi-lane SIMD frame rendering and transport slot management.

use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use services_kernel::KernelFarm;
use transport::{SlotPool, SlotPoolConfig, SlotPoolHandle, SlotPop};

#[test]
fn simd8_emits_lane_frames_beyond_initial_tick() {
    let _ = env_logger::builder().is_test(true).try_init();

    let frame_pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 8,
            slot_size: 128 * 1024,
        })
        .expect("slot pool"),
    ));

    let core_config = kernel_core::CoreConfig {
        lanes: NonZeroUsize::new(8).expect("lanes > 0"),
        ..kernel_core::CoreConfig::default()
    };

    let mut farm = KernelFarm::new(Arc::clone(&frame_pool), core_config);
    let rom_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../third_party/testroms/tetris.gb");
    let rom_bytes = fs::read(rom_path).expect("read tetris rom");
    let rom = Arc::<[u8]>::from(rom_bytes.into_boxed_slice());
    farm.load_rom(0, Arc::clone(&rom));

    // First few ticks should eventually produce a lane frame (boot path).
    assert!(
        tick_until_lane_frame(&frame_pool, &mut farm, 0, 8),
        "expected a lane frame during initial boot ticks"
    );

    // Subsequent ticks should keep producing lane frames; regressions currently stop after one.
    assert!(
        tick_until_lane_frame(&frame_pool, &mut farm, 0, 16),
        "expected lane frames to continue after the first publication"
    );
}

fn tick_until_lane_frame(
    pool: &Arc<SlotPoolHandle>,
    farm: &mut KernelFarm,
    group: u16,
    attempts: usize,
) -> bool {
    let mut reports = Vec::new();
    for attempt in 0..attempts {
        reports.clear();
        farm.tick(group, CYCLES_PER_FRAME, &mut reports);
        println!("tick_attempt={attempt} reports={reports:?}");
        release_ready_slots(pool);
        if reports
            .iter()
            .any(|rep| matches!(rep, service_abi::KernelRep::LaneFrame { .. }))
        {
            return true;
        }
    }
    false
}

fn release_ready_slots(pool: &Arc<SlotPoolHandle>) {
    pool.with_mut(|slot_pool| {
        while let SlotPop::Ok { slot_idx } = slot_pool.pop_ready() {
            let sample = slot_pool.slot_mut(slot_idx);
            let nonzero = sample.iter().copied().step_by(4).any(|b| b != 0);
            println!(
                "slot_idx={} sample_rgba0={:?} has_content={}",
                slot_idx,
                &sample.get(0..8).unwrap_or(&[]),
                nonzero
            );
            slot_pool.release_free(slot_idx);
        }
    });
}
