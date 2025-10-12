use hub::{
    AudioRep, AvCmd, FollowUps, FsCmd, FsRep, GpuCmd, GpuRep, Intent, IntentPriority,
    IntentReducer, KernelCmd, KernelRep, Report, ReportReducer, TickPurpose, WorkCmd,
};
use smallvec::SmallVec;
use std::sync::Arc;

pub struct World {
    rom_loaded: bool,
    paused: bool,
    speed: f32,
    pub(crate) display_lane: u16,
    frame_id: u64,
    audio_underruns: u64,
    last_save_ok: bool,
    last_rom_len: usize,
    auto_pump: bool,
    rom_events: u32,
}

impl World {
    pub fn new() -> Self {
        Self {
            rom_loaded: false,
            paused: false,
            speed: 1.0,
            display_lane: 0,
            frame_id: 0,
            audio_underruns: 0,
            last_save_ok: true,
            last_rom_len: 0,
            auto_pump: true,
            rom_events: 0,
        }
    }

    pub fn rom_loaded(&self) -> bool {
        self.rom_loaded
    }

    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }

    pub fn rom_events(&self) -> u32 {
        self.rom_events
    }

    pub fn audio_underruns(&self) -> u64 {
        self.audio_underruns
    }

    pub fn last_save_ok(&self) -> bool {
        self.last_save_ok
    }

    pub fn auto_pump(&self) -> bool {
        self.auto_pump
    }

    pub fn set_auto_pump(&mut self, value: bool) {
        self.auto_pump = value;
    }

    fn display_tick_budget(&self) -> u32 {
        const BASE: f32 = 70_224.0;
        (BASE * self.speed).round() as u32
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl IntentReducer for World {
    fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]> {
        let mut work = SmallVec::new();
        match intent {
            Intent::PumpFrame => {
                if self.rom_loaded && !self.paused {
                    let budget = self.display_tick_budget();
                    work.push(WorkCmd::Kernel(KernelCmd::Tick {
                        purpose: TickPurpose::Display,
                        budget,
                    }));
                }
            }
            Intent::TogglePause => {
                self.paused = !self.paused;
            }
            Intent::SetSpeed(sp) => {
                self.speed = sp.clamp(0.1, 10.0);
            }
            Intent::LoadRom { bytes } => {
                self.rom_loaded = true;
                let payload = Arc::clone(&bytes);
                work.push(WorkCmd::Kernel(KernelCmd::LoadRom { bytes }));
                work.push(WorkCmd::Fs(FsCmd::Persist {
                    key: "manual-save".to_string(),
                    payload,
                }));
            }
            Intent::SelectDisplayLane(lane) => {
                self.display_lane = lane;
            }
        }
        work
    }
}

impl ReportReducer for World {
    fn reduce_report(&mut self, report: Report) -> FollowUps {
        let mut follow_ups = FollowUps::new();
        match report {
            Report::Kernel(rep) => match rep {
                KernelRep::TickDone { purpose, .. } => {
                    if self.auto_pump && matches!(purpose, TickPurpose::Display) {
                        follow_ups.push_deferred(IntentPriority::P1, Intent::PumpFrame);
                    }
                }
                KernelRep::LaneFrame { lane, frame_id } => {
                    if lane == self.display_lane {
                        self.frame_id = frame_id;
                        follow_ups.push_av(AvCmd::Gpu(GpuCmd::UploadFrame { lane, frame_id }));
                    }
                }
                KernelRep::RomLoaded { bytes_len } => {
                    self.last_rom_len = bytes_len;
                    self.rom_events = self.rom_events.saturating_add(1);
                }
            },
            Report::Fs(rep) => match rep {
                FsRep::Saved { key, ok } => {
                    if key == "manual-save" {
                        self.last_save_ok = ok;
                    }
                }
            },
            Report::Gpu(rep) => match rep {
                GpuRep::FramePresented { lane, frame_id } => {
                    if lane == self.display_lane {
                        self.frame_id = frame_id;
                    }
                }
            },
            Report::Audio(rep) => match rep {
                AudioRep::Played { .. } => {}
                AudioRep::Underrun => {
                    self.audio_underruns = self.audio_underruns.saturating_add(1);
                }
            },
        }
        follow_ups
    }
}
