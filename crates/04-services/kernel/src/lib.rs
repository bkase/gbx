//! Kernel service implementation for emulator core execution.

use hub::{
    FrameSpan, KernelCmd, KernelRep, KernelServiceHandle, Service, SubmitOutcome, SubmitPolicy,
    TickPurpose,
};
use services_common::{drain_queue, try_submit_queue, LocalQueue};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const DEFAULT_CAPACITY: usize = 64;

/// Mock kernel service for testing and prototyping.
pub struct KernelService {
    reports: LocalQueue<KernelRep>,
    capacity: usize,
    next_frame_id: AtomicU64,
}

impl KernelService {
    /// Creates a new kernel service handle with the specified report capacity.
    pub fn new_handle(capacity: usize) -> KernelServiceHandle {
        Arc::new(Self {
            reports: LocalQueue::with_capacity(capacity),
            capacity,
            next_frame_id: AtomicU64::new(0),
        })
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

    fn materialise_reports(&self, cmd: &KernelCmd) -> SmallVec<[KernelRep; 8]> {
        match cmd {
            KernelCmd::Tick {
                group,
                purpose,
                budget,
            } => {
                let mut reports = SmallVec::<[KernelRep; 8]>::new();
                if matches!(purpose, TickPurpose::Display) {
                    let current_id = self
                        .next_frame_id
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1);

                    // Generate checkerboard frame pixels (160x144 RGBA)
                    const W: u16 = 160;
                    const H: u16 = 144;
                    let buffer_size = gbx_frame::FRAME_HEADER + (W as usize) * (H as usize) * 4;
                    let mut buffer = vec![0u8; buffer_size];
                    let ok =
                        gbx_frame::write_checkerboard_rgba(&mut buffer, current_id as u32, W, H);
                    debug_assert!(ok, "checkerboard write should succeed");

                    // Skip the 8-byte header since FrameSpan.pixels is just the pixel data
                    let pixel_data = &buffer[gbx_frame::FRAME_HEADER..];

                    reports.push(KernelRep::LaneFrame {
                        group: *group,
                        lane: 0,
                        span: FrameSpan {
                            width: W,
                            height: H,
                            pixels: Arc::from(pixel_data),
                            slot_span: None,
                        },
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
                let mut reports = SmallVec::<[KernelRep; 8]>::new();
                reports.push(KernelRep::RomLoaded {
                    group: *group,
                    bytes_len: bytes.len(),
                });
                reports
            }
            KernelCmd::SetInputs { .. } => SmallVec::<[KernelRep; 8]>::new(),
            KernelCmd::Terminate { .. } => SmallVec::<[KernelRep; 8]>::new(),
        }
    }
}

impl Default for KernelService {
    fn default() -> Self {
        Self {
            reports: LocalQueue::with_capacity(DEFAULT_CAPACITY),
            capacity: DEFAULT_CAPACITY,
            next_frame_id: AtomicU64::new(0),
        }
    }
}

impl Service for KernelService {
    type Cmd = KernelCmd;
    type Rep = KernelRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let policy = Self::submit_policy(cmd);
        let needed = self.reports_for(cmd);
        try_submit_queue::<KernelRep, _>(&self.reports, self.capacity, policy, needed, || {
            self.materialise_reports(cmd)
        })
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        drain_queue::<KernelRep>(&self.reports, max)
    }
}

/// Creates a kernel service handle with default capacity.
pub fn default_service() -> KernelServiceHandle {
    KernelService::new_handle(DEFAULT_CAPACITY)
}
