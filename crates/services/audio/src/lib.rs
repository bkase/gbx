use hub::{AudioCmd, AudioRep, AudioServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

pub struct AudioService {
    reports: Mutex<VecDeque<AudioRep>>,
    capacity: usize,
}

impl AudioService {
    pub fn new_handle(capacity: usize) -> AudioServiceHandle {
        Arc::new(Self {
            reports: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        })
    }
}

impl Default for AudioService {
    fn default() -> Self {
        Self {
            reports: Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY)),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl Service for AudioService {
    type Command = AudioCmd;
    type Report = AudioRep;

    fn try_submit(&self, cmd: Self::Command, policy: SubmitPolicy) -> SubmitOutcome {
        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            return match policy {
                SubmitPolicy::BestEffort => {
                    reports.push_back(AudioRep::Underrun);
                    SubmitOutcome::Dropped
                }
                SubmitPolicy::Coalesce => {
                    reports.pop_front();
                    SubmitOutcome::Coalesced
                }
                SubmitPolicy::Must | SubmitPolicy::Lossless => SubmitOutcome::WouldBlock,
            };
        }

        match cmd {
            AudioCmd::SubmitSamples { frames } => {
                reports.push_back(AudioRep::Played { frames });
            }
        }
        SubmitOutcome::Accepted
    }

    fn try_poll_report(&self) -> Option<Self::Report> {
        self.reports.lock().pop_front()
    }
}

pub fn default_service() -> AudioServiceHandle {
    AudioService::new_handle(DEFAULT_CAPACITY)
}
