//! Transport worker for WASM using wasm-bindgen pattern.
#![allow(missing_docs)]

pub use types::*;

pub mod types {
    use transport::wasm::{MsgRingLayout, SlotPoolLayout};

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WorkerInitDescriptor {
        pub cmd_ring: MsgRingLayout,
        pub evt_ring: MsgRingLayout,
        pub frame_pool: SlotPoolLayout,
        pub audio_pool: SlotPoolLayout,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct FloodConfig {
        pub frame_count: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct BurstConfig {
        pub bursts: u32,
        pub burst_size: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct BackpressureConfig {
        pub frames: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct ScenarioStats {
        pub produced: u32,
        pub would_block_ready: u32,
        pub would_block_evt: u32,
        pub free_waits: u32,
    }

    impl ScenarioStats {
        pub fn reset(&mut self) {
            *self = Self::default();
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::types::*;
    use std::cell::RefCell;
    use transport::{Envelope, MsgRing, SlotPool, SlotPush};
    use wasm_bindgen::prelude::*;

    const OK: i32 = 0;
    const ERR_NULL_PTR: i32 = -1;
    const ERR_ALREADY_INIT: i32 = -2;
    const ERR_NOT_INIT: i32 = -3;

    const EVENT_ENVELOPE: Envelope = Envelope {
        tag: 0x13,
        ver: 1,
        flags: 0,
    };
    const CMD_DEFAULT_ENVELOPE: Envelope = Envelope {
        tag: 0x01,
        ver: 1,
        flags: 0,
    };

    struct WorkerState {
        #[allow(dead_code)]
        cmd_ring: MsgRing,
        evt_ring: MsgRing,
        frame_pool: SlotPool,
        #[allow(dead_code)]
        audio_pool: SlotPool,
    }

    thread_local! {
        static STATE: RefCell<Option<WorkerState>> = RefCell::new(None);
    }

    /// Initialize the worker with a descriptor containing ring layouts.
    /// This uses wasm-bindgen so memory sharing happens via wasm_bindgen::memory().
    #[wasm_bindgen]
    pub fn worker_init(descriptor_ptr: u32) -> i32 {
        unsafe {
            let descriptor = match ref_from_u32::<WorkerInitDescriptor>(descriptor_ptr) {
                Some(value) => value,
                None => return ERR_NULL_PTR,
            };

            STATE.with(|state| {
                if state.borrow().is_some() {
                    return ERR_ALREADY_INIT;
                }

                let cmd_ring = MsgRing::from_wasm_layout(descriptor.cmd_ring, CMD_DEFAULT_ENVELOPE);
                let evt_ring = MsgRing::from_wasm_layout(descriptor.evt_ring, EVENT_ENVELOPE);
                let frame_pool = SlotPool::from_wasm_layout(descriptor.frame_pool);
                let audio_pool = SlotPool::from_wasm_layout(descriptor.audio_pool);

                *state.borrow_mut() = Some(WorkerState {
                    cmd_ring,
                    evt_ring,
                    frame_pool,
                    audio_pool,
                });
                OK
            })
        }
    }

    #[wasm_bindgen]
    pub fn worker_flood(config_ptr: u32, stats_ptr: u32) -> i32 {
        run_with_state::<_, FloodConfig>(config_ptr, stats_ptr, |state, cfg, stats| {
            let count = cfg.frame_count;
            for frame_id in 0..count {
                produce_frame(state, frame_id, stats);
            }
        })
    }

    #[wasm_bindgen]
    pub fn worker_burst(config_ptr: u32, stats_ptr: u32) -> i32 {
        run_with_state::<_, BurstConfig>(config_ptr, stats_ptr, |state, cfg, stats| {
            for burst in 0..cfg.bursts {
                let base = burst * cfg.burst_size;
                for offset in 0..cfg.burst_size {
                    produce_frame(state, base + offset, stats);
                }
            }
        })
    }

    #[wasm_bindgen]
    pub fn worker_backpressure(config_ptr: u32, stats_ptr: u32) -> i32 {
        run_with_state::<_, BackpressureConfig>(config_ptr, stats_ptr, |state, cfg, stats| {
            let frames = cfg.frames;
            for frame_id in 0..frames {
                produce_frame(state, frame_id, stats);
            }
        })
    }

    fn run_with_state<F, C>(config_ptr: u32, stats_ptr: u32, mut f: F) -> i32
    where
        F: FnMut(&mut WorkerState, C, &mut ScenarioStats),
        C: Copy + 'static,
    {
        unsafe {
            let stats = match mut_from_u32::<ScenarioStats>(stats_ptr) {
                Some(value) => value,
                None => return ERR_NULL_PTR,
            };
            stats.reset();

            let config = match ref_from_u32::<C>(config_ptr) {
                Some(value) => value,
                None => return ERR_NULL_PTR,
            };

            STATE.with(|state| {
                let mut guard = state.borrow_mut();
                let worker = match guard.as_mut() {
                    Some(value) => value,
                    None => return ERR_NOT_INIT,
                };
                f(worker, *config, stats);
                OK
            })
        }
    }

    fn produce_frame(state: &mut WorkerState, frame_id: u32, stats: &mut ScenarioStats) {
        let slot_idx = acquire_free_slot(&mut state.frame_pool, stats);
        write_frame(&mut state.frame_pool, slot_idx, frame_id);
        push_ready_slot(&mut state.frame_pool, slot_idx, stats);
        push_event(&mut state.evt_ring, frame_id, slot_idx, stats);
        stats.produced = stats.produced.wrapping_add(1);
    }

    fn acquire_free_slot(pool: &mut SlotPool, stats: &mut ScenarioStats) -> u32 {
        loop {
            if let Some(idx) = pool.try_acquire_free() {
                return idx;
            }
            stats.free_waits = stats.free_waits.wrapping_add(1);
            core::hint::spin_loop();
        }
    }

    fn push_ready_slot(pool: &mut SlotPool, idx: u32, stats: &mut ScenarioStats) {
        loop {
            match pool.push_ready(idx) {
                SlotPush::Ok => break,
                SlotPush::WouldBlock => {
                    stats.would_block_ready = stats.would_block_ready.wrapping_add(1);
                    core::hint::spin_loop();
                }
            }
        }
    }

    fn push_event(ring: &mut MsgRing, frame_id: u32, slot_idx: u32, stats: &mut ScenarioStats) {
        let payload = event_payload(frame_id, slot_idx);
        loop {
            if let Some(mut grant) = ring.try_reserve(payload.len()) {
                grant.payload()[..payload.len()].copy_from_slice(&payload);
                grant.commit(payload.len());
                break;
            }
            stats.would_block_evt = stats.would_block_evt.wrapping_add(1);
            core::hint::spin_loop();
        }
    }

    fn write_frame(pool: &mut SlotPool, slot_idx: u32, frame_id: u32) {
        if pool.slot_size() < 4 {
            return;
        }
        let bytes = frame_id.to_le_bytes();
        let slot = pool.slot_mut(slot_idx);
        slot[..4].copy_from_slice(&bytes);
    }

    fn event_payload(frame_id: u32, slot_idx: u32) -> [u8; 8] {
        let mut payload = [0u8; 8];
        payload[..4].copy_from_slice(&frame_id.to_le_bytes());
        payload[4..].copy_from_slice(&slot_idx.to_le_bytes());
        payload
    }

    unsafe fn ref_from_u32<T>(ptr: u32) -> Option<&'static T> {
        (ptr as *const T).as_ref()
    }

    unsafe fn mut_from_u32<T>(ptr: u32) -> Option<&'static mut T> {
        (ptr as *mut T).as_mut()
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(not(target_arch = "wasm32"))]
mod stubs {
    #[no_mangle]
    pub extern "C" fn worker_init(_descriptor_ptr: u32) -> i32 {
        let _ = _descriptor_ptr;
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_flood(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_burst(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_backpressure(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use stubs::*;
