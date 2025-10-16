//! GPU service implementation for managing frame presentation.

use hub::{GpuCmd, GpuRep, GpuServiceHandle, Service, SubmitOutcome};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

/// Mock GPU service for testing and prototyping.
pub struct GpuService {
    reports: Mutex<VecDeque<GpuRep>>,
    capacity: usize,
    next_frame_id: Mutex<u64>,
}

impl GpuService {
    /// Creates a new GPU service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> GpuServiceHandle {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            next_frame_id: Mutex::new(0),
        })
    }
}

impl Default for GpuService {
    fn default() -> Self {
        Self {
            reports: Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
            capacity: DEFAULT_CAPACITY,
            next_frame_id: Mutex::new(0),
        }
    }
}

impl Service for GpuService {
    type Cmd = GpuCmd;
    type Rep = GpuRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            return SubmitOutcome::WouldBlock;
        }

        let lane = match cmd {
            GpuCmd::UploadFrame { lane, .. } => *lane,
        };
        let mut frame_id = self.next_frame_id.lock();
        *frame_id = frame_id.wrapping_add(1);
        reports.push_back(GpuRep::FrameShown {
            lane,
            frame_id: *frame_id,
        });
        SubmitOutcome::Accepted
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        if max == 0 {
            return SmallVec::new();
        }

        let mut reports = self.reports.lock();
        let mut out = SmallVec::<[GpuRep; 8]>::new();
        let limit = max.min(reports.len());
        for _ in 0..limit {
            if let Some(rep) = reports.pop_front() {
                out.push(rep);
            }
        }
        out
    }
}

/// Creates a GPU service handle with default capacity.
pub fn default_service() -> GpuServiceHandle {
    GpuService::new_handle(DEFAULT_CAPACITY)
}
