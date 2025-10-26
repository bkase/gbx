//! Integration-style coverage for the world report reducer.

use service_abi::{CpuVM, DebugRep, InspectorVMMinimal, PpuVM, TimersVM};
use world::{
    AudioRep, AvCmd, FrameSpan, GpuCmd, Intent, IntentPriority, KernelRep, Report, ReportReducer,
    ViewMode, World,
};

/// Frames for the active display lane should be forwarded to the GPU immediately.
#[test]
fn kernel_display_lane_frame_emits_gpu_upload() {
    let mut world = World::new();
    let span = FrameSpan::default();

    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::LaneFrame {
        group: 0,
        lane: world.display_lane,
        span: span.clone(),
        frame_id: 42,
    }));

    assert_eq!(follow_ups.immediate_av.len(), 1);
    assert_eq!(
        follow_ups.immediate_av[0],
        AvCmd::Gpu(GpuCmd::UploadFrame {
            lane: world.display_lane,
            span
        })
    );
    assert!(follow_ups.deferred_intents.is_empty());
    assert_eq!(world.perf.last_frame_id, 42);
}

/// Grid mode should forward every lane, even when it differs from the display lane.
#[test]
fn grid_view_forwards_all_lanes() {
    let mut world = World::new();
    world.set_view_mode(ViewMode::Grid);

    let span = FrameSpan::default();
    let lane = world.display_lane + 3;
    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::LaneFrame {
        group: 0,
        lane,
        span: span.clone(),
        frame_id: 99,
    }));

    assert_eq!(follow_ups.immediate_av.len(), 1);
    assert_eq!(
        follow_ups.immediate_av[0],
        AvCmd::Gpu(GpuCmd::UploadFrame { lane, span })
    );
    assert!(follow_ups.deferred_intents.is_empty());
    assert_eq!(
        world.perf.last_frame_id, 0,
        "non-display lanes should not update primary frame tracker"
    );
}

/// Auto-pump worlds should enqueue an intent after each tick completes.
#[test]
fn tick_done_auto_pump_enqueues_pump_intent() {
    let mut world = World::new();

    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::TickDone {
        group: 0,
        lanes_mask: 0b1,
        cycles_done: 10,
    }));

    assert!(follow_ups.immediate_av.is_empty());
    assert_eq!(follow_ups.deferred_intents.len(), 1);
    assert_eq!(
        follow_ups.deferred_intents[0],
        (IntentPriority::P1, Intent::PumpFrame)
    );
}

/// Worlds without auto-pump should not enqueue intents on tick completion.
#[test]
fn tick_done_without_auto_pump_is_noop() {
    let mut world = World::new();
    world.auto_pump = false;

    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::TickDone {
        group: 0,
        lanes_mask: 0b1,
        cycles_done: 10,
    }));

    assert!(follow_ups.immediate_av.is_empty());
    assert!(follow_ups.deferred_intents.is_empty());
}

/// Successful ROM load reports should update world tracking flags.
#[test]
fn rom_loaded_updates_world_state() {
    let mut world = World::new();

    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::RomLoaded {
        group: 0,
        bytes_len: 16,
    }));

    assert!(follow_ups.immediate_av.is_empty());
    assert!(follow_ups.deferred_intents.is_empty());
    assert!(world.rom_loaded);
    assert_eq!(world.rom_events, 1);
}

/// Audio underruns should be reflected in the world's performance metrics.
#[test]
fn audio_underrun_increments_metric() {
    let mut world = World::new();

    let follow_ups = world.reduce_report(Report::Audio(AudioRep::Underrun));

    assert!(follow_ups.immediate_av.is_empty());
    assert!(follow_ups.deferred_intents.is_empty());
    assert_eq!(world.perf.audio_underruns, 1);
}

#[test]
fn debug_snapshot_report_updates_inspector_state() {
    let mut world = World::new();
    let snapshot = DebugRep::Snapshot(InspectorVMMinimal {
        cpu: CpuVM {
            a: 0x42,
            f: 0,
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0xFF00,
            pc: 0x0200,
            ime: false,
            halted: false,
        },
        ppu: PpuVM {
            ly: 0,
            mode: 0,
            stat: 0x85,
            lcdc: 0x91,
            scx: 0,
            scy: 0,
            wy: 0,
            wx: 0,
            bgp: 0,
            frame_ready: false,
        },
        timers: TimersVM {
            div: 0,
            tima: 0,
            tma: 0,
            tac: 0,
        },
        io: vec![0; 0x80],
    });

    let follow_ups = world.reduce_report(Report::Kernel(KernelRep::Debug(snapshot)));

    assert!(follow_ups.immediate_av.is_empty());
    assert_eq!(world.inspector.vm.cpu.a, 0x42);
    assert_eq!(world.inspector.vm.cpu.pc, 0x0200);
}
