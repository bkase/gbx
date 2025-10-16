//! Kernel service implementation for emulator core execution.

use hub::{
    FrameSpan, KernelCmd, KernelRep, KernelServiceHandle, Service, SubmitOutcome, SubmitPolicy,
    TickPurpose,
};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 64;

/// Mock kernel service for testing and prototyping.
pub struct KernelService {
    reports: Mutex<VecDeque<KernelRep>>,
    capacity: usize,
    next_frame_id: Mutex<u64>,
}

impl KernelService {
    /// Creates a new kernel service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> KernelServiceHandle {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            next_frame_id: Mutex::new(0),
        })
    }

    fn ensure_capacity(
        &self,
        current: usize,
        needed: usize,
        policy: SubmitPolicy,
    ) -> SubmitOutcome {
        if current + needed <= self.capacity {
            return SubmitOutcome::Accepted;
        }

        match policy {
            SubmitPolicy::BestEffort => SubmitOutcome::Dropped,
            SubmitPolicy::Coalesce => SubmitOutcome::Coalesced,
            SubmitPolicy::Must | SubmitPolicy::Lossless => SubmitOutcome::WouldBlock,
        }
    }

    fn reports_for(&self, cmd: &KernelCmd) -> usize {
        match cmd {
            KernelCmd::Tick { purpose, .. } => {
                if matches!(purpose, TickPurpose::Display) {
                    2
                } else {
                    1
                }
            }
            KernelCmd::LoadRom { .. } => 1,
            KernelCmd::SetInputs { .. } => 0,
            KernelCmd::Terminate { .. } => 0,
        }
    }

    fn submit_policy(cmd: &KernelCmd) -> SubmitPolicy {
        match cmd {
            KernelCmd::Tick { purpose, .. } => match purpose {
                TickPurpose::Display => SubmitPolicy::Coalesce,
                TickPurpose::Exploration => SubmitPolicy::BestEffort,
            },
            KernelCmd::LoadRom { .. } => SubmitPolicy::Lossless,
            KernelCmd::SetInputs { .. } => SubmitPolicy::Lossless,
            KernelCmd::Terminate { .. } => SubmitPolicy::Lossless,
        }
    }

    fn materialise_reports(&self, cmd: &KernelCmd) -> SmallVec<[KernelRep; 2]> {
        match cmd {
            KernelCmd::Tick {
                group,
                purpose,
                budget,
            } => {
                let mut reports = SmallVec::new();
                if matches!(purpose, TickPurpose::Display) {
                    let mut frame_id = self.next_frame_id.lock();
                    let current_id = (*frame_id).wrapping_add(1);
                    *frame_id = current_id;
                    reports.push(KernelRep::LaneFrame {
                        group: *group,
                        lane: 0,
                        span: FrameSpan::default(),
                        frame_id: current_id,
                    });
                }
                reports.push(KernelRep::TickDone {
                    group: *group,
                    lanes_mask: 0b1,
                    cycles_done: *budget,
                });
                reports
            }
            KernelCmd::LoadRom { group, bytes } => {
                let mut reports = SmallVec::new();
                reports.push(KernelRep::RomLoaded {
                    group: *group,
                    bytes_len: bytes.len(),
                });
                reports
            }
            KernelCmd::SetInputs { .. } => SmallVec::new(),
            KernelCmd::Terminate { .. } => SmallVec::new(),
        }
    }
}

impl Default for KernelService {
    fn default() -> Self {
        Self {
            reports: Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
            capacity: DEFAULT_CAPACITY,
            next_frame_id: Mutex::new(0),
        }
    }
}

impl Service for KernelService {
    type Cmd = KernelCmd;
    type Rep = KernelRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let policy = Self::submit_policy(cmd);
        let needed = self.reports_for(cmd);
        let mut reports = self.reports.lock();

        let status = self.ensure_capacity(reports.len(), needed, policy);
        match status {
            SubmitOutcome::Dropped => return SubmitOutcome::Dropped,
            SubmitOutcome::WouldBlock => return SubmitOutcome::WouldBlock,
            SubmitOutcome::Coalesced => {
                while reports.len() + needed > self.capacity && !reports.is_empty() {
                    reports.pop_front();
                }
            }
            SubmitOutcome::Accepted => {}
            SubmitOutcome::Closed => return SubmitOutcome::Closed,
        }

        if reports.len() + needed > self.capacity {
            return SubmitOutcome::WouldBlock;
        }

        let new_reports = self.materialise_reports(cmd);
        for rep in new_reports {
            reports.push_back(rep);
        }

        if status == SubmitOutcome::Coalesced {
            SubmitOutcome::Coalesced
        } else {
            SubmitOutcome::Accepted
        }
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        if max == 0 {
            return SmallVec::new();
        }

        let mut reports = self.reports.lock();
        let mut out = SmallVec::<[KernelRep; 8]>::new();
        let limit = max.min(reports.len());
        for _ in 0..limit {
            if let Some(rep) = reports.pop_front() {
                out.push(rep);
            }
        }
        out
    }
}

/// Creates a kernel service handle with default capacity.
pub fn default_service() -> KernelServiceHandle {
    KernelService::new_handle(DEFAULT_CAPACITY)
}
