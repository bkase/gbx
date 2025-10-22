//! Service hub orchestration and shared scheduling primitives.

use anyhow::{anyhow, Result};

pub use world::reduce_intent::IntentReducer;
pub use world::reduce_report::ReportReducer;
pub use world::{
    AudioCmd, AudioRep, AudioSpan, AvCmd, FollowUps, FrameSpan, FsCmd, FsRep, GpuCmd, GpuRep,
    Intent, IntentPriority, KernelCmd, KernelRep, Report, SlotSpan, SubmitOutcome, SubmitPolicy,
    TickPurpose, WorkCmd,
};

// Re-export Service trait and handle types from service-abi
pub use service_abi::{
    AudioServiceHandle, FsServiceHandle, GpuServiceHandle, KernelServiceHandle, Service,
};

/// Default budget for processing intents per scheduler tick.
pub const DEFAULT_INTENT_BUDGET: usize = 3;
/// Default budget for draining reports per scheduler tick.
pub const DEFAULT_REPORT_BUDGET: usize = 32;

/// Aggregates backend services and exposes scheduling helpers.
#[derive(Clone)]
pub struct ServicesHub {
    kernel: KernelServiceHandle,
    fs: FsServiceHandle,
    gpu: GpuServiceHandle,
    audio: AudioServiceHandle,
}

impl ServicesHub {
    /// Creates a new builder for constructing a hub.
    pub fn builder() -> ServicesHubBuilder {
        ServicesHubBuilder::new()
    }

    /// Attempts to submit a work command to the appropriate service.
    pub fn try_submit_work(&self, cmd: WorkCmd) -> SubmitOutcome {
        match &cmd {
            WorkCmd::Kernel(inner) => self.kernel.try_submit(inner),
            WorkCmd::Fs(inner) => self.fs.try_submit(inner),
        }
    }

    /// Attempts to submit an AV command to the appropriate service.
    pub fn try_submit_av(&self, cmd: AvCmd) -> SubmitOutcome {
        match &cmd {
            AvCmd::Gpu(inner) => self.gpu.try_submit(inner),
            AvCmd::Audio(inner) => self.audio.try_submit(inner),
        }
    }

    /// Drains reports across all services up to the provided budget.
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
                let drained = self.kernel.drain(remaining);
                if !drained.is_empty() {
                    remaining = remaining.saturating_sub(drained.len());
                    out.extend(drained.into_iter().map(Report::Kernel));
                    progressed = true;
                }
            }

            if remaining > 0 {
                let drained = self.fs.drain(remaining);
                if !drained.is_empty() {
                    remaining = remaining.saturating_sub(drained.len());
                    out.extend(drained.into_iter().map(Report::Fs));
                    progressed = true;
                }
            }

            if remaining > 0 {
                let drained = self.gpu.drain(remaining);
                if !drained.is_empty() {
                    remaining = remaining.saturating_sub(drained.len());
                    out.extend(drained.into_iter().map(Report::Gpu));
                    progressed = true;
                }
            }

            if remaining > 0 {
                let drained = self.audio.drain(remaining);
                if !drained.is_empty() {
                    remaining = remaining.saturating_sub(drained.len());
                    out.extend(drained.into_iter().map(Report::Audio));
                    progressed = true;
                }
            }
        }

        out
    }
}

/// Builder for assembling a [`ServicesHub`] from individual service handles.
pub struct ServicesHubBuilder {
    kernel: Option<KernelServiceHandle>,
    fs: Option<FsServiceHandle>,
    gpu: Option<GpuServiceHandle>,
    audio: Option<AudioServiceHandle>,
}

impl ServicesHubBuilder {
    /// Creates an empty builder with no services attached.
    pub fn new() -> Self {
        Self {
            kernel: None,
            fs: None,
            gpu: None,
            audio: None,
        }
    }

    /// Sets the kernel service handle.
    pub fn kernel(mut self, svc: KernelServiceHandle) -> Self {
        self.kernel = Some(svc);
        self
    }

    /// Sets the filesystem service handle.
    pub fn fs(mut self, svc: FsServiceHandle) -> Self {
        self.fs = Some(svc);
        self
    }

    /// Sets the GPU service handle.
    pub fn gpu(mut self, svc: GpuServiceHandle) -> Self {
        self.gpu = Some(svc);
        self
    }

    /// Sets the audio service handle.
    pub fn audio(mut self, svc: AudioServiceHandle) -> Self {
        self.audio = Some(svc);
        self
    }

    /// Builds a [`ServicesHub`], returning an error if any service is missing.
    pub fn build(self) -> Result<ServicesHub> {
        Ok(ServicesHub {
            kernel: self
                .kernel
                .ok_or_else(|| anyhow!("missing kernel service"))?,
            fs: self
                .fs
                .ok_or_else(|| anyhow!("missing filesystem service"))?,
            gpu: self.gpu.ok_or_else(|| anyhow!("missing GPU service"))?,
            audio: self.audio.ok_or_else(|| anyhow!("missing audio service"))?,
        })
    }
}

impl Default for ServicesHubBuilder {
    fn default() -> Self {
        Self::new()
    }
}
