//! Filesystem service implementation for persisting emulator data.

use hub::{FsCmd, FsRep, FsServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 32;

/// Mock filesystem service for testing and prototyping.
pub struct FsService {
    reports: Mutex<VecDeque<FsRep>>,
    capacity: usize,
}

impl FsService {
    /// Creates a new filesystem service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> FsServiceHandle {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        })
    }

    fn handle_overflow(
        &self,
        policy: SubmitPolicy,
        reports: &mut VecDeque<FsRep>,
    ) -> SubmitOutcome {
        match policy {
            SubmitPolicy::BestEffort => SubmitOutcome::Dropped,
            SubmitPolicy::Coalesce => {
                reports.pop_front();
                SubmitOutcome::Coalesced
            }
            SubmitPolicy::Must | SubmitPolicy::Lossless => SubmitOutcome::WouldBlock,
        }
    }
}

impl Default for FsService {
    fn default() -> Self {
        Self {
            reports: Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
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

        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            let status = self.handle_overflow(policy, &mut reports);
            if status != SubmitOutcome::Coalesced {
                return status;
            }
        }

        match cmd {
            FsCmd::Persist { path, .. } => {
                reports.push_back(FsRep::Saved {
                    path: path.clone(),
                    ok: true,
                });
            }
        }
        SubmitOutcome::Accepted
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        if max == 0 {
            return SmallVec::new();
        }

        let mut reports = self.reports.lock();
        let mut out = SmallVec::<[FsRep; 8]>::new();
        let limit = max.min(reports.len());
        for _ in 0..limit {
            if let Some(rep) = reports.pop_front() {
                out.push(rep);
            }
        }
        out
    }
}

/// Creates a filesystem service handle with default capacity.
pub fn default_service() -> FsServiceHandle {
    FsService::new_handle(DEFAULT_CAPACITY)
}
