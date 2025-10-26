#![deny(unsafe_op_in_unsafe_fn)]
#![allow(missing_docs)]
#![feature(portable_simd)]

mod instance;
mod sink_transport;

use crate::instance::Instance;
use crate::sink_transport::TransportFrameSink;
use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use kernel_core::{BusScalar, BusSimd, Core, CoreConfig, Model, SimdCore};
use log::{debug, trace};
use service_abi::{
    DebugCmd, DebugRep, FrameSpan, KernelCmd, KernelRep, KernelServiceHandle, Service, StepKind,
    SubmitOutcome, SubmitPolicy, TickPurpose,
};
use services_common::{drain_queue, try_submit_queue, LocalQueue};
use smallvec::{smallvec, SmallVec};
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::Arc;
use transport::{SlotPool, SlotPoolConfig, SlotPoolHandle};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

fn load_dmg_boot_rom() -> Option<Arc<[u8]>> {
    // Runtime environments are expected to provide the DMG boot ROM via
    // explicit `LoadRom` commands. Returning `None` keeps the core on the
    // post-boot path until a ROM is supplied.
    None
}

fn load_default_rom() -> Arc<[u8]> {
    // Always start with a blank cartridge; frontends must supply ROM bytes via
    // an explicit `LoadRom` intent.
    Arc::from(vec![0u8; 0x8000].into_boxed_slice())
}

struct SingleThreadCell<T> {
    value: UnsafeCell<T>,
}

// SAFETY: SingleThreadCell wraps UnsafeCell but guarantees single-threaded access through its API.
// The kernel service owns this cell and only accesses it from the scheduler's single-threaded context.
unsafe impl<T> Send for SingleThreadCell<T> {}
// SAFETY: Same as Send - single-threaded access is enforced by the service design, so sharing
// references across threads is safe as long as only one thread ever calls with_mut at a time.
unsafe impl<T> Sync for SingleThreadCell<T> {}

impl<T> SingleThreadCell<T> {
    fn new(value: T) -> Self {
        Self {
            value: UnsafeCell::new(value),
        }
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // SAFETY: The service design ensures only one thread accesses this at a time,
        // so obtaining a mutable reference is safe.
        unsafe { f(&mut *self.value.get()) }
    }
}

/// Collection of emulation instances managed by the kernel service.
pub struct KernelFarm {
    instances: HashMap<u16, Instance>,
    frame_pool: Arc<SlotPoolHandle>,
    core_config: CoreConfig,
    default_rom: Arc<[u8]>,
    boot_rom: Option<Arc<[u8]>>,
}

impl KernelFarm {
    pub fn new(frame_pool: Arc<SlotPoolHandle>, core_config: CoreConfig) -> Self {
        let default_rom = load_default_rom();
        let boot_rom = load_dmg_boot_rom();
        Self {
            instances: HashMap::new(),
            frame_pool,
            core_config,
            default_rom,
            boot_rom,
        }
    }

    fn ensure_instance(&mut self, id: u16) -> &mut Instance {
        if !self.instances.contains_key(&id) {
            let sink = TransportFrameSink::new(
                Arc::clone(&self.frame_pool),
                self.core_config.frame_width,
                self.core_config.frame_height,
            );
            let lanes = self.core_config.lanes.get();
            let instance = match lanes {
                1 => {
                    let mut core = Core::new(
                        BusScalar::new(Arc::clone(&self.default_rom), self.boot_rom.clone()),
                        self.core_config,
                        Model::Dmg,
                    );
                    if self.boot_rom.is_some() {
                        core.reset_power_on(Model::Dmg);
                    } else {
                        core.reset_post_boot(Model::Dmg);
                    }
                    Instance::new_scalar(core, sink)
                }
                2 => {
                    let mut core = SimdCore::<2>::new(
                        BusSimd::new(Arc::clone(&self.default_rom), self.boot_rom.clone()),
                        self.core_config,
                        Model::Dmg,
                    );
                    if self.boot_rom.is_some() {
                        core.reset_power_on(Model::Dmg);
                    } else {
                        core.reset_post_boot(Model::Dmg);
                    }
                    Instance::new_simd2(core, sink)
                }
                4 => {
                    let mut core = SimdCore::<4>::new(
                        BusSimd::new(Arc::clone(&self.default_rom), self.boot_rom.clone()),
                        self.core_config,
                        Model::Dmg,
                    );
                    if self.boot_rom.is_some() {
                        core.reset_power_on(Model::Dmg);
                    } else {
                        core.reset_post_boot(Model::Dmg);
                    }
                    Instance::new_simd4(core, sink)
                }
                8 => {
                    let mut core = SimdCore::<8>::new(
                        BusSimd::new(Arc::clone(&self.default_rom), self.boot_rom.clone()),
                        self.core_config,
                        Model::Dmg,
                    );
                    if self.boot_rom.is_some() {
                        core.reset_power_on(Model::Dmg);
                    } else {
                        core.reset_post_boot(Model::Dmg);
                    }
                    Instance::new_simd8(core, sink)
                }
                _ => panic!("unsupported SIMD lane count {}", lanes),
            };
            self.instances.insert(id, instance);
        }
        self.instances.get_mut(&id).expect("instance exists")
    }

    fn publish_lane_frames(
        id: u16,
        inst: &mut Instance,
        expected_len: usize,
        width: u16,
        height: u16,
        out: &mut Vec<KernelRep>,
    ) -> bool {
        let mut published = false;
        let lanes = inst.lanes.get();
        if lanes == 1 {
            if let Some((pixels, slot_span)) = inst.produce_frame(expected_len) {
                trace!(
                    "publish_lane_frames: scalar lane=0 frame_id={} slot_span={:?} bytes={}",
                    inst.next_frame_id + 1,
                    slot_span.as_ref().map(|s| (s.start_idx, s.count)),
                    pixels.len()
                );
                let frame_id = inst.bump_frame_id();
                out.push(KernelRep::LaneFrame {
                    group: id,
                    lane: 0,
                    span: FrameSpan {
                        width,
                        height,
                        pixels,
                        slot_span,
                    },
                    frame_id,
                });
                return true;
            }
            return false;
        }

        for lane in 0..lanes {
            if let Some((pixels, slot_span)) = inst.produce_frame_for_lane(expected_len, lane) {
                trace!(
                    "publish_lane_frames: lane={} frame_id={} slot_span={:?} bytes={}",
                    lane,
                    inst.next_frame_id + 1,
                    slot_span.as_ref().map(|s| (s.start_idx, s.count)),
                    pixels.len()
                );
                let frame_id = inst.bump_frame_id();
                out.push(KernelRep::LaneFrame {
                    group: id,
                    lane: lane as u16,
                    span: FrameSpan {
                        width,
                        height,
                        pixels,
                        slot_span,
                    },
                    frame_id,
                });
                published = true;
            }
        }
        published
    }

    pub fn tick(&mut self, id: u16, budget: u32, out: &mut Vec<KernelRep>) -> u32 {
        let inst = self.ensure_instance(id);
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "kernel::tick wasm start group={id}"
        )));
        let mut cycles = inst.step_cycles(budget);
        let initial_ready = inst.frame_ready();
        let boot_active = inst.boot_active();
        debug!(
            "kernel::tick start group={id} budget={budget} cycles={cycles} frame_ready={} boot_active={} lanes={}",
            initial_ready,
            boot_active,
            inst.lanes.get(),
        );
        let (mut width, mut height) = inst.sink.dimensions();
        if width == 0 {
            width = 160;
        }
        if height == 0 {
            height = 144;
        }
        if width == 0 || height == 0 {
            panic!("transport frame dimensions must be non-zero");
        }

        let expected_len = usize::from(width)
            .saturating_mul(usize::from(height))
            .saturating_mul(4);

        let mut published = false;
        if inst.frame_ready() {
            trace!("kernel::tick: publishing frames immediately");
            published = Self::publish_lane_frames(id, inst, expected_len, width, height, out);
            #[cfg(target_arch = "wasm32")]
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "kernel::tick wasm publish immediate group={id} frames={}",
                out.len()
            )));
        }

        if !published && inst.boot_active() {
            trace!("kernel::tick: boot path publish attempt");
            published = Self::publish_lane_frames(id, inst, expected_len, width, height, out);
        }

        if !published && !inst.boot_active() {
            const EXTRA_ATTEMPTS: usize = 32;
            let mut attempt = 0;
            while attempt < EXTRA_ATTEMPTS && !inst.boot_active() {
                let extra = inst.step_cycles(CYCLES_PER_FRAME);
                trace!(
                    "kernel::tick: extra attempt={} produced cycles={extra} frame_ready={} boot_active={}",
                    attempt,
                    inst.frame_ready(),
                    inst.boot_active()
                );
                if extra == 0 {
                    trace!(
                        "kernel::tick: extra step produced zero cycles (attempt={})",
                        attempt
                    );
                    break;
                }
                cycles = cycles.wrapping_add(extra);
                if inst.frame_ready() {
                    trace!(
                        "kernel::tick: frame ready after extra attempt={} cycles={}",
                        attempt,
                        cycles
                    );
                    if Self::publish_lane_frames(id, inst, expected_len, width, height, out) {
                        #[cfg(target_arch = "wasm32")]
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "kernel::tick wasm publish after extra attempt={attempt} group={id}"
                        )));
                        break;
                    }
                }
                attempt += 1;
            }
        }

        if !published {
            trace!("kernel::tick: forcing snapshot render (no frame_ready reported)");
            if Self::publish_lane_frames(id, inst, expected_len, width, height, out) {
                published = true;
                #[cfg(target_arch = "wasm32")]
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "kernel::tick wasm forced snapshot group={id}"
                )));
            }
        }
        let lanes = inst.lanes.get();
        let lanes_mask = if lanes >= u32::BITS as usize {
            u32::MAX
        } else {
            (1u32 << lanes) - 1
        };

        out.push(KernelRep::TickDone {
            group: id,
            lanes_mask,
            cycles_done: cycles,
        });
        let published_frames = out
            .iter()
            .filter(|rep| matches!(rep, KernelRep::LaneFrame { .. }))
            .count();
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "kernel::tick wasm end group={id} cycles_done={cycles} published_frames={published_frames} published_flag={published}"
        )));
        debug!(
            "kernel::tick end group={id} cycles_done={cycles} published_frames={published_frames} published_flag={published}"
        );
        cycles
    }

    pub fn load_rom(&mut self, id: u16, rom: Arc<[u8]>) -> usize {
        let inst = self.ensure_instance(id);
        inst.load_rom(rom.clone());
        rom.len()
    }

    pub fn set_inputs(&mut self, id: u16, joypad: u8) {
        if let Some(inst) = self.instances.get_mut(&id) {
            inst.set_inputs(joypad);
        }
    }

    pub fn terminate(&mut self, id: u16) {
        self.instances.remove(&id);
    }

    pub fn handle_debug(&mut self, cmd: &DebugCmd, out: &mut Vec<KernelRep>) {
        match cmd {
            DebugCmd::Snapshot { group } => {
                let inst = self.ensure_instance(*group);
                let snapshot = inst.inspector_snapshot();
                out.push(KernelRep::Debug(DebugRep::Snapshot(snapshot)));
            }
            DebugCmd::MemWindow {
                group,
                space,
                base,
                len,
            } => {
                let inst = self.ensure_instance(*group);
                let bytes = inst.mem_window(*space, *base, *len);
                out.push(KernelRep::Debug(DebugRep::MemWindow {
                    space: *space,
                    base: *base,
                    bytes: Arc::<[u8]>::from(bytes.into_boxed_slice()),
                }));
            }
            DebugCmd::StepInstruction { group, count } => {
                let inst = self.ensure_instance(*group);
                let (cycles, pc) = inst.step_instructions(*count);
                out.push(KernelRep::Debug(DebugRep::Stepped {
                    kind: StepKind::Instruction,
                    cycles,
                    pc,
                    disasm: None,
                }));
            }
            DebugCmd::StepFrame { group } => {
                let mut intermediate = Vec::new();
                let cycles = self.tick(*group, CYCLES_PER_FRAME, &mut intermediate);
                let pc = self.instances.get(group).map(|inst| inst.pc()).unwrap_or(0);
                out.extend(intermediate);
                out.push(KernelRep::Debug(DebugRep::Stepped {
                    kind: StepKind::Frame,
                    cycles,
                    pc,
                    disasm: None,
                }));
            }
        }
    }
}

const DEFAULT_CAPACITY: usize = 64;

/// Kernel service implementation backed by [`KernelFarm`].
pub struct KernelService {
    reports: LocalQueue<KernelRep>,
    capacity: usize,
    farm: SingleThreadCell<KernelFarm>,
}

impl Default for KernelService {
    fn default() -> Self {
        Self::build(
            DEFAULT_CAPACITY,
            default_frame_pool(),
            CoreConfig::default(),
        )
    }
}

impl KernelService {
    pub fn new_handle(capacity: usize) -> KernelServiceHandle {
        let frame_pool = default_frame_pool();
        Self::with_frame_pool(capacity, frame_pool, CoreConfig::default())
    }

    pub fn with_frame_pool(
        capacity: usize,
        frame_pool: Arc<SlotPoolHandle>,
        core_config: CoreConfig,
    ) -> KernelServiceHandle {
        Arc::new(Self::build(capacity, frame_pool, core_config))
    }

    /// Constructs a kernel service with an explicit frame pool and core configuration.
    pub fn new_with_frame_pool(
        capacity: usize,
        frame_pool: Arc<SlotPoolHandle>,
        core_config: CoreConfig,
    ) -> Self {
        Self::build(capacity, frame_pool, core_config)
    }

    /// Constructs a kernel service using the default frame pool but custom core configuration.
    pub fn new_with_config(core_config: CoreConfig) -> Self {
        Self::build(DEFAULT_CAPACITY, default_frame_pool(), core_config)
    }

    fn build(capacity: usize, frame_pool: Arc<SlotPoolHandle>, core_config: CoreConfig) -> Self {
        Self {
            reports: LocalQueue::with_capacity(capacity),
            capacity,
            farm: SingleThreadCell::new(KernelFarm::new(frame_pool, core_config)),
        }
    }

    fn reports_for(cmd: &KernelCmd) -> usize {
        match cmd {
            KernelCmd::Tick { .. } => 2,
            KernelCmd::LoadRom { .. } => 1,
            KernelCmd::SetInputs { .. } => 0,
            KernelCmd::Terminate { .. } => 0,
            KernelCmd::Debug(debug) => debug.expected_reports(),
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
            KernelCmd::Debug(debug) => debug.submit_policy(),
        }
    }

    fn materialise_reports(&self, cmd: &KernelCmd) -> SmallVec<[KernelRep; 8]> {
        match cmd {
            KernelCmd::Tick { group, budget, .. } => {
                let mut out = Vec::new();
                self.farm.with_mut(|farm| {
                    let _ = farm.tick(*group, *budget, &mut out);
                });
                SmallVec::from_vec(out)
            }
            KernelCmd::LoadRom { group, bytes } => {
                let len = self
                    .farm
                    .with_mut(|farm| farm.load_rom(*group, Arc::clone(bytes)));
                smallvec![KernelRep::RomLoaded {
                    group: *group,
                    bytes_len: len,
                }]
            }
            KernelCmd::SetInputs { group, joypad, .. } => {
                self.farm.with_mut(|farm| farm.set_inputs(*group, *joypad));
                SmallVec::new()
            }
            KernelCmd::Terminate { group } => {
                self.farm.with_mut(|farm| farm.terminate(*group));
                SmallVec::new()
            }
            KernelCmd::Debug(debug) => {
                let mut out = Vec::new();
                self.farm
                    .with_mut(|farm| farm.handle_debug(debug, &mut out));
                SmallVec::from_vec(out)
            }
        }
    }
}

impl Service for KernelService {
    type Cmd = KernelCmd;
    type Rep = KernelRep;

    fn try_submit(&self, cmd: &Self::Cmd) -> SubmitOutcome {
        let policy = Self::submit_policy(cmd);
        let needed = Self::reports_for(cmd);
        try_submit_queue(&self.reports, self.capacity, policy, needed, || {
            self.materialise_reports(cmd)
        })
    }

    fn drain(&self, max: usize) -> SmallVec<[Self::Rep; 8]> {
        drain_queue(&self.reports, max)
    }
}

/// Creates a kernel service handle with default capacity.
pub fn default_service() -> KernelServiceHandle {
    KernelService::new_handle(DEFAULT_CAPACITY)
}

fn default_frame_pool() -> Arc<SlotPoolHandle> {
    let pool = SlotPool::new(SlotPoolConfig {
        slot_count: 4,
        slot_size: 160 * 144 * 4,
    })
    .expect("allocate default slot pool");
    Arc::new(SlotPoolHandle::new(pool))
}

#[cfg(test)]
mod tests;
