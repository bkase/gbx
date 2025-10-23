//! Pure report reducer implementation for the Wave B world state.

use crate::types::{AudioRep, AvCmd, FollowUps, GpuCmd, Intent, IntentPriority, KernelRep, Report};
use crate::world::World;

/// Trait for handling reports and producing follow-up actions.
pub trait ReportReducer {
    /// Processes a report and returns follow-up AV commands or intents.
    fn reduce_report(&mut self, report: Report) -> FollowUps;
}

impl ReportReducer for World {
    fn reduce_report(&mut self, report: Report) -> FollowUps {
        let mut follow_ups = FollowUps::new();

        match report {
            Report::Kernel(kernel_report) => match kernel_report {
                KernelRep::LaneFrame {
                    lane,
                    span,
                    frame_id,
                    ..
                } => {
                    if lane == self.display_lane {
                        follow_ups
                            .push_immediate_av(AvCmd::Gpu(GpuCmd::UploadFrame { lane, span }));
                    }
                    self.record_present(frame_id);
                }
                KernelRep::TickDone { .. } => {
                    if self.auto_pump {
                        follow_ups.push_deferred_intent(IntentPriority::P1, Intent::PumpFrame);
                    }
                }
                KernelRep::RomLoaded { .. } => {
                    self.rom_loaded = true;
                    self.rom_events = self.rom_events.saturating_add(1);
                }
                KernelRep::Debug(debug) => {
                    self.inspector.apply_debug_rep(&debug);
                }
                _ => {}
            },
            Report::Audio(audio_report) => match audio_report {
                AudioRep::Underrun => {
                    self.record_audio_underrun();
                }
                AudioRep::Played { .. } => {}
            },
            Report::Gpu(_) | Report::Fs(_) => {}
        }

        self.inspector.sync_perf(&self.perf);

        follow_ups
    }
}
