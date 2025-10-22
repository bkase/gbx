use super::KernelService;
use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use kernel_core::CoreConfig;
use service_abi::{KernelCmd, KernelRep, SubmitOutcome, TickPurpose};
use std::sync::Arc;
use transport::{SlotPool, SlotPoolConfig, SlotPoolHandle};

fn collect_reports(reports: impl IntoIterator<Item = KernelRep>) -> Vec<KernelRep> {
    reports.into_iter().collect()
}

#[test]
fn tick_produces_frame() {
    let service = KernelService::new_handle(8);
    let rom = Arc::<[u8]>::from(vec![0x00u8; 0x8000].into_boxed_slice());

    let load_cmd = KernelCmd::LoadRom {
        group: 1,
        bytes: Arc::clone(&rom),
    };
    assert_eq!(service.try_submit(&load_cmd), SubmitOutcome::Accepted);
    let reports = collect_reports(service.drain(4));
    assert!(matches!(reports.as_slice(), [KernelRep::RomLoaded { .. }]));

    let tick_cmd = KernelCmd::Tick {
        group: 1,
        purpose: TickPurpose::Display,
        budget: CYCLES_PER_FRAME,
    };
    assert_eq!(service.try_submit(&tick_cmd), SubmitOutcome::Accepted);
    let mut reports = collect_reports(service.drain(4));
    reports.sort_by_key(|rep| match rep {
        KernelRep::LaneFrame { .. } => 0,
        KernelRep::TickDone { .. } => 1,
        _ => 2,
    });
    assert!(matches!(reports[0], KernelRep::LaneFrame { .. }));
    assert!(matches!(reports[1], KernelRep::TickDone { .. }));
}

#[test]
fn backpressure_prevents_frame_without_slot() {
    let pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 1,
            slot_size: 160 * 144 * 4,
        })
        .expect("slot pool"),
    ));

    // Acquire the sole slot to simulate backpressure.
    let retained_slot = pool.with_mut(|p| p.try_acquire_free()).expect("slot 0");

    let service = KernelService::with_frame_pool(8, Arc::clone(&pool), CoreConfig::default());
    let rom = Arc::<[u8]>::from(vec![0x00u8; 0x8000].into_boxed_slice());
    let load_cmd = KernelCmd::LoadRom {
        group: 1,
        bytes: Arc::clone(&rom),
    };
    service.try_submit(&load_cmd);
    service.drain(4);

    let tick_cmd = KernelCmd::Tick {
        group: 1,
        purpose: TickPurpose::Display,
        budget: CYCLES_PER_FRAME,
    };
    service.try_submit(&tick_cmd);
    let reports = collect_reports(service.drain(4));
    assert!(reports
        .iter()
        .all(|rep| matches!(rep, KernelRep::TickDone { .. })));

    // Release the slot and ensure the next tick yields a frame.
    pool.with_mut(|p| p.release_free(retained_slot));
    let tick_cmd = KernelCmd::Tick {
        group: 1,
        purpose: TickPurpose::Display,
        budget: CYCLES_PER_FRAME,
    };
    service.try_submit(&tick_cmd);
    let reports = collect_reports(service.drain(4));
    assert!(reports
        .iter()
        .any(|rep| matches!(rep, KernelRep::LaneFrame { .. })));
}
