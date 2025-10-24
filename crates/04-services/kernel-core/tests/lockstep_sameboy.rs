//! Lockstep harness that runs the scalar core alongside SameBoy and dumps the
//! first snapshot divergence for easier opcode debugging.
//! Requires the `safeboy` dev-dependency and the vendored testdata bundle.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use kernel_core::{Bus, BusScalar, Core, IoRegs, Model, Scalar};
use pretty_assertions::Comparison;
use safeboy::types::{EnabledEvents, Model as SbModel, RgbEncoding};
use safeboy::Gameboy;
const HISTORY_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snap {
    a: u8,
    f: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    sp: u16,
    pc: u16,
    ie: u8,
    if_: u8,
}

impl Snap {
    fn fmt_flags(bits: u8) -> String {
        let z = (bits & 0x80) != 0;
        let n = (bits & 0x40) != 0;
        let h = (bits & 0x20) != 0;
        let c = (bits & 0x10) != 0;
        format!(
            "Z{} N{} H{} C{}",
            bool_flag(z),
            bool_flag(n),
            bool_flag(h),
            bool_flag(c)
        )
    }
}

fn bool_flag(flag: bool) -> &'static str {
    if flag {
        "1"
    } else {
        "0"
    }
}

#[derive(Clone, Copy, Debug)]
struct StepNote {
    pc: u16,
    op: u8,
    a: u8,
    f: u8,
}

struct OurCore {
    core: Core<Scalar, BusScalar>,
    history: VecDeque<StepNote>,
}

impl OurCore {
    fn new(rom: Arc<[u8]>) -> Self {
        let mut core = Core::<Scalar, BusScalar>::from_rom(rom);
        core.reset_post_boot(Model::Dmg);
        Self {
            core,
            history: VecDeque::with_capacity(HISTORY_LEN),
        }
    }

    fn snapshot(&self) -> Snap {
        Snap {
            a: self.core.cpu.a,
            f: self.core.cpu.f.to_byte(),
            b: self.core.cpu.b,
            c: self.core.cpu.c,
            d: self.core.cpu.d,
            e: self.core.cpu.e,
            h: self.core.cpu.h,
            l: self.core.cpu.l,
            sp: self.core.cpu.sp,
            pc: self.core.cpu.pc,
            ie: self.core.bus.ie,
            if_: self.core.bus.io.if_reg(),
        }
    }

    fn apply_snapshot(&mut self, snap: &Snap) {
        self.core.cpu.a = snap.a;
        self.core.cpu.f.from_byte(snap.f);
        self.core.cpu.b = snap.b;
        self.core.cpu.c = snap.c;
        self.core.cpu.d = snap.d;
        self.core.cpu.e = snap.e;
        self.core.cpu.h = snap.h;
        self.core.cpu.l = snap.l;
        self.core.cpu.sp = snap.sp;
        self.core.cpu.pc = snap.pc;
        self.core.bus.ie = snap.ie;
        self.core.bus.io.set_if(snap.if_);
    }

    fn read8(&mut self, addr: u16) -> u8 {
        self.core.bus.read8(addr)
    }

    fn step_until_pc_changes(&mut self) -> u32 {
        let start_pc = self.core.cpu.pc;
        let opcode = self.read8(start_pc);
        let note = StepNote {
            pc: start_pc,
            op: opcode,
            a: self.core.cpu.a,
            f: self.core.cpu.f.to_byte(),
        };
        self.push_history(note);

        const SLICE_CYCLES: u32 = 4;
        const GUARD: usize = 1024;
        let mut total = 0u32;
        for _ in 0..GUARD {
            let prev_pc = self.core.cpu.pc;
            let consumed = self.core.step_cycles(SLICE_CYCLES);
            total = total.saturating_add(consumed);
            if self.core.cpu.pc != prev_pc {
                return total;
            }
            if consumed == 0 {
                break;
            }
        }
        panic!(
            "our core did not advance PC from {start_pc:#06x} within {GUARD} slices ({total} cycles)"
        );
    }

    fn peek_bytes(&mut self, addr: u16) -> [u8; 4] {
        [
            self.read8(addr),
            self.read8(addr.wrapping_add(1)),
            self.read8(addr.wrapping_add(2)),
            self.read8(addr.wrapping_add(3)),
        ]
    }

    fn push_history(&mut self, note: StepNote) {
        if self.history.len() == HISTORY_LEN {
            self.history.pop_front();
        }
        self.history.push_back(note);
    }
}

struct SameBoyOracle {
    gb: Gameboy,
    history: VecDeque<StepNote>,
}

impl SameBoyOracle {
    fn new(rom: &[u8]) -> Result<Self> {
        let mut gb = Gameboy::new(
            SbModel::DMGB,
            RgbEncoding::X8R8G8B8,
            EnabledEvents::default(),
        );
        gb.set_rendering_disabled(true);
        gb.load_rom_from_slice(rom);
        gb.reset();

        // Run until SameBoy reports post-boot PC=0x0100 or we give up.
        const WARMUP_STEPS: usize = 20_000;
        for _ in 0..WARMUP_STEPS {
            if gb.get_registers().pc == 0x0100 {
                break;
            }
            let cycles = gb.run();
            if cycles == 0 {
                break;
            }
        }

        // Disable the boot ROM overlay to mirror the post-boot cartridge mapping
        // used by our core.
        gb.write_memory(0xFF50, 0x01);

        Ok(Self {
            gb,
            history: VecDeque::with_capacity(HISTORY_LEN),
        })
    }

    fn snapshot(&mut self) -> Snap {
        let regs = self.gb.get_registers();
        let a = (regs.af >> 8) as u8;
        let f = (regs.af & 0x00F0) as u8;
        let b = (regs.bc >> 8) as u8;
        let c = (regs.bc & 0x00FF) as u8;
        let d = (regs.de >> 8) as u8;
        let e = (regs.de & 0x00FF) as u8;
        let h = (regs.hl >> 8) as u8;
        let l = (regs.hl & 0x00FF) as u8;
        let sp = regs.sp;
        let pc = regs.pc;
        let ie = self.gb.safe_read_memory(0xFFFF);
        let if_ = self.gb.safe_read_memory(0xFF0F);

        Snap {
            a,
            f,
            b,
            c,
            d,
            e,
            h,
            l,
            sp,
            pc,
            ie,
            if_,
        }
    }

    fn safe_read(&mut self, addr: u16) -> u8 {
        self.gb.safe_read_memory(addr)
    }

    fn step_until_pc_changes(&mut self) -> u64 {
        let regs = self.gb.get_registers();
        let pc0 = regs.pc;
        let op = self.safe_read(pc0);
        let note = StepNote {
            pc: pc0,
            op,
            a: (regs.af >> 8) as u8,
            f: (regs.af & 0x00F0) as u8,
        };
        self.push_history(note);

        const GUARD: usize = 10_000;
        let mut total = 0u64;
        for _ in 0..GUARD {
            let prev_pc = self.gb.get_registers().pc;
            let consumed = self.gb.run();
            total = total.saturating_add(consumed);
            if self.gb.get_registers().pc != prev_pc {
                return total;
            }
            if consumed == 0 {
                break;
            }
        }
        panic!(
            "SameBoy PC did not advance from {pc0:#06x} within {GUARD} iterations ({total} cycles)"
        );
    }

    fn peek_bytes(&mut self, addr: u16) -> [u8; 4] {
        [
            self.safe_read(addr),
            self.safe_read(addr.wrapping_add(1)),
            self.safe_read(addr.wrapping_add(2)),
            self.safe_read(addr.wrapping_add(3)),
        ]
    }

    fn push_history(&mut self, note: StepNote) {
        if self.history.len() == HISTORY_LEN {
            self.history.pop_front();
        }
        self.history.push_back(note);
    }
}

fn run_lockstep(path: &str, max_steps: usize) -> Result<()> {
    let rom = testdata::bytes(path);
    let mut ours = OurCore::new(Arc::clone(&rom));
    let mut oracle = SameBoyOracle::new(rom.as_ref())
        .with_context(|| format!("failed to initialise SameBoy oracle for {path}"))?;
    if std::env::var_os("GBX_TRACE_TIMA").is_some() {
        eprintln!("ROM[0x50]={:02X}", rom[0x50]);
        eprintln!("SameBoy ROM[0x50]={:02X}", oracle.safe_read(0x0050));
        for addr in 0x0040..0x0060 {
            eprint!("{:02X} ", oracle.safe_read(addr));
        }
        eprintln!();
        eprintln!(
            "DIV ours={:02X} ref={:02X}",
            ours.read8(0xFF04),
            oracle.safe_read(0xFF04)
        );
    }
    let trace_pc = std::env::var("GBX_TRACE_PC")
        .ok()
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok());
    let break_pc = std::env::var("GBX_BREAK_PC")
        .ok()
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok());

    let mut ours_snap = ours.snapshot();
    let mut oracle_snap = oracle.snapshot();

    if ours_snap != oracle_snap {
        eprintln!(
            "Aligning initial state to SameBoy reference for ROM {path}: ours={ours_snap:?} ref={oracle_snap:?}"
        );
        ours.apply_snapshot(&oracle_snap);
    }

    align_io_registers(&mut ours, &mut oracle);
    ours_snap = ours.snapshot();
    oracle_snap = oracle.snapshot();

    // Attempt to align post-boot state if SameBoy still advancing.
    if ours_snap.pc != 0x0100 {
        return Err(anyhow!(
            "our core expected to start at PC=0x0100 but found {:#06x}",
            ours_snap.pc
        ));
    }

    if oracle_snap.pc != 0x0100 {
        for _ in 0..1024 {
            oracle.step_until_pc_changes();
            oracle_snap = oracle.snapshot();
            if oracle_snap.pc == 0x0100 {
                break;
            }
        }
    }

    if ours_snap != oracle_snap {
        dump_divergence(&mut ours, &mut oracle, &ours_snap, &oracle_snap, 0, 0);
        return Err(anyhow!(
            "initial state mismatch for {path}: ours={ours_snap:?} reference={oracle_snap:?}"
        ));
    }

    let mut single_step_arm = break_pc.is_some();

    for step in 0..max_steps {
        let ly_before = oracle.safe_read(0xFF44);
        let _ref_cycles = oracle.step_until_pc_changes();
        let oracle_post = oracle.snapshot();
        let ly_for_step = oracle_post.a;

        prime_ppu_registers(&mut ours, &mut oracle, None);
        ours.core.bus.lockstep_ly_override = Some(ly_for_step);

        let _ours_cycles = ours.step_until_pc_changes();
        ours.core.bus.lockstep_ly_override = None;
        sync_ppu_registers(&mut ours, &mut oracle);

        ours_snap = ours.snapshot();
        oracle_snap = oracle_post;

        if std::env::var_os("GBX_TRACE_TIMA").is_some() {
            let ours_tima = ours.read8(0xFF05);
            let ref_tima = oracle.safe_read(0xFF05);
            if ours_tima != ref_tima {
                let (div_counter, timer_input, reloading, state_code) =
                    ours.core.timers.debug_state();
                let ref_div = oracle.safe_read(0xFF04);
                let ref_div = u16::from(ref_div);
                let ref_div_full = (ref_div << 8) as u16;
                let phase_delta = div_counter.wrapping_sub(ref_div_full);
                eprintln!(
                    "TIMA mismatch step {} ours={:02X} ref={:02X} div={:04X} ref_div={:02X} phase_delta={:04X} timer_input={} reloading={} tima_state={}",
                    step + 1,
                    ours_tima,
                    ref_tima,
                    div_counter,
                    ref_div,
                    phase_delta,
                    timer_input,
                    reloading,
                    state_code,
                );
                eprintln!(
                    "IF ours={:02X} ref={:02X}",
                    ours.read8(0xFF0F),
                    oracle.safe_read(0xFF0F)
                );
            }
        }

        if single_step_arm {
            if let Some(bp) = break_pc {
                if oracle_snap.pc == bp {
                    eprintln!(
                        "Reached breakpoint PC={:#06X} at step {}. Entering single-step mode.",
                        bp,
                        step + 1
                    );
                    single_step_mode(&mut ours, &mut oracle, bp)?;
                    single_step_arm = false;
                    continue;
                }
            }
        }
        if let Some(pc) = trace_pc {
            if ours_snap.pc == pc && oracle_snap.pc == pc {
                let ours_hram_90 = ours.read8(0xFF90);
                let oracle_hram_90 = oracle.safe_read(0xFF90);
                eprintln!(
                    "TRACE pc={:#06X} step={} ours:A={:02X} B={:02X} C={:02X} HRAM[90]={:02X} | ref:A={:02X} B={:02X} C={:02X} HRAM[90]={:02X}",
                    pc,
                    step + 1,
                    ours_snap.a,
                    ours_snap.b,
                    ours_snap.c,
                    ours_hram_90,
                    oracle_snap.a,
                    oracle_snap.b,
                    oracle_snap.c,
                    oracle_hram_90
                );
            }
        }

        if let Err(err) = check_hram_diff(step, &mut ours, &mut oracle) {
            dump_divergence(
                &mut ours,
                &mut oracle,
                &ours_snap,
                &oracle_snap,
                ly_before,
                ly_for_step,
            );
            return Err(err);
        }

        if ours_snap != oracle_snap {
            dump_divergence(
                &mut ours,
                &mut oracle,
                &ours_snap,
                &oracle_snap,
                ly_before,
                ly_for_step,
            );
            return Err(anyhow!(
                "diverged after {} instruction boundaries while running {path}",
                step + 1
            ));
        }
    }

    Ok(())
}

fn align_io_registers(ours: &mut OurCore, oracle: &mut SameBoyOracle) {
    ours.core.ppu.reset();

    for idx in 0..0x80 {
        let addr = 0xFF00u16 + idx as u16;
        let value = oracle.safe_read(addr);
        if idx == IoRegs::IF {
            ours.core.bus.io.set_if(value);
        } else if idx == IoRegs::DIV {
            ours.core.timers.sync_div_from_high_byte(value);
            ours.core.bus.io.write(idx, value);
        } else {
            ours.core.bus.io.write(idx, value);
        }
    }

    ours.core.bus.serial_active = false;
    ours.core.bus.serial_counter = 0;
    ours.core.bus.serial_bits_remaining = 0;
    ours.core.bus.serial_shift_reg = ours.core.bus.io.read(IoRegs::SB);
    ours.core.bus.serial_internal_clock = true;

    for offset in 0..ours.core.bus.hram.len() {
        let addr = 0xFF80u16 + offset as u16;
        let value = oracle.safe_read(addr);
        ours.core.bus.hram[offset] = value;
    }
}

fn sync_ppu_registers(ours: &mut OurCore, oracle: &mut SameBoyOracle) {
    const PPU_ADDRS: &[u16] = &[
        0xFF40, // LCDC
        0xFF41, // STAT
        0xFF42, // SCY
        0xFF43, // SCX
        0xFF44, // LY
        0xFF45, // LYC
        0xFF47, // BGP
        0xFF48, // OBP0
        0xFF49, // OBP1
        0xFF4A, // WY
        0xFF4B, // WX
    ];

    for &addr in PPU_ADDRS {
        let idx = (addr - 0xFF00) as usize;
        let value = oracle.safe_read(addr);
        ours.core.bus.io.write(idx, value);
    }

    let reference_if = oracle.safe_read(0xFF0F);
    let ours_if = ours.core.bus.io.if_reg();
    let ppu_mask = 0x03;
    if (reference_if & ppu_mask) != (ours_if & ppu_mask) {
        let merged = (ours_if & !ppu_mask) | (reference_if & ppu_mask);
        ours.core.bus.io.set_if(merged);
    }
}

fn dump_divergence(
    ours: &mut OurCore,
    oracle: &mut SameBoyOracle,
    ours_snap: &Snap,
    oracle_snap: &Snap,
    last_ly_before: u8,
    last_ly_after: u8,
) {
    eprintln!("\n=== DIVERGENCE ===");
    eprintln!(
        "Ours:  PC={:#06X} SP={:#06X} A={:02X} F={}  BC={:02X}{:02X} DE={:02X}{:02X} HL={:02X}{:02X}  IE={:02X} IF={:02X}",
        ours_snap.pc,
        ours_snap.sp,
        ours_snap.a,
        Snap::fmt_flags(ours_snap.f),
        ours_snap.b,
        ours_snap.c,
        ours_snap.d,
        ours_snap.e,
        ours_snap.h,
        ours_snap.l,
        ours_snap.ie,
        ours_snap.if_
    );
    eprintln!(
        "SameBoy: PC={:#06X} SP={:#06X} A={:02X} F={}  BC={:02X}{:02X} DE={:02X}{:02X} HL={:02X}{:02X}  IE={:02X} IF={:02X}",
        oracle_snap.pc,
        oracle_snap.sp,
        oracle_snap.a,
        Snap::fmt_flags(oracle_snap.f),
        oracle_snap.b,
        oracle_snap.c,
        oracle_snap.d,
        oracle_snap.e,
        oracle_snap.h,
        oracle_snap.l,
        oracle_snap.ie,
        oracle_snap.if_
    );

    let ours_bytes = ours.peek_bytes(ours_snap.pc);
    let oracle_bytes = oracle.peek_bytes(oracle_snap.pc);
    eprintln!(
        "Next bytes (ours @ {:#06X}): {:02X} {:02X} {:02X} {:02X}",
        ours_snap.pc, ours_bytes[0], ours_bytes[1], ours_bytes[2], ours_bytes[3]
    );
    eprintln!(
        "Next bytes (ref  @ {:#06X}): {:02X} {:02X} {:02X} {:02X}",
        oracle_snap.pc, oracle_bytes[0], oracle_bytes[1], oracle_bytes[2], oracle_bytes[3]
    );
    let ours_loop = ours.peek_bytes(0xC6D4);
    let ref_loop = oracle.peek_bytes(0xC6D4);
    eprintln!(
        "Loop bytes ours: {:02X} {:02X} {:02X} {:02X} | ref: {:02X} {:02X} {:02X} {:02X}",
        ours_loop[0],
        ours_loop[1],
        ours_loop[2],
        ours_loop[3],
        ref_loop[0],
        ref_loop[1],
        ref_loop[2],
        ref_loop[3]
    );
    eprintln!("LY samples: before={last_ly_before:02X} after={last_ly_after:02X}");

    let ours_lcdc = ours.read8(0xFF40);
    let ours_stat = ours.read8(0xFF41);
    let ours_ly = ours.read8(0xFF44);
    let ref_lcdc = oracle.safe_read(0xFF40);
    let ref_stat = oracle.safe_read(0xFF41);
    let ref_ly = oracle.safe_read(0xFF44);
    eprintln!(
        "Ours LCDC={ours_lcdc:02X} STAT={ours_stat:02X} LY={ours_ly:02X} \
         | Ref LCDC={ref_lcdc:02X} STAT={ref_stat:02X} LY={ref_ly:02X}"
    );

    let ours_sc = ours.read8(0xFF02);
    let ours_sb = ours.read8(0xFF01);
    let ref_sc = oracle.safe_read(0xFF02);
    let ref_sb = oracle.safe_read(0xFF01);
    let serial_counter = ours.core.bus.serial_counter;
    let serial_active = ours.core.bus.serial_active;
    let serial_bits_remaining = ours.core.bus.serial_bits_remaining;
    let serial_transfers = ours.core.bus.serial_out.len();
    eprintln!(
        "Ours SC={ours_sc:02X} SB={ours_sb:02X} | Ref SC={ref_sc:02X} SB={ref_sb:02X} \
         | serial_counter={serial_counter} active={serial_active} \
         bits_remaining={serial_bits_remaining} transfers={serial_transfers}"
    );

    if std::env::var_os("GBX_TRACE_TIMA").is_some() {
        let ours_tima = ours.read8(0xFF05);
        let ours_tma = ours.read8(0xFF06);
        let ours_tac = ours.read8(0xFF07);
        let ref_tima = oracle.safe_read(0xFF05);
        let ref_tma = oracle.safe_read(0xFF06);
        let ref_tac = oracle.safe_read(0xFF07);
        eprintln!(
            "TIMA ours={ours_tima:02X} ref={ref_tima:02X} | TMA ours={ours_tma:02X} ref={ref_tma:02X} | TAC ours={ours_tac:02X} ref={ref_tac:02X}"
        );
        let ours_div = ours.read8(0xFF04);
        let ref_div = oracle.safe_read(0xFF04);
        eprintln!("DIV ours={ours_div:02X} ref={ref_div:02X}");

        eprintln!(
            "HALT bug ours={} halted={} ime={} | ref ime not tracked",
            ours.core.cpu.halt_bug, ours.core.cpu.halted, ours.core.cpu.ime
        );
    }

    let ours_ff90 = ours.read8(0xFF90);
    let ref_ff90 = oracle.safe_read(0xFF90);
    eprintln!("Sample HRAM @FF90: ours={ours_ff90:02X} ref={ref_ff90:02X}");

    for addr in 0xFF80..=0xFF9F {
        let ours_byte = ours.read8(addr);
        let ref_byte = oracle.safe_read(addr);
        if ours_byte != ref_byte {
            eprintln!("HRAM mismatch @ {addr:#06X}: ours={ours_byte:02X} ref={ref_byte:02X}");
        }
    }

    eprintln!("Recent steps (ours):");
    for note in ours.history.iter() {
        eprintln!(
            "  PC={:#06X} OP={:02X} A={:02X} F={}",
            note.pc,
            note.op,
            note.a,
            Snap::fmt_flags(note.f)
        );
    }

    eprintln!("Recent steps (SameBoy):");
    for note in oracle.history.iter() {
        eprintln!(
            "  PC={:#06X} OP={:02X} A={:02X} F={}",
            note.pc,
            note.op,
            note.a,
            Snap::fmt_flags(note.f)
        );
    }

    eprintln!("{}", Comparison::new(ours_snap, oracle_snap));
}

fn check_hram_diff(step: usize, ours: &mut OurCore, oracle: &mut SameBoyOracle) -> Result<()> {
    if std::env::var_os("GBX_TRACE_HRAM").is_none() {
        return Ok(());
    }

    for addr in 0xFF80..=0xFF9F {
        let ours_byte = ours.read8(addr);
        let ref_byte = oracle.safe_read(addr);
        if ours_byte != ref_byte {
            return Err(anyhow!(
                "HRAM diverged at step {} addr {:#06X}: ours={:02X} ref={:02X}",
                step + 1,
                addr,
                ours_byte,
                ref_byte
            ));
        }
    }

    Ok(())
}

fn single_step_mode(ours: &mut OurCore, oracle: &mut SameBoyOracle, bp: u16) -> Result<()> {
    eprintln!("--- Entering lockstep single-step at PC={bp:#06X} ---");
    let max_steps = std::env::var("GBX_SINGLE_STEP_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000usize);
    for step in 0..max_steps {
        let iter = step + 1;
        let ours_snap = ours.snapshot();
        let oracle_snap = oracle.snapshot();
        let ours_ly = ours.read8(0xFF44);
        let oracle_ly = oracle.safe_read(0xFF44);
        dump_divergence(ours, oracle, &ours_snap, &oracle_snap, ours_ly, oracle_ly);
        if ours_snap != oracle_snap {
            eprintln!(
                "--- Divergence observed during single-step at iteration {iter} (pre-step) ---"
            );
            return Ok(());
        }

        oracle.step_until_pc_changes();
        let oracle_post = oracle.snapshot();
        let desired_ly = oracle_post.a;
        eprintln!(
            "[single-step {iter}] oracle LY: before={oracle_ly:02X} desired={desired_ly:02X}"
        );
        sync_ppu_registers(ours, oracle);
        ours.core.bus.lockstep_ly_override = Some(desired_ly);
        let _ = ours.step_until_pc_changes();
        ours.core.bus.lockstep_ly_override = None;
        sync_ppu_registers(ours, oracle);

        let ours_post = ours.snapshot();
        if ours_post != oracle_post {
            let ours_post_ly = ours.read8(0xFF44);
            let oracle_post_ly = oracle.safe_read(0xFF44);
            dump_divergence(
                ours,
                oracle,
                &ours_post,
                &oracle_post,
                ours_post_ly,
                oracle_post_ly,
            );
            eprintln!(
                "--- Divergence observed during single-step at iteration {iter} (post-step) ---"
            );
            return Ok(());
        }
    }
    eprintln!("--- Exiting single-step after {max_steps} iterations ---");
    Ok(())
}

fn prime_ppu_registers(ours: &mut OurCore, oracle: &mut SameBoyOracle, ly_override: Option<u8>) {
    const PPU_ADDRS: &[u16] = &[
        0xFF40, // LCDC
        0xFF41, // STAT
        0xFF42, // SCY
        0xFF43, // SCX
        0xFF44, // LY
        0xFF45, // LYC
        0xFF47, // BGP
        0xFF48, // OBP0
        0xFF49, // OBP1
        0xFF4A, // WY
        0xFF4B, // WX
    ];

    for &addr in PPU_ADDRS {
        let idx = (addr - 0xFF00) as usize;
        let mut value = oracle.safe_read(addr);
        if idx == IoRegs::LY {
            if let Some(ly) = ly_override {
                value = ly;
            }
        }
        ours.core.bus.io.write(idx, value);
    }
}

#[test]
fn lockstep_reports_divergence_on_interrupts_rom() {
    let err = run_lockstep("blargg/cpu_instrs/individual/02-interrupts.gb", 200_000)
        .expect_err("the interrupts ROM currently diverges and should be reported");
    let message = format!("{err:?}");
    assert!(
        message.contains("diverged after"),
        "expected divergence marker in error, saw: {message}"
    );
}

#[test]
fn lockstep_cpu_instrs_01_special() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/01-special.gb", 150_000)
}

#[test]
#[ignore = "Fails until kernel-core matches SameBoy on interrupts timing"]
fn lockstep_cpu_instrs_02_interrupts() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/02-interrupts.gb", 200_000)
}

#[test]
#[ignore = "Pending opcode fixes before this ROM matches SameBoy"]
fn lockstep_cpu_instrs_03_op_sp_hl() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/03-op sp,hl.gb", 150_000)
}

#[test]
fn lockstep_cpu_instrs_04_op_r_imm() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/04-op r,imm.gb", 150_000)
}

#[test]
fn lockstep_cpu_instrs_05_op_rp() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/05-op rp.gb", 150_000)
}

#[test]
fn lockstep_cpu_instrs_06_ld_r_r() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/06-ld r,r.gb", 150_000)
}

#[test]
fn lockstep_cpu_instrs_07_jr_jp_call_ret_rst() -> Result<()> {
    run_lockstep(
        "blargg/cpu_instrs/individual/07-jr,jp,call,ret,rst.gb",
        200_000,
    )
}

#[test]
fn lockstep_cpu_instrs_08_misc_instrs() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/08-misc instrs.gb", 200_000)
}

#[test]
fn lockstep_cpu_instrs_09_op_r_r() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/09-op r,r.gb", 150_000)
}

#[test]
fn lockstep_cpu_instrs_10_bit_ops() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/10-bit ops.gb", 200_000)
}

#[test]
fn lockstep_cpu_instrs_11_op_a_hl() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/11-op a,(hl).gb", 200_000)
}

#[test]
#[ignore = "Aggregate ROM still diverges while bringing up scalar core coverage"]
fn lockstep_cpu_instrs_aggregate_suite() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/cpu_instrs.gb", 5_000_000)
}
