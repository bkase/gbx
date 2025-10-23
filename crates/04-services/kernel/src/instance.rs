use crate::sink_transport::TransportFrameSink;
use core::num::NonZeroUsize;
use core::simd::{LaneCount, SupportedLaneCount};
use kernel_core::bus::IoRegs;
use kernel_core::Exec;
use kernel_core::{BusScalar, BusSimd, Core, Model, Scalar, SimdCore, SimdExec};
use service_abi::{CpuVM, InspectorVMMinimal, MemSpace, PpuVM, TimersVM};
use std::sync::Arc;

/// Execution backend container.
pub enum AnyCore {
    /// Scalar single-instance backend.
    Scalar(Box<Core<Scalar, BusScalar>>),
    /// Two-lane SIMD backend.
    Simd2(Box<Core<SimdExec<2>, BusSimd<2>>>),
    /// Four-lane SIMD backend.
    Simd4(Box<Core<SimdExec<4>, BusSimd<4>>>),
}

impl AnyCore {
    pub fn step_cycles(&mut self, budget: u32) -> u32 {
        match self {
            AnyCore::Scalar(core) => core.step_cycles(budget),
            AnyCore::Simd2(core) => core.step_cycles(budget),
            AnyCore::Simd4(core) => core.step_cycles(budget),
        }
    }

    pub fn step_instruction(&mut self) -> (u32, u16) {
        match self {
            AnyCore::Scalar(core) => core.step_instruction(),
            AnyCore::Simd2(core) => core.step_instruction(),
            AnyCore::Simd4(core) => core.step_instruction(),
        }
    }

    pub fn frame_ready(&self) -> bool {
        match self {
            AnyCore::Scalar(core) => core.frame_ready(),
            AnyCore::Simd2(core) => core.frame_ready(),
            AnyCore::Simd4(core) => core.frame_ready(),
        }
    }

    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        match self {
            AnyCore::Scalar(core) => core.load_rom(rom),
            AnyCore::Simd2(core) => core.load_rom(rom),
            AnyCore::Simd4(core) => core.load_rom(rom),
        }
    }

    pub fn reset_post_boot(&mut self, model: Model) {
        match self {
            AnyCore::Scalar(core) => core.reset_post_boot(model),
            AnyCore::Simd2(core) => core.reset_post_boot(model),
            AnyCore::Simd4(core) => core.reset_post_boot(model),
        }
    }
}

fn inspector_from_simd<const LANES: usize>(
    core: &Core<SimdExec<LANES>, BusSimd<LANES>>,
) -> InspectorVMMinimal
where
    LaneCount<LANES>: SupportedLaneCount,
{
    let cpu = &core.cpu;
    let bus = core.bus.lane(0);
    let cpu_vm = CpuVM {
        a: SimdExec::<LANES>::to_u8(cpu.a),
        f: cpu.f.to_byte(),
        b: SimdExec::<LANES>::to_u8(cpu.b),
        c: SimdExec::<LANES>::to_u8(cpu.c),
        d: SimdExec::<LANES>::to_u8(cpu.d),
        e: SimdExec::<LANES>::to_u8(cpu.e),
        h: SimdExec::<LANES>::to_u8(cpu.h),
        l: SimdExec::<LANES>::to_u8(cpu.l),
        sp: SimdExec::<LANES>::to_u16(cpu.sp),
        pc: SimdExec::<LANES>::to_u16(cpu.pc),
        ime: cpu.ime,
        halted: cpu.halted,
    };
    let stat = bus.io.read(IoRegs::STAT);
    let ppu = PpuVM {
        ly: bus.io.read(IoRegs::LY),
        mode: stat & 0x03,
        stat,
        lcdc: bus.io.read(IoRegs::LCDC),
        scx: bus.io.read(IoRegs::SCX),
        scy: bus.io.read(IoRegs::SCY),
        wy: bus.io.read(IoRegs::WY),
        wx: bus.io.read(IoRegs::WX),
        bgp: bus.io.read(IoRegs::BGP),
        frame_ready: core.ppu.frame_ready(),
    };
    let timers = TimersVM {
        div: bus.io.div(),
        tima: bus.io.tima(),
        tma: bus.io.tma(),
        tac: bus.io.tac(),
    };
    let io = bus.io.regs().to_vec();
    InspectorVMMinimal {
        cpu: cpu_vm,
        ppu,
        timers,
        io,
    }
}

fn mem_window_simd<const LANES: usize>(
    core: &Core<SimdExec<LANES>, BusSimd<LANES>>,
    space: MemSpace,
    base: u16,
    len: u16,
) -> Vec<u8>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    let bus = core.bus.lane(0);
    match space {
        MemSpace::Vram => window_slice(bus.vram.as_ref(), 0x8000, base, len),
        MemSpace::Wram => window_slice(bus.wram.as_ref(), 0xC000, base, len),
        MemSpace::Oam => window_slice(bus.oam.as_ref(), 0xFE00, base, len),
        MemSpace::Io => window_slice(bus.io.regs(), 0xFF00, base, len),
    }
}

/// Kernel instance state.
pub struct Instance {
    pub core: AnyCore,
    pub sink: TransportFrameSink,
    pub next_frame_id: u64,
    pub joypad: u8,
    pub lanes: NonZeroUsize,
}

impl Instance {
    pub fn new_scalar(core: Core<Scalar, BusScalar>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Scalar(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(1).unwrap(),
        }
    }

    pub fn new_simd2(core: SimdCore<2>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Simd2(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(2).unwrap(),
        }
    }

    pub fn new_simd4(core: SimdCore<4>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Simd4(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(4).unwrap(),
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
            AnyCore::Simd2(core) => core.bus.set_inputs(joypad),
            AnyCore::Simd4(core) => core.bus.set_inputs(joypad),
        }
    }

    pub fn pc(&self) -> u16 {
        match &self.core {
            AnyCore::Scalar(core) => Scalar::to_u16(core.cpu.pc),
            AnyCore::Simd2(core) => SimdExec::<2>::to_u16(core.cpu.pc),
            AnyCore::Simd4(core) => SimdExec::<4>::to_u16(core.cpu.pc),
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
            AnyCore::Simd2(core) => inspector_from_simd::<2>(core),
            AnyCore::Simd4(core) => inspector_from_simd::<4>(core),
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
            AnyCore::Simd2(core) => mem_window_simd::<2>(core, space, base, len),
            AnyCore::Simd4(core) => mem_window_simd::<4>(core, space, base, len),
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
