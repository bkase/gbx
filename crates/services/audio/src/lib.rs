//! Audio service implementation for managing sound playback.

use hub::{AudioCmd, AudioRep, AudioServiceHandle, Service, SubmitOutcome};
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

/// Mock audio service for testing and prototyping.
pub struct AudioService {
    reports: Mutex<VecDeque<AudioRep>>,
    capacity: usize,
}

impl AudioService {
    /// Creates a new audio service handle with the specified report capacity.
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
    type Cmd = AudioCmd;
    type Rep = AudioRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let mut reports = self.reports.lock();
        if reports.len() >= self.capacity {
            return SubmitOutcome::WouldBlock;
        }

        match cmd {
            AudioCmd::Submit { span } => {
                let frames = if span.channels == 0 {
                    0
                } else {
                    span.samples.len() / usize::from(span.channels)
                };
                reports.push_back(AudioRep::Played { frames });
            }
        }
        SubmitOutcome::Accepted
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        if max == 0 {
            return SmallVec::new();
        }

        let mut reports = self.reports.lock();
        let mut out = SmallVec::<[AudioRep; 8]>::new();
        let limit = max.min(reports.len());
        for _ in 0..limit {
            if let Some(rep) = reports.pop_front() {
                out.push(rep);
            }
        }
        out
    }
}

/// Creates an audio service handle with default capacity.
pub fn default_service() -> AudioServiceHandle {
    AudioService::new_handle(DEFAULT_CAPACITY)
}
