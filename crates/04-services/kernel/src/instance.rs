use crate::sink_transport::TransportFrameSink;
use kernel_core::bus::IoRegs;
use kernel_core::exec::Exec;
use kernel_core::{BusScalar, Core, Model, Scalar};
use service_abi::{CpuVM, InspectorVMMinimal, MemSpace, PpuVM, TimersVM};
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

    pub fn step_instruction(&mut self) -> (u32, u16) {
        match self {
            AnyCore::Scalar(core) => core.step_instruction(),
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

    pub fn step_instructions(&mut self, count: u32) -> (u32, u16) {
        let mut total_cycles = 0u32;
        let mut last_pc = self.pc();
        if count == 0 {
            return (0, last_pc);
        }
        for _ in 0..count {
            let (cycles, pc) = self.core.step_instruction();
            total_cycles = total_cycles.wrapping_add(cycles);
            last_pc = pc;
            if cycles == 0 {
                break;
            }
        }
        (total_cycles, last_pc)
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

    pub fn pc(&self) -> u16 {
        match &self.core {
            AnyCore::Scalar(core) => Scalar::to_u16(core.cpu.pc),
        }
    }

    pub fn inspector_snapshot(&self) -> InspectorVMMinimal {
        match &self.core {
            AnyCore::Scalar(core) => {
                let cpu = CpuVM {
                    a: Scalar::to_u8(core.cpu.a),
                    f: core.cpu.f.to_byte(),
                    b: Scalar::to_u8(core.cpu.b),
                    c: Scalar::to_u8(core.cpu.c),
                    d: Scalar::to_u8(core.cpu.d),
                    e: Scalar::to_u8(core.cpu.e),
                    h: Scalar::to_u8(core.cpu.h),
                    l: Scalar::to_u8(core.cpu.l),
                    sp: Scalar::to_u16(core.cpu.sp),
                    pc: Scalar::to_u16(core.cpu.pc),
                    ime: core.cpu.ime,
                    halted: core.cpu.halted,
                };
                let stat = core.bus.io.read(IoRegs::STAT);
                let ppu = PpuVM {
                    ly: core.bus.io.read(IoRegs::LY),
                    mode: stat & 0x03,
                    stat,
                    lcdc: core.bus.io.read(IoRegs::LCDC),
                    scx: core.bus.io.read(IoRegs::SCX),
                    scy: core.bus.io.read(IoRegs::SCY),
                    wy: core.bus.io.read(IoRegs::WY),
                    wx: core.bus.io.read(IoRegs::WX),
                    bgp: core.bus.io.read(IoRegs::BGP),
                    frame_ready: core.ppu.frame_ready(),
                };
                let timers = TimersVM {
                    div: core.bus.io.div(),
                    tima: core.bus.io.tima(),
                    tma: core.bus.io.tma(),
                    tac: core.bus.io.tac(),
                };
                let io = core.bus.io.regs().to_vec();
                InspectorVMMinimal {
                    cpu,
                    ppu,
                    timers,
                    io,
                }
            }
        }
    }

    pub fn mem_window(&self, space: MemSpace, base: u16, len: u16) -> Vec<u8> {
        match &self.core {
            AnyCore::Scalar(core) => match space {
                MemSpace::Vram => window_slice(core.bus.vram.as_ref(), 0x8000, base, len),
                MemSpace::Wram => window_slice(core.bus.wram.as_ref(), 0xC000, base, len),
                MemSpace::Oam => window_slice(core.bus.oam.as_ref(), 0xFE00, base, len),
                MemSpace::Io => window_slice(core.bus.io.regs(), 0xFF00, base, len),
            },
        }
    }
}

fn window_slice(data: &[u8], region_base: u16, base: u16, len: u16) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    if base < region_base {
        return Vec::new();
    }
    let start = usize::from(base - region_base);
    if start >= data.len() {
        return Vec::new();
    }
    let max_len = data.len().saturating_sub(start);
    let take = max_len.min(len as usize);
    data[start..start + take].to_vec()
}
