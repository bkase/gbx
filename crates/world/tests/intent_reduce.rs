//! Integration tests validating world intent reducer behavior.

use std::sync::Arc;
use world::{Intent, IntentReducer, KernelCmd, TickPurpose, WorkCmd, World};

/// PumpFrame should enqueue a display tick whose budget scales with the speed multiplier.
#[test]
fn pump_frame_emits_display_tick_with_scaled_budget() {
    let mut cases = vec![(1.0, 70_224u32), (1.5, 105_336u32)];

    for (speed, expected_budget) in cases.drain(..) {
        let mut world = World::new();
        world.speed = speed;

        let commands = world.reduce_intent(Intent::PumpFrame);
        assert_eq!(
            commands.len(),
            1,
            "pump frame should emit exactly one command"
        );

        match &commands[0] {
            WorkCmd::Kernel(KernelCmd::Tick {
                group,
                purpose,
                budget,
            }) => {
                assert_eq!(*group, 0, "display tick should target group 0");
                assert_eq!(
                    *purpose,
                    TickPurpose::Display,
                    "pump frame should always request a display tick"
                );
                assert_eq!(
                    *budget, expected_budget,
                    "speed multiplier {speed} should scale display budget"
                );
            }
            other => panic!("unexpected work command for pump frame: {other:?}"),
        }
    }
}

/// LoadRom should forward the group and payload into a kernel load command.
#[test]
fn load_rom_forwards_group_and_payload() {
    let mut world = World::new();
    let payload: Arc<[u8]> = Arc::from([1u8, 2, 3, 4]);

    let commands = world.reduce_intent(Intent::LoadRom {
        group: 2,
        bytes: Arc::clone(&payload),
    });

    assert_eq!(
        commands.len(),
        1,
        "load ROM should emit a single kernel command"
    );
    assert_eq!(
        commands[0],
        WorkCmd::Kernel(KernelCmd::LoadRom {
            group: 2,
            bytes: payload
        })
    );
}

/// Toggle, speed, and display lane intents mutate the world without emitting commands.
#[test]
fn stateful_intents_update_world_without_emitting_commands() {
    enum Expectation {
        Paused(bool),
        Speed(f32),
        DisplayLane(u16),
    }

    let cases = vec![
        (
            "toggle_pause",
            Intent::TogglePause,
            Expectation::Paused(true),
        ),
        (
            "set_speed_normal",
            Intent::SetSpeed(2.5),
            Expectation::Speed(2.5),
        ),
        (
            "set_speed_min_clamp",
            Intent::SetSpeed(0.01),
            Expectation::Speed(0.1),
        ),
        (
            "set_speed_max_clamp",
            Intent::SetSpeed(12.0),
            Expectation::Speed(10.0),
        ),
        (
            "select_display_lane",
            Intent::SelectDisplayLane(7),
            Expectation::DisplayLane(7),
        ),
    ];

    for (name, intent, expectation) in cases {
        let mut world = World::new();
        let commands = world.reduce_intent(intent);

        assert!(
            commands.is_empty(),
            "{name} should not emit work commands, got {commands:?}"
        );

        match expectation {
            Expectation::Paused(value) => {
                assert_eq!(world.paused, value, "{name} should flip paused flag");
            }
            Expectation::Speed(expected) => {
                let diff = (world.speed - expected).abs();
                assert!(
                    diff <= 1e-6,
                    "{name} should clamp speed to {expected}, found {}",
                    world.speed
                );
            }
            Expectation::DisplayLane(expected) => {
                assert_eq!(
                    world.display_lane, expected,
                    "{name} should update display lane"
                );
            }
        }
    }
}
