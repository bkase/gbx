use hub::{GpuCmd, GpuRep, GpuServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

pub struct GpuService {
    reports: Mutex<VecDeque<GpuRep>>,
    capacity: usize,
}

impl GpuService {
    pub fn new_handle(capacity: usize) -> GpuServiceHandle {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        })
    }
}

impl Default for GpuService {
    fn default() -> Self {
        Self {
            reports: Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl Service for GpuService {
    type Command = GpuCmd;
    type Report = GpuRep;

    fn try_submit(&self, cmd: Self::Command, policy: SubmitPolicy) -> SubmitOutcome {
        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            return match policy {
                SubmitPolicy::BestEffort => SubmitOutcome::Dropped,
                SubmitPolicy::Coalesce => {
                    reports.pop_front();
                    SubmitOutcome::Coalesced
                }
                SubmitPolicy::Must | SubmitPolicy::Lossless => SubmitOutcome::WouldBlock,
            };
        }

        let (lane, frame_id) = match cmd {
            GpuCmd::UploadFrame { lane, frame_id } => (lane, frame_id),
        };
        reports.push_back(GpuRep::FramePresented { lane, frame_id });
        SubmitOutcome::Accepted
    }

    fn try_poll_report(&self) -> Option<Self::Report> {
        self.reports.lock().pop_front()
    }
}

pub fn default_service() -> GpuServiceHandle {
    GpuService::new_handle(DEFAULT_CAPACITY)
}
