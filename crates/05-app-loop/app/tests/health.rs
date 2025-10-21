//! Unit tests for stall relief behavior and health flag transitions.

use app::health::{Health, HealthFlags};

/// Beginning stall relief should latch GPU blocked and extend the window.
#[test]
fn begin_stall_relief_sets_flag_and_extends_window() {
    let mut health = Health::default();

    health.begin_stall_relief(5);
    assert!(health.flags.gpu_blocked);
    assert_eq!(health.stall_relief_frames, 5);

    health.begin_stall_relief(3);
    assert_eq!(
        health.stall_relief_frames, 5,
        "relief window should not shrink"
    );

    health.begin_stall_relief(8);
    assert_eq!(health.stall_relief_frames, 8);
}

/// Clearing after a successful submission unblocks GPU and decays once.
#[test]
fn clear_on_success_unlatches_gpu_block_and_decays_once() {
    let mut health = Health {
        flags: HealthFlags {
            gpu_blocked: true,
            service_pressure: false,
            fatal: false,
        },
        stall_relief_frames: 4,
    };

    health.clear_on_success();

    assert!(!health.flags.gpu_blocked);
    assert_eq!(health.stall_relief_frames, 3);
}

/// Decay should saturate at zero with or without a success path.
#[test]
fn decay_one_frame_saturates_at_zero() {
    let mut health = Health {
        flags: HealthFlags::default(),
        stall_relief_frames: 1,
    };

    health.decay_one_frame();
    assert_eq!(health.stall_relief_frames, 0);

    health.decay_one_frame();
    assert_eq!(health.stall_relief_frames, 0);

    health.clear_on_success();
    assert_eq!(health.stall_relief_frames, 0);
}
