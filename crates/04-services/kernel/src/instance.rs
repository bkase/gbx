use crate::sink_transport::TransportFrameSink;
use kernel_core::{BusScalar, Core, Model, Scalar};
use std::sync::Arc;

/// Execution backend container.
pub enum AnyCore {
    /// Scalar single-instance backend.
    Scalar(Core<Scalar, BusScalar>),
}

impl AnyCore {
    pub fn step_cycles(&mut self, budget: u32) -> u32 {
        match self {
            AnyCore::Scalar(core) => core.step_cycles(budget),
        }
    }

    pub fn frame_ready(&self) -> bool {
        match self {
            AnyCore::Scalar(core) => core.frame_ready(),
        }
    }

    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        match self {
            AnyCore::Scalar(core) => core.load_rom(rom),
        }
    }

    pub fn reset_post_boot(&mut self, model: Model) {
        match self {
            AnyCore::Scalar(core) => core.reset_post_boot(model),
        }
    }
}

/// Kernel instance state.
pub struct Instance {
    pub core: AnyCore,
    pub sink: TransportFrameSink,
    pub next_frame_id: u64,
    pub joypad: u8,
}

impl Instance {
    pub fn new_scalar(core: Core<Scalar, BusScalar>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Scalar(core),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
        }
    }

    pub fn step_cycles(&mut self, budget: u32) -> u32 {
        self.core.step_cycles(budget)
    }

    pub fn frame_ready(&self) -> bool {
        self.core.frame_ready()
    }

    pub fn bump_frame_id(&mut self) -> u64 {
        self.next_frame_id = self.next_frame_id.wrapping_add(1);
        self.next_frame_id
    }

    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        self.core.load_rom(rom);
        self.core.reset_post_boot(Model::Dmg);
        self.next_frame_id = 0;
    }

    pub fn set_inputs(&mut self, joypad: u8) {
        self.joypad = joypad;
        match &mut self.core {
            AnyCore::Scalar(core) => core.bus.set_inputs(joypad),
        }
    }
}
