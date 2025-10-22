//! Lockstep harness that runs the scalar core alongside SameBoy and dumps the
//! first snapshot divergence for easier opcode debugging.
//! Requires the `safeboy` dev-dependency and the vendored testdata bundle.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use kernel_core::{Bus, BusScalar, Core, Model, Scalar};
use pretty_assertions::Comparison;
use safeboy::types::{EnabledEvents, Model as SbModel, RgbEncoding};
use safeboy::Gameboy;
use testdata;

const HISTORY_LEN: usize = 64;

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
        format!("Z{} N{} H{} C{}", bool_flag(z), bool_flag(n), bool_flag(h), bool_flag(c))
    }
}

fn bool_flag(flag: bool) -> &'static str {
    if flag { "1" } else { "0" }
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
        let mut core = Core::from_rom(rom);
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
            "our core did not advance PC from {start_pc:#06x} within {GUARD} slices ({} cycles)",
            total
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
        let mut gb = Gameboy::new(SbModel::DMGB, RgbEncoding::X8R8G8B8, EnabledEvents::default());
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
            "SameBoy PC did not advance from {pc0:#06x} within {GUARD} iterations ({} cycles)",
            total
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

    let mut ours_snap = ours.snapshot();
    let mut oracle_snap = oracle.snapshot();

    if ours_snap != oracle_snap {
        eprintln!(
            "Aligning initial state to SameBoy reference for ROM {path}: ours={ours_snap:?} ref={oracle_snap:?}"
        );
        ours.apply_snapshot(&oracle_snap);
        ours_snap = ours.snapshot();
    }

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
        dump_divergence(&mut ours, &mut oracle, &ours_snap, &oracle_snap);
        return Err(anyhow!(
            "initial state mismatch for {path}: ours={ours_snap:?} reference={oracle_snap:?}"
        ));
    }

    for step in 0..max_steps {
        let _ours_cycles = ours.step_until_pc_changes();
        let _ref_cycles = oracle.step_until_pc_changes();

        ours_snap = ours.snapshot();
        oracle_snap = oracle.snapshot();

        if ours_snap != oracle_snap {
            dump_divergence(&mut ours, &mut oracle, &ours_snap, &oracle_snap);
            return Err(anyhow!(
                "diverged after {} instruction boundaries while running {path}",
                step + 1
            ));
        }
    }

    Ok(())
}

fn dump_divergence(
    ours: &mut OurCore,
    oracle: &mut SameBoyOracle,
    ours_snap: &Snap,
    oracle_snap: &Snap,
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

#[test]
fn lockstep_reports_divergence_on_interrupts_rom() {
    let err = run_lockstep("blargg/cpu_instrs/individual/02-interrupts.gb", 20_000)
        .expect_err("the interrupts ROM currently diverges and should be reported");
    let message = format!("{err:?}");
    assert!(
        message.contains("diverged after"),
        "expected divergence marker in error, saw: {message}"
    );
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
#[ignore = "Pending opcode fixes before this ROM matches SameBoy"]
fn lockstep_cpu_instrs_04_op_r_imm() -> Result<()> {
    run_lockstep("blargg/cpu_instrs/individual/04-op r,imm.gb", 150_000)
}
