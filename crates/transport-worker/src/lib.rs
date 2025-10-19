//! Reusable transport worker for WASM using wasm-bindgen pattern.
//!
//! This crate provides app-agnostic worker functions for transport operations.
//! It has NO dependencies on app/hub/world - it's purely about transport layer.
//!
//! For GBX-specific test orchestration, see the `gbx-wasm` crate which re-exports
//! these functions and adds test entry points.
#![allow(missing_docs)]

pub use types::*;

pub mod types {
    use transport::wasm::{MsgRingLayout, SlotPoolLayout};
    pub use transport_fabric::layout::{
        ArchivedFabricLayout, EndpointLayout, FabricLayout, PortLayout, PortRole,
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct WorkerInitDescriptor {
        pub cmd_ring: MsgRingLayout,
        pub evt_ring: MsgRingLayout,
        pub frame_pool: SlotPoolLayout,
        pub audio_pool: SlotPoolLayout,
    }

    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TestType {
        Flood = 0,
        Burst = 1,
        Backpressure = 2,
    }

    impl TestType {
        pub fn from_u32(value: u32) -> Option<Self> {
            match value {
                0 => Some(TestType::Flood),
                1 => Some(TestType::Burst),
                2 => Some(TestType::Backpressure),
                _ => None,
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct TestConfig {
        pub test_type: u32,
        pub param1: u32,  // frame_count for Flood, bursts for Burst, frames for Backpressure
        pub param2: u32,  // unused for Flood, burst_size for Burst, unused for Backpressure
    }

    impl TestConfig {
        pub fn flood(frame_count: u32) -> Self {
            Self {
                test_type: TestType::Flood as u32,
                param1: frame_count,
                param2: 0,
            }
        }

        pub fn burst(bursts: u32, burst_size: u32) -> Self {
            Self {
                test_type: TestType::Burst as u32,
                param1: bursts,
                param2: burst_size,
            }
        }

        pub fn backpressure(frames: u32) -> Self {
            Self {
                test_type: TestType::Backpressure as u32,
                param1: frames,
                param2: 0,
            }
        }

        pub fn get_type(&self) -> Option<TestType> {
            TestType::from_u32(self.test_type)
        }
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
    use transport_fabric::{ArchivedFabricLayout, ServiceEngine, WorkerRuntime};
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

    /// Reconstructed fabric endpoints available to service engines
    struct FabricEndpoints {
        evt_ring: MsgRing,
        frame_pool: SlotPool,
        #[allow(dead_code)]
        audio_pool: SlotPool,
    }

    thread_local! {
        static FABRIC_ENDPOINTS: RefCell<Vec<FabricEndpoints>> = RefCell::new(Vec::new());
        static FABRIC_RUNTIME: RefCell<Option<WorkerRuntime>> = RefCell::new(None);
        static SCENARIO_STATS: RefCell<Option<*mut ScenarioStats>> = RefCell::new(None);
    }

    /// Test scenario engine: Flood frames
    struct FloodEngine {
        frame_count: u32,
        current_frame: u32,
    }

    impl ServiceEngine for FloodEngine {
        fn poll(&mut self) -> usize {
            if self.current_frame >= self.frame_count {
                return 0;
            }

            let mut work = 0;
            FABRIC_ENDPOINTS.with(|endpoints_cell| {
                let mut endpoints_guard = endpoints_cell.borrow_mut();
                if let Some(endpoint) = endpoints_guard.first_mut() {
                    SCENARIO_STATS.with(|stats_cell| {
                        let stats_ptr = stats_cell.borrow().unwrap();
                        let stats = unsafe { &mut *stats_ptr };

                        while self.current_frame < self.frame_count {
                            produce_frame(endpoint, self.current_frame, stats);
                            self.current_frame += 1;
                            work += 1;

                            // Yield after some work to allow other engines to run
                            if work >= 100 {
                                break;
                            }
                        }
                    });
                }
            });

            work
        }

        fn name(&self) -> &'static str {
            "flood"
        }
    }

    /// Test scenario engine: Burst frames
    struct BurstEngine {
        bursts: u32,
        burst_size: u32,
        current_burst: u32,
        current_offset: u32,
    }

    impl ServiceEngine for BurstEngine {
        fn poll(&mut self) -> usize {
            if self.current_burst >= self.bursts {
                return 0;
            }

            let mut work = 0;
            FABRIC_ENDPOINTS.with(|endpoints_cell| {
                let mut endpoints_guard = endpoints_cell.borrow_mut();
                if let Some(endpoint) = endpoints_guard.first_mut() {
                    SCENARIO_STATS.with(|stats_cell| {
                        let stats_ptr = stats_cell.borrow().unwrap();
                        let stats = unsafe { &mut *stats_ptr };

                        while self.current_burst < self.bursts {
                            while self.current_offset < self.burst_size {
                                let frame_id = self.current_burst * self.burst_size + self.current_offset;
                                produce_frame(endpoint, frame_id, stats);
                                self.current_offset += 1;
                                work += 1;
                            }
                            self.current_offset = 0;
                            self.current_burst += 1;

                            // Yield after each burst
                            break;
                        }
                    });
                }
            });

            work
        }

        fn name(&self) -> &'static str {
            "burst"
        }
    }

    /// Test scenario engine: Backpressure
    struct BackpressureEngine {
        frames: u32,
        current_frame: u32,
    }

    impl ServiceEngine for BackpressureEngine {
        fn poll(&mut self) -> usize {
            if self.current_frame >= self.frames {
                return 0;
            }

            let mut work = 0;
            FABRIC_ENDPOINTS.with(|endpoints_cell| {
                let mut endpoints_guard = endpoints_cell.borrow_mut();
                if let Some(endpoint) = endpoints_guard.first_mut() {
                    SCENARIO_STATS.with(|stats_cell| {
                        let stats_ptr = stats_cell.borrow().unwrap();
                        let stats = unsafe { &mut *stats_ptr };

                        while self.current_frame < self.frames {
                            produce_frame(endpoint, self.current_frame, stats);
                            self.current_frame += 1;
                            work += 1;

                            // Yield occasionally to simulate backpressure
                            if work >= 50 {
                                break;
                            }
                        }
                    });
                }
            });

            work
        }

        fn name(&self) -> &'static str {
            "backpressure"
        }
    }

    /// Initialize fabric from WorkerInitDescriptor
    /// Builds a FabricLayout and reconstructs endpoints properly
    #[wasm_bindgen]
    pub fn worker_init(descriptor_ptr: u32) -> i32 {
        unsafe {
            let descriptor = match ref_from_u32::<WorkerInitDescriptor>(descriptor_ptr) {
                Some(value) => value,
                None => return ERR_NULL_PTR,
            };

            // Build FabricLayout from descriptor
            let mut layout = FabricLayout::default();
            let mut endpoint_layout = EndpointLayout::default();

            // Add event ring port
            endpoint_layout.push_port(
                PortRole::Replies,
                PortLayout::MsgRing(descriptor.evt_ring),
            );

            // Add frame pool
            endpoint_layout.push_port(
                PortRole::SlotPool(0),
                PortLayout::SlotPool(descriptor.frame_pool),
            );

            // Add audio pool
            endpoint_layout.push_port(
                PortRole::SlotPool(1),
                PortLayout::SlotPool(descriptor.audio_pool),
            );

            layout.add_endpoint(endpoint_layout);

            // Serialize the layout with rkyv
            use rkyv::rancor::Error;
            let bytes = rkyv::to_bytes::<Error>(&layout).unwrap();

            // Initialize fabric with the layout
            fabric_worker_init(bytes.as_ptr() as u32, bytes.len() as u32)
        }
    }

    /// Register a test scenario engine in the fabric runtime
    #[wasm_bindgen]
    pub fn worker_register_test(config_ptr: u32, stats_ptr: u32) -> i32 {
        unsafe {
            let config = match ref_from_u32::<TestConfig>(config_ptr) {
                Some(c) => c,
                None => return ERR_NULL_PTR,
            };
            let stats = match mut_from_u32::<ScenarioStats>(stats_ptr) {
                Some(s) => s,
                None => return ERR_NULL_PTR,
            };
            stats.reset();

            SCENARIO_STATS.with(|cell| {
                *cell.borrow_mut() = Some(stats);
            });

            FABRIC_RUNTIME.with(|runtime_cell| {
                let mut runtime_guard = runtime_cell.borrow_mut();
                let runtime = match runtime_guard.as_mut() {
                    Some(r) => r,
                    None => return ERR_NOT_INIT,
                };

                let test_type = match config.get_type() {
                    Some(t) => t,
                    None => return -5, // ERR_INVALID_TEST_TYPE
                };

                match test_type {
                    TestType::Flood => {
                        let engine = FloodEngine {
                            frame_count: config.param1,
                            current_frame: 0,
                        };
                        runtime.register(engine);
                    }
                    TestType::Burst => {
                        let engine = BurstEngine {
                            bursts: config.param1,
                            burst_size: config.param2,
                            current_burst: 0,
                            current_offset: 0,
                        };
                        runtime.register(engine);
                    }
                    TestType::Backpressure => {
                        let engine = BackpressureEngine {
                            frames: config.param1,
                            current_frame: 0,
                        };
                        runtime.register(engine);
                    }
                }

                OK
            })
        }
    }

    /// Initialize fabric worker runtime from a rkyv-serialized FabricLayout.
    /// The layout_ptr points to rkyv-archived bytes containing the FabricLayout.
    /// Reconstructs all endpoints from the layout and makes them available to engines.
    #[wasm_bindgen]
    pub fn fabric_worker_init(layout_ptr: u32, layout_len: u32) -> i32 {
        use rkyv::access_unchecked;

        if layout_ptr == 0 || layout_len == 0 {
            // Empty layout - just init empty runtime
            return FABRIC_RUNTIME.with(|runtime| {
                if runtime.borrow().is_some() {
                    return ERR_ALREADY_INIT;
                }
                *runtime.borrow_mut() = Some(WorkerRuntime::new());
                OK
            });
        }

        unsafe {
            let layout_bytes = std::slice::from_raw_parts(layout_ptr as *const u8, layout_len as usize);

            // Access the archived FabricLayout
            let archived_layout = access_unchecked::<ArchivedFabricLayout>(layout_bytes);

            // Reconstruct endpoints from the layout
            let mut endpoints = Vec::new();

            for endpoint_layout in archived_layout.endpoints.iter() {
                let mut evt_ring = None;
                let mut frame_pool = None;
                let mut audio_pool = None;

                for port_tuple in endpoint_layout.ports.iter() {
                    use rkyv::Archived;

                    // Access tuple fields (rkyv::ArchivedTuple2 has .0 and .1 fields)
                    let role = &port_tuple.0;
                    let port_layout = &port_tuple.1;

                    match (role, port_layout) {
                        (Archived::<PortRole>::Replies, Archived::<PortLayout>::MsgRing(ring_layout)) => {
                            evt_ring = Some(MsgRing::from_wasm_layout(ring_layout, EVENT_ENVELOPE));
                        }
                        (Archived::<PortRole>::SlotPool(idx), Archived::<PortLayout>::SlotPool(pool_layout)) if idx.to_native() == 0 => {
                            frame_pool = Some(SlotPool::from_wasm_layout(pool_layout));
                        }
                        (Archived::<PortRole>::SlotPool(idx), Archived::<PortLayout>::SlotPool(pool_layout)) if idx.to_native() == 1 => {
                            audio_pool = Some(SlotPool::from_wasm_layout(pool_layout));
                        }
                        _ => {} // Ignore other ports
                    }
                }

                if let (Some(evt_ring), Some(frame_pool), Some(audio_pool)) = (evt_ring, frame_pool, audio_pool) {
                    endpoints.push(FabricEndpoints {
                        evt_ring,
                        frame_pool,
                        audio_pool,
                    });
                }
            }

            // Store reconstructed endpoints
            FABRIC_ENDPOINTS.with(|eps| {
                *eps.borrow_mut() = endpoints;
            });

            FABRIC_RUNTIME.with(|runtime| {
                if runtime.borrow().is_some() {
                    return ERR_ALREADY_INIT;
                }

                // Create a new WorkerRuntime
                let worker_runtime = WorkerRuntime::new();

                *runtime.borrow_mut() = Some(worker_runtime);
                OK
            })
        }
    }

    /// Run one tick of the fabric worker runtime, polling all registered engines.
    /// Returns the total amount of work done (sum of all engine poll results).
    #[wasm_bindgen]
    pub fn fabric_worker_run() -> i32 {
        FABRIC_RUNTIME.with(|runtime| {
            let mut guard = runtime.borrow_mut();
            let worker_runtime = match guard.as_mut() {
                Some(rt) => rt,
                None => return ERR_NOT_INIT,
            };

            let work = worker_runtime.run_tick();
            work as i32
        })
    }

    fn produce_frame(endpoint: &mut FabricEndpoints, frame_id: u32, stats: &mut ScenarioStats) {
        let slot_idx = acquire_free_slot(&mut endpoint.frame_pool, stats);
        write_frame(&mut endpoint.frame_pool, slot_idx, frame_id);
        push_ready_slot(&mut endpoint.frame_pool, slot_idx, stats);
        push_event(&mut endpoint.evt_ring, frame_id, slot_idx, stats);
        stats.produced = stats.produced.wrapping_add(1);
    }

    fn acquire_free_slot(pool: &mut SlotPool, stats: &mut ScenarioStats) -> u32 {
        loop {
            if let Some(idx) = pool.try_acquire_free() {
                return idx;
            }
            stats.free_waits = stats.free_waits.wrapping_add(1);
            pool.wait_for_free_slot();
        }
    }

    fn push_ready_slot(pool: &mut SlotPool, idx: u32, stats: &mut ScenarioStats) {
        loop {
            match pool.push_ready(idx) {
                SlotPush::Ok => break,
                SlotPush::WouldBlock => {
                    stats.would_block_ready = stats.would_block_ready.wrapping_add(1);
                    pool.wait_for_ready_drain();
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
            ring.wait_for_space();
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
pub use wasm::{fabric_worker_init, fabric_worker_run, worker_init, worker_register_test};

#[cfg(not(target_arch = "wasm32"))]
mod stubs {
    #[no_mangle]
    pub extern "C" fn worker_init(_descriptor_ptr: u32) -> i32 {
        let _ = _descriptor_ptr;
        -1
    }

    #[no_mangle]
    pub extern "C" fn worker_register_test(_config_ptr: u32, _stats_ptr: u32) -> i32 {
        let _ = (_config_ptr, _stats_ptr);
        -1
    }

    #[no_mangle]
    pub extern "C" fn fabric_worker_init(_layout_ptr: u32, _layout_len: u32) -> i32 {
        let _ = (_layout_ptr, _layout_len);
        -1
    }

    #[no_mangle]
    pub extern "C" fn fabric_worker_run() -> i32 {
        -1
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use stubs::*;
