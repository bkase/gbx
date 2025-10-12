use anyhow::{anyhow, Result};
use smallvec::SmallVec;
use std::sync::Arc;

pub const DEFAULT_INTENT_BUDGET: usize = 3;
pub const DEFAULT_REPORT_BUDGET: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitPolicy {
    Must,
    Coalesce,
    BestEffort,
    Lossless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    Accepted,
    Coalesced,
    Dropped,
    WouldBlock,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickPurpose {
    Display,
    Exploration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentPriority {
    P0,
    P1,
    P2,
}

impl IntentPriority {
    pub fn index(self) -> usize {
        match self {
            IntentPriority::P0 => 0,
            IntentPriority::P1 => 1,
            IntentPriority::P2 => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Intent {
    PumpFrame,
    TogglePause,
    SetSpeed(f32),
    LoadRom { bytes: Arc<[u8]> },
    SelectDisplayLane(u16),
}

impl Intent {
    pub fn priority(&self) -> IntentPriority {
        match self {
            Intent::PumpFrame => IntentPriority::P1,
            Intent::TogglePause => IntentPriority::P0,
            Intent::SetSpeed(_) => IntentPriority::P0,
            Intent::LoadRom { .. } => IntentPriority::P0,
            Intent::SelectDisplayLane(_) => IntentPriority::P1,
        }
    }
}

#[derive(Clone, Debug)]
pub enum KernelCmd {
    Tick { purpose: TickPurpose, budget: u32 },
    LoadRom { bytes: Arc<[u8]> },
}

#[derive(Clone, Debug)]
pub enum FsCmd {
    Persist { key: String, payload: Arc<[u8]> },
}

#[derive(Clone, Debug)]
pub enum WorkCmd {
    Kernel(KernelCmd),
    Fs(FsCmd),
}

impl WorkCmd {
    pub fn default_policy(&self) -> SubmitPolicy {
        match self {
            WorkCmd::Kernel(KernelCmd::Tick { purpose, .. }) => match purpose {
                TickPurpose::Display => SubmitPolicy::Coalesce,
                TickPurpose::Exploration => SubmitPolicy::BestEffort,
            },
            WorkCmd::Kernel(KernelCmd::LoadRom { .. }) => SubmitPolicy::Lossless,
            WorkCmd::Fs(FsCmd::Persist { key, .. }) => {
                if key == "manual-save" {
                    SubmitPolicy::Lossless
                } else {
                    SubmitPolicy::Coalesce
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum GpuCmd {
    UploadFrame { lane: u16, frame_id: u64 },
}

#[derive(Clone, Debug)]
pub enum AudioCmd {
    SubmitSamples { frames: usize },
}

#[derive(Clone, Debug)]
pub enum AvCmd {
    Gpu(GpuCmd),
    Audio(AudioCmd),
}

impl AvCmd {
    pub fn default_policy(&self) -> SubmitPolicy {
        match self {
            AvCmd::Gpu(_) => SubmitPolicy::Must,
            AvCmd::Audio(_) => SubmitPolicy::Must,
        }
    }
}

#[derive(Clone, Debug)]
pub enum KernelRep {
    TickDone { purpose: TickPurpose, budget: u32 },
    LaneFrame { lane: u16, frame_id: u64 },
    RomLoaded { bytes_len: usize },
}

#[derive(Clone, Debug)]
pub enum FsRep {
    Saved { key: String, ok: bool },
}

#[derive(Clone, Debug)]
pub enum GpuRep {
    FramePresented { lane: u16, frame_id: u64 },
}

#[derive(Clone, Debug)]
pub enum AudioRep {
    Played { frames: usize },
    Underrun,
}

#[derive(Clone, Debug)]
pub enum Report {
    Kernel(KernelRep),
    Fs(FsRep),
    Gpu(GpuRep),
    Audio(AudioRep),
}

#[derive(Clone, Debug)]
pub struct FollowUps {
    pub immediate_av: SmallVec<[AvCmd; 8]>,
    pub deferred_intents: SmallVec<[(IntentPriority, Intent); 8]>,
}

impl FollowUps {
    pub fn new() -> Self {
        Self {
            immediate_av: SmallVec::new(),
            deferred_intents: SmallVec::new(),
        }
    }

    pub fn push_av(&mut self, cmd: AvCmd) {
        self.immediate_av.push(cmd);
    }

    pub fn push_deferred(&mut self, priority: IntentPriority, intent: Intent) {
        self.deferred_intents.push((priority, intent));
    }
}

impl Default for FollowUps {
    fn default() -> Self {
        Self::new()
    }
}

pub trait IntentReducer {
    fn reduce_intent(&mut self, intent: Intent) -> SmallVec<[WorkCmd; 8]>;
}

pub trait ReportReducer {
    fn reduce_report(&mut self, report: Report) -> FollowUps;
}

pub trait Service: Send + Sync {
    type Command: Send + 'static;
    type Report: Send + 'static;

    fn try_submit(&self, cmd: Self::Command, policy: SubmitPolicy) -> SubmitOutcome;
    fn try_poll_report(&self) -> Option<Self::Report>;
}

pub type KernelServiceHandle = Arc<dyn Service<Command = KernelCmd, Report = KernelRep>>;
pub type FsServiceHandle = Arc<dyn Service<Command = FsCmd, Report = FsRep>>;
pub type GpuServiceHandle = Arc<dyn Service<Command = GpuCmd, Report = GpuRep>>;
pub type AudioServiceHandle = Arc<dyn Service<Command = AudioCmd, Report = AudioRep>>;

#[derive(Clone)]
pub struct ServicesHub {
    kernel: KernelServiceHandle,
    fs: FsServiceHandle,
    gpu: GpuServiceHandle,
    audio: AudioServiceHandle,
}

impl ServicesHub {
    pub fn builder() -> ServicesHubBuilder {
        ServicesHubBuilder::new()
    }

    pub fn try_submit_work(&self, cmd: WorkCmd) -> SubmitOutcome {
        let policy = cmd.default_policy();
        match cmd {
            WorkCmd::Kernel(inner) => self.kernel.try_submit(inner, policy),
            WorkCmd::Fs(inner) => self.fs.try_submit(inner, policy),
        }
    }

    pub fn try_submit_av(&self, cmd: AvCmd) -> SubmitOutcome {
        let policy = cmd.default_policy();
        match cmd {
            AvCmd::Gpu(inner) => self.gpu.try_submit(inner, policy),
            AvCmd::Audio(inner) => self.audio.try_submit(inner, policy),
        }
    }

    pub fn drain_reports(&self, budget: usize) -> Vec<Report> {
        if budget == 0 {
            return Vec::new();
        }

        let mut remaining = budget;
        let mut out = Vec::with_capacity(budget);
        let mut progressed = true;

        while remaining > 0 && progressed {
            progressed = false;

            if remaining > 0 {
                if let Some(rep) = self.kernel.try_poll_report() {
                    out.push(Report::Kernel(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.fs.try_poll_report() {
                    out.push(Report::Fs(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.gpu.try_poll_report() {
                    out.push(Report::Gpu(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }

            if remaining > 0 {
                if let Some(rep) = self.audio.try_poll_report() {
                    out.push(Report::Audio(rep));
                    remaining -= 1;
                    progressed = true;
                }
            }
        }

        out
    }
}

pub struct ServicesHubBuilder {
    kernel: Option<KernelServiceHandle>,
    fs: Option<FsServiceHandle>,
    gpu: Option<GpuServiceHandle>,
    audio: Option<AudioServiceHandle>,
}

impl ServicesHubBuilder {
    pub fn new() -> Self {
        Self {
            kernel: None,
            fs: None,
            gpu: None,
            audio: None,
        }
    }

    pub fn kernel(mut self, svc: KernelServiceHandle) -> Self {
        self.kernel = Some(svc);
        self
    }

    pub fn fs(mut self, svc: FsServiceHandle) -> Self {
        self.fs = Some(svc);
        self
    }

    pub fn gpu(mut self, svc: GpuServiceHandle) -> Self {
        self.gpu = Some(svc);
        self
    }

    pub fn audio(mut self, svc: AudioServiceHandle) -> Self {
        self.audio = Some(svc);
        self
    }

    pub fn build(self) -> Result<ServicesHub> {
        Ok(ServicesHub {
            kernel: self
                .kernel
                .ok_or_else(|| anyhow!("missing kernel service"))?,
            fs: self.fs.ok_or_else(|| anyhow!("missing fs service"))?,
            gpu: self.gpu.ok_or_else(|| anyhow!("missing gpu service"))?,
            audio: self.audio.ok_or_else(|| anyhow!("missing audio service"))?,
        })
    }
}

impl Default for ServicesHubBuilder {
    fn default() -> Self {
        Self::new()
    }
}
