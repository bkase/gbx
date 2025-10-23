//! Intent reducer coverage for debug inspector flows.

use service_abi::{DebugCmd, MemSpace};
use world::{Intent, IntentReducer, KernelCmd, WorkCmd, World};

fn single_cmd(intent: Intent) -> WorkCmd {
    let mut world = World::new();
    let mut cmds = world.reduce_intent(intent);
    assert_eq!(cmds.len(), 1, "expected exactly one command");
    cmds.pop().unwrap()
}

#[test]
fn debug_snapshot_intent_emits_kernel_command() {
    match single_cmd(Intent::DebugSnapshot(9)) {
        WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::Snapshot { group })) => {
            assert_eq!(group, 9);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn debug_mem_intent_emits_window_params() {
    match single_cmd(Intent::DebugMem {
        group: 1,
        space: MemSpace::Wram,
        base: 0xC120,
        len: 0x40,
    }) {
        WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::MemWindow {
            group,
            space,
            base,
            len,
        })) => {
            assert_eq!(group, 1);
            assert_eq!(space, MemSpace::Wram);
            assert_eq!(base, 0xC120);
            assert_eq!(len, 0x40);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn debug_step_instruction_uses_lossless_policy() {
    match single_cmd(Intent::DebugStepInstruction { group: 2, count: 3 }) {
        WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::StepInstruction { group, count })) => {
            assert_eq!(group, 2);
            assert_eq!(count, 3);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn debug_step_frame_targets_group() {
    match single_cmd(Intent::DebugStepFrame(4)) {
        WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::StepFrame { group })) => {
            assert_eq!(group, 4);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}
