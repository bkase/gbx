#![deny(unsafe_op_in_unsafe_fn)]
#![allow(missing_docs)]

mod instance;
mod sink_transport;

use crate::instance::{AnyCore, Instance};
use crate::sink_transport::TransportFrameSink;
use kernel_core::ppu_stub::CYCLES_PER_FRAME;
use kernel_core::{BusScalar, Core, CoreConfig, Model};
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
    blank_rom: Arc<[u8]>,
}

impl KernelFarm {
    pub fn new(frame_pool: Arc<SlotPoolHandle>, core_config: CoreConfig) -> Self {
        let blank_rom = Arc::<[u8]>::from(vec![0u8; 0x8000].into_boxed_slice());
        Self {
            instances: HashMap::new(),
            frame_pool,
            core_config,
            blank_rom,
        }
    }

    fn ensure_instance(&mut self, id: u16) -> &mut Instance {
        if !self.instances.contains_key(&id) {
            let mut core = Core::new(
                BusScalar::new(Arc::clone(&self.blank_rom)),
                self.core_config,
                Model::Dmg,
            );
            core.reset_post_boot(Model::Dmg);
            let sink = TransportFrameSink::new(
                Arc::clone(&self.frame_pool),
                self.core_config.frame_width,
                self.core_config.frame_height,
            );
            self.instances.insert(id, Instance::new_scalar(core, sink));
        }
        self.instances.get_mut(&id).expect("instance exists")
    }

    pub fn tick(&mut self, id: u16, budget: u32, out: &mut Vec<KernelRep>) -> u32 {
        let inst = self.ensure_instance(id);
        let cycles = inst.step_cycles(budget);
        let mut publish_frame = |inst: &mut Instance| -> bool {
            let sink = &inst.sink;
            let core = &mut inst.core;
            let (width, height) = sink.dimensions();
            let expected_len = usize::from(width)
                .saturating_mul(usize::from(height))
                .saturating_mul(4);
            if let Some(span) = sink.produce_frame(expected_len, |buf| match core {
                AnyCore::Scalar(core) => core.take_frame(buf),
            }) {
                let frame_id = inst.bump_frame_id();
                out.push(KernelRep::LaneFrame {
                    group: id,
                    lane: 0,
                    span: FrameSpan {
                        width,
                        height,
                        pixels: Arc::from(&[][..]),
                        slot_span: Some(span),
                    },
                    frame_id,
                });
                true
            } else {
                false
            }
        };

        let mut published = false;
        if inst.frame_ready() {
            published = publish_frame(inst);
        }

        if !published {
            publish_frame(inst);
        }
        out.push(KernelRep::TickDone {
            group: id,
            lanes_mask: 0b1,
            cycles_done: cycles,
        });
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
