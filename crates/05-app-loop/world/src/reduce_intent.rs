//! Pure intent reducer implementation for the Wave B world state.

use crate::types::{DebugCmd, Intent, KernelCmd, TickPurpose, WorkCmd};
use crate::world::World;
use smallvec::{smallvec, SmallVec};

/// Trait for handling intents and producing work commands.
pub trait IntentReducer {
    /// Reduces an intent into zero or more work commands.
    fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]>;
}

impl IntentReducer for World {
    fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]> {
        match intent {
            Intent::PumpFrame => smallvec![WorkCmd::Kernel(KernelCmd::Tick {
                group: DISPLAY_GROUP,
                purpose: TickPurpose::Display,
                budget: display_cycle_budget(self.speed),
            })],
            Intent::LoadRom { group, bytes } => {
                smallvec![WorkCmd::Kernel(KernelCmd::LoadRom { group, bytes })]
            }
            Intent::TogglePause => {
                self.paused = !self.paused;
                SmallVec::new()
            }
            Intent::SetSpeed(multiplier) => {
                self.speed = clamp_speed(multiplier);
                SmallVec::new()
            }
            Intent::SelectDisplayLane(lane) => {
                self.display_lane = lane;
                SmallVec::new()
            }
            Intent::SetInputs { group, joypad } => {
                smallvec![WorkCmd::Kernel(KernelCmd::SetInputs {
                    group,
                    lanes_mask: 0xFFFF_FFFF,
                    joypad,
                })]
            }
            Intent::DebugSnapshot(group) => {
                smallvec![WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::Snapshot {
                    group
                }))]
            }
            Intent::DebugMem {
                group,
                space,
                base,
                len,
            } => smallvec![WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::MemWindow {
                group,
                space,
                base,
                len,
            }))],
            Intent::DebugStepInstruction { group, count } => {
                smallvec![WorkCmd::Kernel(KernelCmd::Debug(
                    DebugCmd::StepInstruction { group, count }
                ))]
            }
            Intent::DebugStepFrame(group) => {
                smallvec![WorkCmd::Kernel(KernelCmd::Debug(DebugCmd::StepFrame {
                    group
                }))]
            }
        }
    }
}

const DISPLAY_GROUP: u16 = 0;
const BASE_DISPLAY_CYCLES_PER_FRAME: f32 = 70_224.0;
const MIN_SPEED: f32 = 0.1;
const MAX_SPEED: f32 = 10.0;

fn display_cycle_budget(speed: f32) -> u32 {
    (BASE_DISPLAY_CYCLES_PER_FRAME * speed).round() as u32
}

fn clamp_speed(speed: f32) -> f32 {
    speed.clamp(MIN_SPEED, MAX_SPEED)
}
