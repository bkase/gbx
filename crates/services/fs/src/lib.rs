use hub::{FsCmd, FsRep, FsServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 32;

pub struct FsService {
    reports: Mutex<VecDeque<FsRep>>,
    capacity: usize,
}

impl FsService {
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
    type Command = FsCmd;
    type Report = FsRep;

    fn try_submit(&self, cmd: Self::Command, policy: SubmitPolicy) -> SubmitOutcome {
        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            let status = self.handle_overflow(policy, &mut reports);
            if status != SubmitOutcome::Coalesced {
                return status;
            }
        }

        match cmd {
            FsCmd::Persist { key, .. } => {
                reports.push_back(FsRep::Saved { key, ok: true });
            }
        }
        SubmitOutcome::Accepted
    }

    fn try_poll_report(&self) -> Option<Self::Report> {
        self.reports.lock().pop_front()
    }
}

pub fn default_service() -> FsServiceHandle {
    FsService::new_handle(DEFAULT_CAPACITY)
}
