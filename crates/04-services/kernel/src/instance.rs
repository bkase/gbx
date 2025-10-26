use crate::sink_transport::TransportFrameSink;
use core::num::NonZeroUsize;
use core::simd::{LaneCount, SupportedLaneCount};
use kernel_core::bus::IoRegs;
use kernel_core::Exec;
use kernel_core::{BusScalar, BusSimd, Core, Model, Scalar, SimdCore, SimdExec};
use service_abi::{CpuVM, InspectorVMMinimal, MemSpace, PpuVM, SlotSpan, TimersVM};
use std::sync::Arc;

/// Execution backend container.
pub enum AnyCore {
    /// Scalar single-instance backend.
    Scalar(Box<Core<Scalar, BusScalar>>),
    /// Two-lane SIMD backend.
    Simd2(Box<Core<SimdExec<2>, BusSimd<2>>>),
    /// Four-lane SIMD backend.
    Simd4(Box<Core<SimdExec<4>, BusSimd<4>>>),
    /// Eight-lane SIMD backend.
    Simd8(Box<Core<SimdExec<8>, BusSimd<8>>>),
}

impl AnyCore {
    pub fn step_cycles(&mut self, budget: u32) -> u32 {
        match self {
            AnyCore::Scalar(core) => core.step_cycles(budget),
            AnyCore::Simd2(core) => core.step_cycles(budget),
            AnyCore::Simd4(core) => core.step_cycles(budget),
            AnyCore::Simd8(core) => core.step_cycles(budget),
        }
    }

    pub fn step_instruction(&mut self) -> (u32, u16) {
        match self {
            AnyCore::Scalar(core) => core.step_instruction(),
            AnyCore::Simd2(core) => core.step_instruction(),
            AnyCore::Simd4(core) => core.step_instruction(),
            AnyCore::Simd8(core) => core.step_instruction(),
        }
    }

    pub fn frame_ready(&self) -> bool {
        match self {
            AnyCore::Scalar(core) => core.frame_ready(),
            AnyCore::Simd2(core) => core.frame_ready(),
            AnyCore::Simd4(core) => core.frame_ready(),
            AnyCore::Simd8(core) => core.frame_ready(),
        }
    }

    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        match self {
            AnyCore::Scalar(core) => core.load_rom(rom),
            AnyCore::Simd2(core) => core.load_rom(rom),
            AnyCore::Simd4(core) => core.load_rom(rom),
            AnyCore::Simd8(core) => core.load_rom(rom),
        }
    }

    pub fn reset_post_boot(&mut self, model: Model) {
        match self {
            AnyCore::Scalar(core) => core.reset_post_boot(model),
            AnyCore::Simd2(core) => core.reset_post_boot(model),
            AnyCore::Simd4(core) => core.reset_post_boot(model),
            AnyCore::Simd8(core) => core.reset_post_boot(model),
        }
    }

    pub fn reset_power_on(&mut self, model: Model) {
        match self {
            AnyCore::Scalar(core) => core.reset_power_on(model),
            AnyCore::Simd2(core) => core.reset_power_on(model),
            AnyCore::Simd4(core) => core.reset_power_on(model),
            AnyCore::Simd8(core) => core.reset_power_on(model),
        }
    }

    pub fn has_boot_rom(&self) -> bool {
        match self {
            AnyCore::Scalar(core) => core.has_boot_rom(),
            AnyCore::Simd2(core) => core.has_boot_rom(),
            AnyCore::Simd4(core) => core.has_boot_rom(),
            AnyCore::Simd8(core) => core.has_boot_rom(),
        }
    }

    #[cfg(test)]
    pub fn boot_rom_enabled(&self) -> bool {
        match self {
            AnyCore::Scalar(core) => core.boot_rom_enabled(),
            AnyCore::Simd2(core) => core.boot_rom_enabled(),
            AnyCore::Simd4(core) => core.boot_rom_enabled(),
            AnyCore::Simd8(core) => core.boot_rom_enabled(),
        }
    }

    pub fn set_boot_rom_enabled(&mut self, enabled: bool) {
        match self {
            AnyCore::Scalar(core) => core.set_boot_rom_enabled(enabled),
            AnyCore::Simd2(core) => core.set_boot_rom_enabled(enabled),
            AnyCore::Simd4(core) => core.set_boot_rom_enabled(enabled),
            AnyCore::Simd8(core) => core.set_boot_rom_enabled(enabled),
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
    boot: Option<BootSequence>,
}

impl Instance {
    pub fn new_scalar(core: Core<Scalar, BusScalar>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Scalar(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(1).unwrap(),
            boot: None,
        }
    }

    pub fn new_simd2(core: SimdCore<2>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Simd2(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(2).unwrap(),
            boot: None,
        }
    }

    pub fn new_simd4(core: SimdCore<4>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Simd4(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(4).unwrap(),
            boot: None,
        }
    }

    pub fn new_simd8(core: SimdCore<8>, sink: TransportFrameSink) -> Self {
        Self {
            core: AnyCore::Simd8(Box::new(core)),
            sink,
            next_frame_id: 0,
            joypad: 0xFF,
            lanes: NonZeroUsize::new(8).unwrap(),
            boot: None,
        }
    }

    pub fn step_cycles(&mut self, budget: u32) -> u32 {
        if let Some(boot) = self.boot.as_mut() {
            return boot.step(budget);
        }
        self.core.step_cycles(budget)
    }

    pub fn step_instructions(&mut self, count: u32) -> (u32, u16) {
        let mut total_cycles = 0u32;
        let mut last_pc = self.pc();
        if count == 0 {
            return (0, last_pc);
        }
        if self.boot.is_some() {
            self.boot = None;
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
        self.boot.is_none() && self.core.frame_ready()
    }

    pub fn bump_frame_id(&mut self) -> u64 {
        self.next_frame_id = self.next_frame_id.wrapping_add(1);
        self.next_frame_id
    }

    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        let blank_rom = rom.iter().all(|&byte| byte == 0);
        self.core.load_rom(Arc::clone(&rom));
        let using_boot_rom = self.core.has_boot_rom();
        if using_boot_rom && !blank_rom {
            self.core.reset_power_on(Model::Dmg);
            self.core.set_boot_rom_enabled(true);
            self.boot = None;
        } else {
            self.core.reset_post_boot(Model::Dmg);
            self.boot = if blank_rom {
                None
            } else {
                match &mut self.core {
                    AnyCore::Scalar(core) => Some(BootSequence::new(core.as_mut(), rom.as_ref())),
                    AnyCore::Simd2(_) | AnyCore::Simd4(_) | AnyCore::Simd8(_) => None,
                }
            };
        }
        self.next_frame_id = 0;
    }

    pub fn set_inputs(&mut self, joypad: u8) {
        self.joypad = joypad;
        match &mut self.core {
            AnyCore::Scalar(core) => core.bus.set_inputs(joypad),
            AnyCore::Simd2(core) => core.bus.set_inputs(joypad),
            AnyCore::Simd4(core) => core.bus.set_inputs(joypad),
            AnyCore::Simd8(core) => core.bus.set_inputs(joypad),
        }
    }

    pub fn pc(&self) -> u16 {
        match &self.core {
            AnyCore::Scalar(core) => Scalar::to_u16(core.cpu.pc),
            AnyCore::Simd2(core) => SimdExec::<2>::to_u16(core.cpu.pc),
            AnyCore::Simd4(core) => SimdExec::<4>::to_u16(core.cpu.pc),
            AnyCore::Simd8(core) => SimdExec::<8>::to_u16(core.cpu.pc),
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
            AnyCore::Simd8(core) => inspector_from_simd::<8>(core),
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
            AnyCore::Simd8(core) => mem_window_simd::<8>(core, space, base, len),
        }
    }

    fn render_lane_into(&mut self, lane: usize, buf: &mut [u8]) {
        let total_lanes = self.lanes.get();
        assert!(
            lane < total_lanes,
            "lane {} out of range (lanes = {})",
            lane,
            total_lanes
        );
        match (&mut self.boot, &mut self.core) {
            (Some(boot), AnyCore::Scalar(core)) => {
                debug_assert_eq!(lane, 0, "scalar backend only exposes lane 0");
                let finished = boot.render(core.as_mut(), buf);
                if finished {
                    self.boot = None;
                }
            }
            (Some(_), AnyCore::Simd2(core)) => {
                core.take_frame_lane(lane, buf);
                self.boot = None;
            }
            (Some(_), AnyCore::Simd4(core)) => {
                core.take_frame_lane(lane, buf);
                self.boot = None;
            }
            (Some(_), AnyCore::Simd8(core)) => {
                core.take_frame_lane(lane, buf);
                self.boot = None;
            }
            (None, AnyCore::Scalar(core)) => {
                debug_assert_eq!(lane, 0, "scalar backend only exposes lane 0");
                core.take_frame(buf);
            }
            (None, AnyCore::Simd2(core)) => core.take_frame_lane(lane, buf),
            (None, AnyCore::Simd4(core)) => core.take_frame_lane(lane, buf),
            (None, AnyCore::Simd8(core)) => core.take_frame_lane(lane, buf),
        }
    }

    pub fn render_into(&mut self, buf: &mut [u8]) {
        self.render_lane_into(0, buf);
    }

    pub fn produce_frame(&mut self, expected_len: usize) -> Option<(Arc<[u8]>, Option<SlotSpan>)> {
        let this = self as *mut Instance;
        self.sink.produce_frame(expected_len, |buf| {
            // SAFETY: `this` is a raw pointer to `self`. The closure is executed
            // synchronously by `produce_frame`, so no other references to `self`
            // exist while we render the frame into `buf`.
            unsafe { (&mut *this).render_into(buf) }
        })
    }

    pub fn produce_frame_for_lane(
        &mut self,
        expected_len: usize,
        lane: usize,
    ) -> Option<(Arc<[u8]>, Option<SlotSpan>)> {
        let total_lanes = self.lanes.get();
        assert!(
            lane < total_lanes,
            "lane {} out of range (lanes = {})",
            lane,
            total_lanes
        );
        let this = self as *mut Instance;
        self.sink.produce_frame(expected_len, |buf| {
            // SAFETY: `this` is a raw pointer to `self`. The closure is executed
            // synchronously by `produce_frame`, so no other references to `self`
            // exist while we render the frame into `buf`.
            unsafe { (&mut *this).render_lane_into(lane, buf) }
        })
    }

    pub fn boot_active(&self) -> bool {
        self.boot.is_some()
    }

    #[cfg(test)]
    pub fn boot_rom_enabled(&self) -> bool {
        self.core.boot_rom_enabled()
    }
}

struct BootSequence {
    scy: u8,
    phase: BootPhase,
    tiles: Vec<[u8; 64]>,
    map: [[u8; 32]; 32],
}

enum BootPhase {
    Scroll {
        remaining_steps: u16,
        frame_delay: u8,
    },
    Pause {
        frames_left: u16,
    },
    Done,
}

const BOOT_TILE_COUNT: usize = 0x19;
const BOOT_SHADES: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

impl BootSequence {
    fn new(core: &mut Core<Scalar, BusScalar>, rom: &[u8]) -> Self {
        prepare_boot_vram(&mut core.bus, rom);
        core.bus.io.write(IoRegs::BGP, 0xFC);
        core.bus.io.write(IoRegs::SCY, 0x00);
        let tiles = capture_logo_tiles(&core.bus);
        let map = build_logo_map();
        Self {
            scy: 0,
            phase: BootPhase::Scroll {
                remaining_steps: 100,
                frame_delay: 2,
            },
            tiles,
            map,
        }
    }

    fn step(&mut self, budget: u32) -> u32 {
        budget
    }

    fn render(&mut self, core: &mut Core<Scalar, BusScalar>, buf: &mut [u8]) -> bool {
        render_boot_frame(buf, self.scy, &self.tiles, &self.map);
        self.advance(core);
        matches!(self.phase, BootPhase::Done)
    }

    fn advance(&mut self, core: &mut Core<Scalar, BusScalar>) {
        match &mut self.phase {
            BootPhase::Scroll {
                remaining_steps,
                frame_delay,
            } => {
                if *remaining_steps == 0 {
                    self.phase = BootPhase::Pause { frames_left: 64 };
                    return;
                }

                if *frame_delay == 0 {
                    self.scy = self.scy.wrapping_sub(1);
                    core.bus.io.write(IoRegs::SCY, self.scy);
                    *remaining_steps = remaining_steps.saturating_sub(1);
                    *frame_delay = 2;
                    if *remaining_steps == 0 {
                        self.phase = BootPhase::Pause { frames_left: 64 };
                    }
                } else {
                    *frame_delay -= 1;
                }
            }
            BootPhase::Pause { frames_left } => {
                if *frames_left == 0 {
                    self.scy = 0;
                    core.bus.io.write(IoRegs::SCY, 0x00);
                    self.phase = BootPhase::Done;
                } else {
                    *frames_left -= 1;
                }
            }
            BootPhase::Done => {}
        }
    }
}

fn capture_logo_tiles(bus: &BusScalar) -> Vec<[u8; 64]> {
    let mut tiles = vec![[0u8; 64]; BOOT_TILE_COUNT + 1];
    let base = (0x8010 - 0x8000) as usize;
    for (tile_idx, tile) in tiles.iter_mut().enumerate().skip(1).take(BOOT_TILE_COUNT) {
        let offset = base + (tile_idx - 1) * 16;
        if offset + 16 > bus.vram.len() {
            break;
        }
        let tile_bytes = &bus.vram[offset..offset + 16];
        for row in 0..8 {
            let lo = tile_bytes[row * 2];
            let hi = tile_bytes[row * 2 + 1];
            for col in 0..8 {
                let bit = 7 - col;
                let color = ((hi >> bit) & 0x01) << 1 | ((lo >> bit) & 0x01);
                tile[row * 8 + col] = color;
            }
        }
    }
    tiles
}

fn build_logo_map() -> [[u8; 32]; 32] {
    let mut map = [[0u8; 32]; 32];
    map[8][16] = BOOT_TILE_COUNT as u8;
    let mut a = BOOT_TILE_COUNT as u8;
    let mut y = 9usize;
    let mut x = 15usize;
    loop {
        let mut c = 12u8;
        while c > 0 {
            a = a.wrapping_sub(1);
            if a == 0 {
                return map;
            }
            map[y % 32][x % 32] = a;
            if x == 0 {
                x = 31;
                y = y.wrapping_sub(1);
            } else {
                x -= 1;
            }
            c -= 1;
        }
        x = 15;
        y = y.wrapping_sub(1);
    }
}

fn render_boot_frame(buf: &mut [u8], scy: u8, tiles: &[[u8; 64]], map: &[[u8; 32]; 32]) {
    let width = 160usize;
    let height = 144usize;
    for y in 0..height {
        let bg_y = (y + scy as usize) & 0xFF;
        let tile_row = (bg_y / 8) % 32;
        let tile_y = bg_y % 8;
        for x in 0..width {
            let bg_x = x & 0xFF;
            let tile_col = (bg_x / 8) % 32;
            let tile_x = bg_x % 8;
            let tile_id = map[tile_row][tile_col] as usize;
            let color_idx = if tile_id < tiles.len() {
                tiles[tile_id][tile_y * 8 + tile_x] as usize
            } else {
                0
            };
            let offset = (y * width + x) * 4;
            let shade = BOOT_SHADES[color_idx];
            buf[offset] = shade;
            buf[offset + 1] = shade;
            buf[offset + 2] = shade;
            buf[offset + 3] = 0xFF;
        }
    }
}

fn prepare_boot_vram(bus: &mut BusScalar, rom: &[u8]) {
    write_logo_tiles(bus, rom);
    write_trademark_tile(bus);
    write_logo_tilemap(bus);
}

fn write_logo_tiles(bus: &mut BusScalar, rom: &[u8]) {
    let logo_bytes = if rom.len() >= 0x0134 {
        &rom[0x0104..0x0134]
    } else {
        &[]
    };

    let mut addr = 0x8010u16;
    for &byte in logo_bytes {
        let high = byte >> 4;
        write_logo_nibble(bus, addr, high);
        addr = addr.wrapping_add(4);
        let low = byte & 0x0F;
        write_logo_nibble(bus, addr, low);
        addr = addr.wrapping_add(4);
    }
}

fn write_logo_nibble(bus: &mut BusScalar, addr: u16, nibble: u8) {
    let expanded = expand_nibble(nibble & 0x0F);
    let idx = vram_index(addr);
    if idx + 3 >= bus.vram.len() {
        return;
    }
    bus.vram[idx] = expanded;
    bus.vram[idx + 1] = 0;
    bus.vram[idx + 2] = expanded;
    bus.vram[idx + 3] = 0;
}

fn expand_nibble(nibble: u8) -> u8 {
    let b3 = (nibble >> 3) & 1;
    let b2 = (nibble >> 2) & 1;
    let b1 = (nibble >> 1) & 1;
    let b0 = nibble & 1;
    ((b3 * 0b11) << 6) | ((b2 * 0b11) << 4) | ((b1 * 0b11) << 2) | (b0 * 0b11)
}

fn write_trademark_tile(bus: &mut BusScalar) {
    const TRADEMARK: [u8; 8] = [0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C];
    let mut addr = 0x80D0u16;
    for &byte in TRADEMARK.iter() {
        let idx = vram_index(addr);
        if idx >= bus.vram.len() {
            break;
        }
        bus.vram[idx] = byte;
        addr = addr.wrapping_add(2);
    }
}

fn write_logo_tilemap(bus: &mut BusScalar) {
    let mut a = 0x19u8;
    if let Some(idx) = vram_index_checked(0x9910, bus.vram.len()) {
        bus.vram[idx] = a;
    }

    let mut hl = 0x992Fu16;
    'outer: loop {
        let mut c = 0x0Cu8;
        loop {
            a = a.wrapping_sub(1);
            if a == 0 {
                break 'outer;
            }
            if let Some(idx) = vram_index_checked(hl, bus.vram.len()) {
                bus.vram[idx] = a;
            }
            hl = hl.wrapping_sub(1);
            c = c.saturating_sub(1);
            if c == 0 {
                hl = (hl & 0xFF00) | 0x000F;
                break;
            }
        }
    }
}

fn vram_index(addr: u16) -> usize {
    (addr - 0x8000) as usize
}

fn vram_index_checked(addr: u16, len: usize) -> Option<usize> {
    let idx = vram_index(addr);
    if idx < len {
        Some(idx)
    } else {
        None
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
