use crate::bus::BusScalar;
use crate::core::Core;
use crate::exec::{Exec, Scalar};
use crate::ppu_stub::PpuStub;
use crate::timers::Timers;

/// Snapshot of a scalar core state used for determinism tests and migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreState {
    /// Serialized CPU register file.
    pub regs: RegsScalar,
    /// Copy of work RAM.
    pub wram: Vec<u8>,
    /// Copy of video RAM.
    pub vram: Vec<u8>,
    /// Copy of object attribute memory.
    pub oam: Vec<u8>,
    /// Copy of high RAM.
    pub hram: Vec<u8>,
    /// Snapshot of IO registers.
    pub io: Vec<u8>,
    /// Interrupt enable register value.
    pub ie: u8,
    /// Timer counters.
    pub timers: TimersState,
    /// Persisted PPU state.
    pub ppu: PpuState,
    /// Cycles accumulated in the current frame.
    pub cycles_this_frame: u32,
    /// Interrupt master enable flag.
    pub ime: bool,
    /// HALT indicator.
    pub halted: bool,
    /// Pending IME enable flag.
    pub enable_ime_pending: bool,
}

/// Scalar register snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegsScalar {
    /// Accumulator value.
    pub a: u8,
    /// Flags register.
    pub f: u8,
    /// `B` register.
    pub b: u8,
    /// `C` register.
    pub c: u8,
    /// `D` register.
    pub d: u8,
    /// `E` register.
    pub e: u8,
    /// `H` register.
    pub h: u8,
    /// `L` register.
    pub l: u8,
    /// Stack pointer.
    pub sp: u16,
    /// Program counter.
    pub pc: u16,
}

/// Timer counters persisted alongside IO registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimersState {
    /// Internal divider counter accumulator.
    pub div_counter: u32,
    /// Internal TIMA counter accumulator.
    pub tima_counter: u32,
}

/// PPU stub persisted state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PpuState {
    /// Dot position within the active scanline.
    pub dot_in_line: u32,
    /// Currently latched LY value.
    pub ly: u8,
    /// Current PPU mode (0–3).
    pub mode: u8,
    /// Cached coincidence flag state.
    pub lyc_equal: bool,
    /// Latched frame-ready flag.
    pub frame_ready: bool,
    /// Whether the LCD controller was previously enabled.
    pub lcd_was_on: bool,
}

impl From<&Timers> for TimersState {
    fn from(t: &Timers) -> Self {
        Self {
            div_counter: t.div_counter,
            tima_counter: t.tima_counter,
        }
    }
}

impl Timers {
    /// Restores timer counters from a persisted state.
    pub fn load_state(&mut self, state: &TimersState) {
        self.div_counter = state.div_counter;
        self.tima_counter = state.tima_counter;
    }
}

impl From<&PpuStub> for PpuState {
    fn from(ppu: &PpuStub) -> Self {
        Self {
            dot_in_line: ppu.dot_in_line,
            ly: ppu.ly,
            mode: ppu.mode,
            lyc_equal: ppu.lyc_equal,
            frame_ready: ppu.frame_ready,
            lcd_was_on: ppu.lcd_was_on,
        }
    }
}

impl PpuStub {
    /// Restores counters from persisted state.
    pub fn load_state(&mut self, state: &PpuState) {
        self.dot_in_line = state.dot_in_line;
        self.ly = state.ly;
        self.mode = state.mode;
        self.lyc_equal = state.lyc_equal;
        self.frame_ready = state.frame_ready;
        self.lcd_was_on = state.lcd_was_on;
    }
}

impl From<&Core<Scalar, BusScalar>> for CoreState {
    fn from(core: &Core<Scalar, BusScalar>) -> Self {
        let cpu = &core.cpu;
        Self {
            regs: RegsScalar {
                a: <Scalar as Exec>::to_u8(cpu.a),
                f: cpu.f.to_byte(),
                b: <Scalar as Exec>::to_u8(cpu.b),
                c: <Scalar as Exec>::to_u8(cpu.c),
                d: <Scalar as Exec>::to_u8(cpu.d),
                e: <Scalar as Exec>::to_u8(cpu.e),
                h: <Scalar as Exec>::to_u8(cpu.h),
                l: <Scalar as Exec>::to_u8(cpu.l),
                sp: <Scalar as Exec>::to_u16(cpu.sp),
                pc: <Scalar as Exec>::to_u16(cpu.pc),
            },
            wram: core.bus.wram.as_ref().to_vec(),
            vram: core.bus.vram.as_ref().to_vec(),
            oam: core.bus.oam.as_ref().to_vec(),
            hram: core.bus.hram.to_vec(),
            io: core.bus.io.regs().to_vec(),
            ie: core.bus.ie,
            timers: TimersState::from(&core.timers),
            ppu: PpuState::from(&core.ppu),
            cycles_this_frame: core.cycles_this_frame,
            ime: cpu.ime,
            halted: cpu.halted,
            enable_ime_pending: cpu.enable_ime_pending,
        }
    }
}

impl Core<Scalar, BusScalar> {
    /// Loads a previously captured state into the core.
    pub fn load_state(&mut self, state: &CoreState) {
        self.cpu.a = <Scalar as Exec>::from_u8(state.regs.a);
        self.cpu.f.from_byte(state.regs.f);
        self.cpu.b = <Scalar as Exec>::from_u8(state.regs.b);
        self.cpu.c = <Scalar as Exec>::from_u8(state.regs.c);
        self.cpu.d = <Scalar as Exec>::from_u8(state.regs.d);
        self.cpu.e = <Scalar as Exec>::from_u8(state.regs.e);
        self.cpu.h = <Scalar as Exec>::from_u8(state.regs.h);
        self.cpu.l = <Scalar as Exec>::from_u8(state.regs.l);
        self.cpu.sp = <Scalar as Exec>::from_u16(state.regs.sp);
        self.cpu.pc = <Scalar as Exec>::from_u16(state.regs.pc);
        self.cpu.ime = state.ime;
        self.cpu.halted = state.halted;
        self.cpu.enable_ime_pending = state.enable_ime_pending;

        self.bus.wram.copy_from_slice(&state.wram);
        self.bus.vram.copy_from_slice(&state.vram);
        self.bus.oam.copy_from_slice(&state.oam);
        self.bus.hram.copy_from_slice(&state.hram);
        self.bus.io.regs_mut().copy_from_slice(&state.io);
        self.bus.ie = state.ie;
        self.bus.joyp_select = self.bus.io.joyp() & 0x30;

        self.timers.load_state(&state.timers);
        self.ppu.load_state(&state.ppu);
        self.cycles_this_frame = state.cycles_this_frame;
    }
}
