use super::{instance::AnyCore, KernelFarm, KernelService};
use kernel_core::bus::IoRegs;
use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use kernel_core::CoreConfig;
use service_abi::{
    DebugCmd, DebugRep, FrameSpan, KernelCmd, KernelRep, KernelServiceHandle, MemSpace, StepKind,
    SubmitOutcome, TickPurpose,
};
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use testdata::bytes as test_rom_bytes;
use transport::{SlotPool, SlotPoolConfig, SlotPoolHandle, SlotPop};

const DMG_FRAME_BYTES: usize = 160 * 144 * 4;

fn take_frame_pixels(pool: &Arc<SlotPoolHandle>, span: &FrameSpan, expected_len: usize) -> Vec<u8> {
    if !span.pixels.is_empty() {
        return span.pixels.as_ref().to_vec();
    }

    let Some(slot_span) = span.slot_span.as_ref() else {
        return Vec::new();
    };

    let mut pixels = vec![0u8; expected_len];
    let mut written = 0usize;

    pool.with_mut(|p| {
        let mut consumed = Vec::with_capacity(slot_span.count as usize);
        for _ in 0..slot_span.count {
            match p.pop_ready() {
                SlotPop::Ok { slot_idx } => {
                    let slot = p.slot_mut(slot_idx);
                    let take = expected_len.saturating_sub(written).min(slot.len());
                    let end = written + take;
                    pixels[written..end].copy_from_slice(&slot[..take]);
                    written = end;
                    consumed.push(slot_idx);
                }
                SlotPop::Empty => panic!("expected ready slot for frame span"),
            }
        }

        for idx in consumed {
            p.release_free(idx);
        }
    });

    pixels.truncate(written);
    pixels
}

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
fn frame_reports_slot_span_when_ready() {
    let slot_size = DMG_FRAME_BYTES;
    let pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 2,
            slot_size,
        })
        .expect("slot pool"),
    ));

    let service = KernelService::with_frame_pool(8, Arc::clone(&pool), CoreConfig::default());
    let rom = Arc::<[u8]>::from(vec![0x00u8; 0x8000].into_boxed_slice());

    let load_cmd = KernelCmd::LoadRom {
        group: 7,
        bytes: Arc::clone(&rom),
    };
    assert_eq!(service.try_submit(&load_cmd), SubmitOutcome::Accepted);
    collect_reports(service.drain(4));

    let tick_cmd = KernelCmd::Tick {
        group: 7,
        purpose: TickPurpose::Display,
        budget: CYCLES_PER_FRAME,
    };
    assert_eq!(service.try_submit(&tick_cmd), SubmitOutcome::Accepted);

    let reports = collect_reports(service.drain(8));
    let span = reports
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::LaneFrame { span, .. } => Some(span),
            _ => None,
        })
        .expect("expected lane frame");

    assert!(
        span.pixels.is_empty(),
        "zero-copy span should embed no pixels"
    );
    let slot_span = span
        .slot_span
        .as_ref()
        .expect("zero-copy frame should carry slot span");
    assert_eq!(slot_span.count, 1, "frame should occupy a single slot");

    let popped = pool.with_mut(|p| match p.pop_ready() {
        SlotPop::Ok { slot_idx } => {
            p.release_free(slot_idx);
            slot_idx
        }
        SlotPop::Empty => panic!("expected ready slot"),
    });
    assert_eq!(
        popped, slot_span.start_idx,
        "slot span start should match ready slot"
    );
}

#[test]
fn backpressure_prevents_frame_without_slot() {
    let pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 1,
            slot_size: DMG_FRAME_BYTES,
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

    service.try_submit(&KernelCmd::Debug(DebugCmd::Snapshot { group }));
    let pc_before = drain_debug(&service, 4)
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::Snapshot(snapshot)) => Some(snapshot.cpu.pc),
            _ => None,
        })
        .expect("snapshot after load");

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
    assert_eq!(
        stepped.1,
        pc_before.wrapping_add(1),
        "PC should advance by one instruction"
    );

    // Snapshot after stepping to confirm state was committed.
    service.try_submit(&KernelCmd::Debug(DebugCmd::Snapshot { group }));
    let snapshot = drain_debug(&service, 4)
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(DebugRep::Snapshot(s)) => Some(s),
            _ => None,
        })
        .expect("snapshot after step");
    assert_eq!(snapshot.cpu.pc, pc_before.wrapping_add(1));
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

fn optional_tetris_rom() -> Option<Arc<[u8]>> {
    if let Ok(path) = env::var("GBX_TETRIS_ROM") {
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    eprintln!("GBX_TETRIS_ROM at {:?} is empty; skipping test", path);
                    return None;
                }
                return Some(Arc::from(bytes.into_boxed_slice()));
            }
            Err(err) => {
                eprintln!(
                    "failed to load tetris ROM from GBX_TETRIS_ROM={:?}: {}; skipping test",
                    path, err
                );
                return None;
            }
        }
    }

    let default_path = Path::new("third_party/testroms/tetris.gb");
    if !default_path.exists() {
        eprintln!(
            "tetris ROM not found at {:?}; skipping tetris-specific test",
            default_path
        );
        return None;
    }

    match fs::read(default_path) {
        Ok(bytes) => Some(Arc::from(bytes.into_boxed_slice())),
        Err(err) => {
            eprintln!(
                "failed to read tetris ROM at {:?}: {}; skipping test",
                default_path, err
            );
            None
        }
    }
}

#[test]
fn default_rom_matches_blargg_bundle() {
    let frame_pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 4,
            slot_size: 160 * 144 * 4,
        })
        .expect("slot pool"),
    ));

    let mut farm = KernelFarm::new(frame_pool, CoreConfig::default());
    let inst = farm.ensure_instance(0);
    let expected = test_rom_bytes(super::DEFAULT_ROM_PATH);
    let expected_ref: &[u8] = expected.as_ref();
    let actual = match &inst.core {
        AnyCore::Scalar(core) => core.bus.rom.as_ref(),
        AnyCore::Simd2(core) => core.bus.lane(0).rom.as_ref(),
        AnyCore::Simd4(core) => core.bus.lane(0).rom.as_ref(),
    };

    assert_eq!(
        actual.len(),
        expected_ref.len(),
        "default ROM length differs"
    );
    assert_eq!(
        actual, expected_ref,
        "default ROM contents differ from expected bundle"
    );
}

#[test]
fn tetris_rom_produces_multiple_frames() {
    let frame_pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 8,
            slot_size: 160 * 144 * 4,
        })
        .expect("slot pool"),
    ));

    let Some(rom) = optional_tetris_rom() else {
        return;
    };

    let mut farm = KernelFarm::new(frame_pool, CoreConfig::default());
    let _ = farm.load_rom(0, Arc::clone(&rom));

    const MAX_TICKS: usize = 2048;
    const TARGET_FRAMES_WITH_BOOT: usize = 120;
    const TARGET_NON_WHITE_WITH_BOOT: usize = 60;

    let boot_rom_present = {
        let inst = farm.ensure_instance(0);
        inst.boot_rom_enabled()
    };
    let target_frames = if boot_rom_present {
        TARGET_FRAMES_WITH_BOOT
    } else {
        1
    };
    let target_non_white_frames = if boot_rom_present {
        TARGET_NON_WHITE_WITH_BOOT
    } else {
        1
    };

    let mut total_frames = 0usize;
    let mut lcd_on_ticks = 0usize;
    let mut lcd_off_ticks = 0usize;
    let mut non_white_frames = 0usize;
    let mut ticks_run = 0usize;
    let mut boot_rom_disabled = false;
    let mut diff_frame_after_boot = false;
    let mut boot_frame_pixels: Option<Vec<u8>> = None;
    let mut last_overlay_enabled = farm.ensure_instance(0).boot_rom_enabled();

    for _ in 0..MAX_TICKS {
        let mut reports = Vec::new();
        farm.tick(0, CYCLES_PER_FRAME, &mut reports);
        let (overlay_enabled, lcdc) = {
            let inst = farm.ensure_instance(0);
            let lcdc = match &inst.core {
                AnyCore::Scalar(core) => core.bus.io.read(IoRegs::LCDC),
                AnyCore::Simd2(core) => core.bus.lane(0).io.read(IoRegs::LCDC),
                AnyCore::Simd4(core) => core.bus.lane(0).io.read(IoRegs::LCDC),
            };
            (inst.boot_rom_enabled(), lcdc)
        };

        for rep in reports.into_iter() {
            if let KernelRep::LaneFrame { span, .. } = rep {
                total_frames += 1;
                let pixels = span.pixels.as_ref();
                if pixels
                    .chunks_exact(4)
                    .any(|px| px[0] != 0xFF || px[1] != 0xFF || px[2] != 0xFF)
                {
                    non_white_frames += 1;
                }

                if overlay_enabled {
                    if boot_frame_pixels.is_none() {
                        boot_frame_pixels = Some(pixels.to_vec());
                    }
                } else if total_frames > 360 {
                    if let Some(boot_pixels) = boot_frame_pixels.as_ref() {
                        if !diff_frame_after_boot
                            && boot_pixels.len() == pixels.len()
                            && boot_pixels.iter().zip(pixels.iter()).any(|(a, b)| a != b)
                        {
                            diff_frame_after_boot = true;
                        }
                    }
                }
            }
        }

        if (lcdc & 0x80) != 0 {
            lcd_on_ticks += 1;
        } else {
            lcd_off_ticks += 1;
        }

        ticks_run += 1;
        if last_overlay_enabled && !overlay_enabled {
            boot_rom_disabled = true;
        }
        last_overlay_enabled = overlay_enabled;
        if total_frames >= target_frames && non_white_frames >= target_non_white_frames {
            break;
        }
    }

    assert!(
        total_frames >= target_frames,
        "expected Tetris ROM to produce at least {} frame(s), saw {} after {} ticks; lcd_on_ticks={} lcd_off_ticks={}",
        target_frames,
        total_frames,
        ticks_run,
        lcd_on_ticks,
        lcd_off_ticks
    );

    assert!(
        non_white_frames >= target_non_white_frames,
        "expected at least {} non-white frame(s), saw {} after {} ticks",
        target_non_white_frames,
        non_white_frames,
        ticks_run
    );

    if boot_rom_present {
        assert!(boot_rom_disabled, "expected boot ROM to disable itself");
        assert!(
            diff_frame_after_boot,
            "expected to observe a frame that differs from the boot logo after the boot ROM completes"
        );
    }
}

#[test]
fn tetris_rom_advances_without_boot_rom() {
    let _guard = EnvVarGuard::set("GBX_BOOT_ROM_DMG", "/nonexistent");

    let frame_pool = Arc::new(SlotPoolHandle::new(
        SlotPool::new(SlotPoolConfig {
            slot_count: 8,
            slot_size: DMG_FRAME_BYTES,
        })
        .expect("slot pool"),
    ));

    let mut farm = KernelFarm::new(Arc::clone(&frame_pool), CoreConfig::default());
    let rom =
        Arc::<[u8]>::from(include_bytes!("../../../../third_party/testroms/tetris.gb").as_slice());
    let _ = farm.load_rom(0, Arc::clone(&rom));

    let mut total_frames = 0usize;
    let mut ticks_run = 0usize;
    let mut boot_rom_disabled = false;
    let mut diff_frame_after_boot = false;
    let mut boot_frame_pixels: Option<Vec<u8>> = None;
    let mut last_boot_active = farm.ensure_instance(0).boot_active();

    for _ in 0..4096 {
        let mut reports = Vec::new();
        farm.tick(0, CYCLES_PER_FRAME, &mut reports);

        let inst = farm.ensure_instance(0);
        let boot_enabled = inst.boot_active();

        for rep in reports.into_iter() {
            if let KernelRep::LaneFrame { span, .. } = rep {
                total_frames += 1;
                let pixels = take_frame_pixels(&frame_pool, &span, DMG_FRAME_BYTES);
                if boot_enabled {
                    if boot_frame_pixels.is_none() {
                        boot_frame_pixels = Some(pixels.clone());
                    }
                } else if total_frames > 360 {
                    if let Some(boot) = boot_frame_pixels.as_ref() {
                        if boot.len() == pixels.len()
                            && boot.iter().zip(pixels.iter()).any(|(a, b)| a != b)
                        {
                            diff_frame_after_boot = true;
                        }
                    }
                }
            }
        }

        if last_boot_active && !boot_enabled {
            boot_rom_disabled = true;
        }
        last_boot_active = boot_enabled;

        ticks_run += 1;
        if diff_frame_after_boot && total_frames >= 180 {
            break;
        }
    }

    assert!(
        boot_rom_disabled,
        "boot sequence should complete (ticks_run={} total_frames={})",
        ticks_run, total_frames
    );
    assert!(
        diff_frame_after_boot,
        "expected a frame different from the boot logo after {} ticks (produced {} frames)",
        ticks_run, total_frames
    );
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.as_ref() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}
