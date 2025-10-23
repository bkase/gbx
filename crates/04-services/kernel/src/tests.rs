use super::KernelService;
use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use kernel_core::CoreConfig;
use service_abi::{
    DebugCmd, DebugRep, KernelCmd, KernelRep, KernelServiceHandle, MemSpace, StepKind,
    SubmitOutcome, TickPurpose,
};
use std::sync::Arc;
use transport::{SlotPool, SlotPoolConfig, SlotPoolHandle};

fn collect_reports(reports: impl IntoIterator<Item = KernelRep>) -> Vec<KernelRep> {
    reports.into_iter().collect()
}

fn load_blank_rom(service: &KernelServiceHandle, group: u16) {
    let rom = Arc::<[u8]>::from(vec![0x00u8; 0x8000].into_boxed_slice());
    let load_cmd = KernelCmd::LoadRom {
        group,
        bytes: Arc::clone(&rom),
    };
    assert_eq!(service.try_submit(&load_cmd), SubmitOutcome::Accepted);
    let reports = collect_reports(service.drain(4));
    assert!(
        reports
            .into_iter()
            .any(|rep| matches!(rep, KernelRep::RomLoaded { group: g, .. } if g == group)),
        "expected RomLoaded report"
    );
}

fn drain_debug(service: &KernelServiceHandle, budget: usize) -> Vec<KernelRep> {
    collect_reports(service.drain(budget))
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

#[test]
fn debug_snapshot_reports_initial_cpu_state() {
    let service = KernelService::new_handle(8);
    let group = 3;
    load_blank_rom(&service, group);

    let outcome = service.try_submit(&KernelCmd::Debug(DebugCmd::Snapshot { group }));
    assert_eq!(outcome, SubmitOutcome::Accepted);

    let reports = drain_debug(&service, 4);
    let snapshot = reports
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::Snapshot(s)) => Some(s),
            _ => None,
        })
        .expect("snapshot report");

    assert_eq!(snapshot.cpu.pc, 0x0100, "PC should initialise to 0x0100");
    assert_eq!(snapshot.cpu.sp, 0xFFFE, "SP should initialise to 0xFFFE");
    assert_eq!(snapshot.io.len(), 0x80, "IO window should be 0x80 bytes");
}

#[test]
fn debug_mem_window_respects_requested_length() {
    let service = KernelService::new_handle(8);
    let group = 4;
    load_blank_rom(&service, group);

    let outcome = service.try_submit(&KernelCmd::Debug(DebugCmd::MemWindow {
        group,
        space: MemSpace::Vram,
        base: 0x8000,
        len: 0x20,
    }));
    assert_eq!(outcome, SubmitOutcome::Accepted);

    let reports = drain_debug(&service, 4);
    let window = reports
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::MemWindow { bytes, .. }) => Some(bytes),
            _ => None,
        })
        .expect("mem window report");

    assert_eq!(window.len(), 0x20);
}

#[test]
fn debug_step_instruction_advances_pc() {
    let service = KernelService::new_handle(8);
    let group = 5;
    load_blank_rom(&service, group);

    let outcome = service.try_submit(&KernelCmd::Debug(DebugCmd::StepInstruction {
        group,
        count: 1,
    }));
    assert_eq!(outcome, SubmitOutcome::Accepted);

    let reports = drain_debug(&service, 8);
    let stepped = reports
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::Stepped {
                kind: StepKind::Instruction,
                cycles,
                pc,
                ..
            }) => Some((cycles, pc)),
            _ => None,
        })
        .expect("stepped report");

    assert!(stepped.0 > 0, "instruction step should consume cycles");
    assert_eq!(stepped.1, 0x0101, "PC should advance by one instruction");

    // Snapshot after stepping to confirm state was committed.
    service.try_submit(&KernelCmd::Debug(DebugCmd::Snapshot { group }));
    let snapshot = drain_debug(&service, 4)
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::Snapshot(s)) => Some(s),
            _ => None,
        })
        .expect("snapshot after step");
    assert_eq!(snapshot.cpu.pc, 0x0101);
}

#[test]
fn debug_step_frame_produces_frame_and_tick() {
    let service = KernelService::new_handle(16);
    let group = 6;
    load_blank_rom(&service, group);

    let outcome = service.try_submit(&KernelCmd::Debug(DebugCmd::StepFrame { group }));
    assert_eq!(outcome, SubmitOutcome::Accepted);

    let reports = drain_debug(&service, 16);
    let mut saw_debug = false;
    let mut saw_tick_done = false;

    for rep in reports {
        match rep {
            KernelRep::Debug(DebugRep::Stepped { kind, cycles, .. }) => {
                if matches!(kind, StepKind::Frame) {
                    assert!(cycles > 0, "frame step should consume cycles");
                    saw_debug = true;
                }
            }
            KernelRep::TickDone {
                group: g,
                cycles_done,
                ..
            } if g == group => {
                assert!(cycles_done > 0);
                saw_tick_done = true;
            }
            KernelRep::LaneFrame { group: g, .. } if g == group => {
                // accept optional display frame
            }
            _ => {}
        }
    }

    assert!(saw_debug, "expected debug stepped frame report");
    assert!(saw_tick_done, "expected TickDone alongside frame step");
}
