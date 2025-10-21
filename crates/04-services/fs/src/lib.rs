//! Filesystem service implementation for persisting emulator data.

use hub::{FsCmd, FsRep, FsServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use services_common::{drain_queue, try_submit_queue, LocalQueue};
use smallvec::SmallVec;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 32;

/// Mock filesystem service for testing and prototyping.
pub struct FsService {
    reports: LocalQueue<FsRep>,
    capacity: usize,
}

impl FsService {
    /// Creates a new filesystem service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> FsServiceHandle {
        Arc::new(Self {
            reports: LocalQueue::with_capacity(capacity),
            capacity,
        })
    }
}

impl Default for FsService {
    fn default() -> Self {
        Self {
            reports: LocalQueue::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl Service for FsService {
    type Cmd = FsCmd;
    type Rep = FsRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let policy = match cmd {
            FsCmd::Persist { path, .. } => {
                if path.file_name().and_then(|name| name.to_str()) == Some("manual-save") {
                    SubmitPolicy::Lossless
                } else {
                    SubmitPolicy::Coalesce
                }
            }
        };

        try_submit_queue::<FsRep, _>(&self.reports, self.capacity, policy, 1, || match cmd {
            FsCmd::Persist { path, .. } => {
                let mut reps = SmallVec::new();
                reps.push(FsRep::Saved {
                    path: path.clone(),
                    ok: true,
                });
                reps
            }
        })
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        drain_queue::<FsRep>(&self.reports, max)
    }
}

/// Creates a filesystem service handle with default capacity.
pub fn default_service() -> FsServiceHandle {
    FsService::new_handle(DEFAULT_CAPACITY)
}
