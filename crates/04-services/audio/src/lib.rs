//! Audio service implementation for managing sound playback.

use hub::{AudioCmd, AudioRep, AudioServiceHandle, Service, SubmitOutcome, SubmitPolicy};
use services_common::{drain_queue, try_submit_queue, LocalQueue};
use smallvec::SmallVec;
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 128;

/// Mock audio service for testing and prototyping.
pub struct AudioService {
    reports: LocalQueue<AudioRep>,
    capacity: usize,
}

impl AudioService {
    /// Creates a new audio service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> AudioServiceHandle {
        Arc::new(Self {
            reports: LocalQueue::with_capacity(capacity),
            capacity,
        })
    }
}

impl Default for AudioService {
    fn default() -> Self {
        Self {
            reports: LocalQueue::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl Service for AudioService {
    type Cmd = AudioCmd;
    type Rep = AudioRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        try_submit_queue::<AudioRep, _>(&self.reports, self.capacity, SubmitPolicy::Must, 1, || {
            match cmd {
                AudioCmd::Submit { span } => {
                    let frames = if span.channels == 0 {
                        0
                    } else {
                        span.samples.len() / usize::from(span.channels)
                    };
                    let mut reps = SmallVec::new();
                    reps.push(AudioRep::Played { frames });
                    reps
                }
            }
        })
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        drain_queue::<AudioRep>(&self.reports, max)
    }
}

/// Creates an audio service handle with default capacity.
pub fn default_service() -> AudioServiceHandle {
    AudioService::new_handle(DEFAULT_CAPACITY)
}
