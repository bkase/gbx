use crate::config::ScenarioKind;
use crate::handle::FabricHandle;
use crate::stats::StatsSink;
use crate::wrapping_add;
use transport::SlotPush;
use transport_fabric::ServiceEngine;

pub struct FrameScenarioEngine<H, S> {
    handle: H,
    stats: S,
    state: ScenarioState,
}

enum ScenarioState {
    Flood {
        frame_count: u32,
        current: u32,
    },
    Burst {
        bursts: u32,
        burst_size: u32,
        current_burst: u32,
        current_offset: u32,
    },
    Backpressure {
        frames: u32,
        produced: u32,
    },
}

impl<H, S> FrameScenarioEngine<H, S>
where
    H: FabricHandle,
    S: StatsSink,
{
    pub fn new(handle: H, stats: S, kind: ScenarioKind) -> Self {
        let state = match kind {
            ScenarioKind::Flood { frame_count } => ScenarioState::Flood {
                frame_count,
                current: 0,
            },
            ScenarioKind::Burst { bursts, burst_size } => ScenarioState::Burst {
                bursts,
                burst_size,
                current_burst: 0,
                current_offset: 0,
            },
            ScenarioKind::Backpressure { frames } => ScenarioState::Backpressure {
                frames,
                produced: 0,
            },
        };

        Self {
            handle,
            stats,
            state,
        }
    }

    fn produce_frame(handle: &mut H, stats: &S, frame_id: u32) -> bool {
        let mut free_waits = 0u32;
        let slot_idx = loop {
            if let Some(idx) = handle.acquire_free_slot() {
                break idx;
            }
            free_waits = free_waits.wrapping_add(1);
            handle.wait_for_free_slot();
        };

        handle.write_frame(slot_idx, frame_id);

        let mut ready_blocks = 0u32;
        loop {
            match handle.push_ready(slot_idx) {
                SlotPush::Ok => break,
                SlotPush::WouldBlock => {
                    ready_blocks = ready_blocks.wrapping_add(1);
                    handle.wait_for_ready_drain();
                }
            }
        }

        let mut evt_blocks = 0u32;
        loop {
            if handle.try_push_event(frame_id, slot_idx) {
                break;
            }
            evt_blocks = evt_blocks.wrapping_add(1);
            handle.wait_for_event_space();
        }

        stats.with_stats(|stats| {
            stats.produced = wrapping_add(stats.produced, 1);
            stats.free_waits = wrapping_add(stats.free_waits, free_waits);
            stats.would_block_ready = wrapping_add(stats.would_block_ready, ready_blocks);
            stats.would_block_evt = wrapping_add(stats.would_block_evt, evt_blocks);
        });

        true
    }
}

impl<H, S> ServiceEngine for FrameScenarioEngine<H, S>
where
    H: FabricHandle,
    S: StatsSink,
{
    fn poll(&mut self) -> usize {
        let stats = &self.stats;
        let handle = &mut self.handle;
        match &mut self.state {
            ScenarioState::Flood {
                frame_count,
                current,
            } => {
                let mut work = 0usize;
                while *current < *frame_count {
                    let frame_id = *current;
                    if Self::produce_frame(handle, stats, frame_id) {
                        *current += 1;
                        work += 1;
                    }
                    if work >= 100 {
                        break;
                    }
                }
                work
            }
            ScenarioState::Burst {
                bursts,
                burst_size,
                current_burst,
                current_offset,
            } => {
                if *current_burst >= *bursts {
                    return 0;
                }

                let mut work = 0usize;
                while *current_burst < *bursts {
                    while *current_offset < *burst_size {
                        let frame_id = (*current_burst) * (*burst_size) + (*current_offset);
                        if Self::produce_frame(handle, stats, frame_id) {
                            *current_offset += 1;
                            work += 1;
                        }
                    }
                    *current_offset = 0;
                    *current_burst += 1;
                    break;
                }
                work
            }
            ScenarioState::Backpressure { frames, produced } => {
                if *produced >= *frames {
                    return 0;
                }

                let mut work = 0usize;
                while *produced < *frames {
                    let frame_id = *produced;
                    if Self::produce_frame(handle, stats, frame_id) {
                        *produced += 1;
                        work += 1;
                    }
                    if work >= 50 {
                        break;
                    }
                }
                work
            }
        }
    }

    fn name(&self) -> &'static str {
        match self.state {
            ScenarioState::Flood { .. } => "flood",
            ScenarioState::Burst { .. } => "burst",
            ScenarioState::Backpressure { .. } => "backpressure",
        }
    }
}
