//! GPU service implementation for managing frame presentation.

use hub::{GpuCmd, GpuRep, GpuServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use services_common::{drain_queue, try_submit_queue, LocalQueue};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

/// Mock GPU service for testing and prototyping.
pub struct GpuService {
    reports: LocalQueue<GpuRep>,
    capacity: usize,
    next_frame_id: AtomicU64,
}

impl GpuService {
    /// Creates a new GPU service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> GpuServiceHandle {
        Arc::new(Self {
            reports: LocalQueue::with_capacity(capacity),
            capacity,
            next_frame_id: AtomicU64::new(0),
        })
    }
}

impl Default for GpuService {
    fn default() -> Self {
        Self {
            reports: LocalQueue::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
            next_frame_id: AtomicU64::new(0),
        }
    }
}

impl Service for GpuService {
    type Cmd = GpuCmd;
    type Rep = GpuRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        try_submit_queue::<GpuRep, _>(&self.reports, self.capacity, SubmitPolicy::Must, 1, || {
            let lane = match cmd {
                GpuCmd::UploadFrame { lane, .. } => *lane,
            };
            let frame_id = self
                .next_frame_id
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            let mut reps = SmallVec::new();
            reps.push(GpuRep::FrameShown { lane, frame_id });
            reps
        })
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        drain_queue::<GpuRep>(&self.reports, max)
    }
}

/// Creates a GPU service handle with default capacity.
pub fn default_service() -> GpuServiceHandle {
    GpuService::new_handle(DEFAULT_CAPACITY)
}
