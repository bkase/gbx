//! Command-line utility for exercising the Phase A debug inspector.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use inspector_vm::InspectorVM;
use service_abi::{DebugCmd, DebugRep, KernelCmd, KernelRep, MemSpace, SubmitOutcome};
use services_kernel::default_service;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Text rendering helpers used by the CLI commands.
mod render {
    use inspector_vm::InspectorVM;
    use std::fmt::Write;

    /// Format the CPU/PPU/timer portion of an inspector snapshot.
    pub fn snapshot(vm: &InspectorVM) -> String {
        let mut out = String::new();
        writeln!(
            out,
            "Registers: AF={:04X} BC={:04X} DE={:04X} HL={:04X}",
            ((vm.cpu.a as u16) << 8) | vm.cpu.f as u16,
            ((vm.cpu.b as u16) << 8) | vm.cpu.c as u16,
            ((vm.cpu.d as u16) << 8) | vm.cpu.e as u16,
            ((vm.cpu.h as u16) << 8) | vm.cpu.l as u16
        )
        .expect("write registers");
        writeln!(
            out,
            "SP={:04X} PC={:04X} IME={} HALT={}",
            vm.cpu.sp, vm.cpu.pc, vm.cpu.ime as u8, vm.cpu.halted as u8
        )
        .expect("write cpu");
        writeln!(
            out,
            "PPU: LY={:02X} MODE={} LCDC={:02X} STAT={:02X}",
            vm.ppu.ly, vm.ppu.mode, vm.ppu.lcdc, vm.ppu.stat
        )
        .expect("write ppu");
        writeln!(
            out,
            "Timers: DIV={:02X} TIMA={:02X} TMA={:02X} TAC={:02X}",
            vm.timers.div, vm.timers.tima, vm.timers.tma, vm.timers.tac
        )
        .expect("write timers");
        out
    }

    /// Format a hexdump-style view of a memory window.
    pub fn hexdump(base: u16, bytes: &[u8]) -> String {
        let mut out = String::new();
        let mut offset = 0usize;
        while offset < bytes.len() {
            let line_base = base.wrapping_add(offset as u16);
            write!(out, "{line_base:04X}: ").expect("write prefix");
            for idx in 0..16 {
                if let Some(byte) = bytes.get(offset + idx) {
                    write!(out, "{byte:02X} ").expect("write byte");
                } else {
                    out.push_str("   ");
                }
            }
            out.push('\n');
            offset += 16;
        }
        if bytes.is_empty() {
            writeln!(out, "{base:04X}: ").expect("write empty");
        }
        out
    }

    /// Format the output for stepping instructions.
    pub fn step_instruction(count: u32, cycles: u32, pc: u16) -> String {
        format!("Stepped {count} instruction(s) -> PC={pc:04X} cycles={cycles}\n")
    }

    /// Format the output for stepping a single frame.
    pub fn step_frame(cycles: u32, pc: u16) -> String {
        format!("Advanced one frame -> PC={pc:04X} cycles={cycles}\n")
    }
}

/// Inspect GBX kernels via the Phase A debug pipeline.
#[derive(Parser, Debug)]
#[command(author, version, about = "Interact with the GBX inspector", long_about = None)]
struct Cli {
    /// Path to the ROM to load.
    #[arg(value_name = "ROM")]
    rom: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Capture and print a snapshot of CPU/PPU/timer state.
    Snapshot {
        /// Kernel group identifier (defaults to 0).
        #[arg(short, long, default_value_t = 0)]
        group: u16,
    },
    /// Dump a memory window from VRAM/OAM/WRAM/IO.
    Mem {
        /// Kernel group identifier (defaults to 0).
        #[arg(short, long, default_value_t = 0)]
        group: u16,
        /// Memory space to read.
        #[arg(value_enum)]
        space: MemSpaceArg,
        /// Start address (decimal or hex, e.g. 0x8000).
        #[arg(value_parser = parse_u16, value_name = "BASE")]
        base: u16,
        /// Byte length (decimal or hex).
        #[arg(value_parser = parse_u16, value_name = "LEN")]
        len: u16,
    },
    /// Step N CPU instructions and print the resulting snapshot.
    Step {
        /// Kernel group identifier (defaults to 0).
        #[arg(short, long, default_value_t = 0)]
        group: u16,
        /// Number of instructions to execute (defaults to 1).
        #[arg(value_parser = parse_u32, default_value_t = 1, value_name = "COUNT")]
        count: u32,
    },
    /// Advance one frame worth of cycles and print the resulting snapshot.
    StepFrame {
        /// Kernel group identifier (defaults to 0).
        #[arg(short, long, default_value_t = 0)]
        group: u16,
    },
}

impl Command {
    fn group(&self) -> u16 {
        match *self {
            Command::Snapshot { group }
            | Command::Mem { group, .. }
            | Command::Step { group, .. }
            | Command::StepFrame { group } => group,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MemSpaceArg {
    /// Video RAM window (0x8000-0x9FFF).
    Vram,
    /// Work RAM window (0xC000-0xDFFF).
    Wram,
    /// Object attribute memory (0xFE00-0xFE9F).
    Oam,
    /// I/O register window (0xFF00-0xFF7F).
    Io,
}

impl From<MemSpaceArg> for MemSpace {
    fn from(arg: MemSpaceArg) -> Self {
        match arg {
            MemSpaceArg::Vram => MemSpace::Vram,
            MemSpaceArg::Wram => MemSpace::Wram,
            MemSpaceArg::Oam => MemSpace::Oam,
            MemSpaceArg::Io => MemSpace::Io,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let bytes = load_rom_bytes(&cli.rom)?;
    let kernel = default_service();

    let group = cli.command.group();
    load_rom(&kernel, group, Arc::clone(&bytes))?;

    match cli.command {
        Command::Snapshot { group } => handle_snapshot(&kernel, group)?,
        Command::Mem {
            group,
            space,
            base,
            len,
        } => handle_mem(&kernel, group, space.into(), base, len)?,
        Command::Step { group, count } => {
            handle_step_instructions(&kernel, group, count)?;
            handle_snapshot(&kernel, group)?;
        }
        Command::StepFrame { group } => {
            handle_step_frame(&kernel, group)?;
            handle_snapshot(&kernel, group)?;
        }
    }

    Ok(())
}

fn load_rom_bytes(path: &Path) -> Result<Arc<[u8]>> {
    let data = fs::read(path).with_context(|| format!("failed to read ROM {path:?}"))?;
    Ok(Arc::from(data.into_boxed_slice()))
}

fn load_rom(kernel: &service_abi::KernelServiceHandle, group: u16, bytes: Arc<[u8]>) -> Result<()> {
    submit(kernel, KernelCmd::LoadRom { group, bytes })?;
    let reports = kernel.drain(8);
    if !reports
        .iter()
        .any(|rep| matches!(rep, KernelRep::RomLoaded { .. }))
    {
        bail!("kernel did not acknowledge ROM load");
    }
    Ok(())
}

fn handle_snapshot(kernel: &service_abi::KernelServiceHandle, group: u16) -> Result<()> {
    let debug = issue_debug(kernel, DebugCmd::Snapshot { group })?;
    match debug {
        DebugRep::Snapshot(snapshot) => {
            let mut vm = InspectorVM::default();
            vm.apply_snapshot(&snapshot);
            print_snapshot(&vm);
            Ok(())
        }
        other => bail!("unexpected debug payload: {other:?}"),
    }
}

fn handle_mem(
    kernel: &service_abi::KernelServiceHandle,
    group: u16,
    space: MemSpace,
    base: u16,
    len: u16,
) -> Result<()> {
    let debug = issue_debug(
        kernel,
        DebugCmd::MemWindow {
            group,
            space,
            base,
            len,
        },
    )?;
    match debug {
        DebugRep::MemWindow { bytes, .. } => {
            print_hexdump(base, bytes.as_ref());
            Ok(())
        }
        other => bail!("unexpected debug payload: {other:?}"),
    }
}

fn handle_step_instructions(
    kernel: &service_abi::KernelServiceHandle,
    group: u16,
    count: u32,
) -> Result<()> {
    let debug = issue_debug(kernel, DebugCmd::StepInstruction { group, count })?;
    match debug {
        DebugRep::Stepped { cycles, pc, .. } => {
            print!("{}", render::step_instruction(count, cycles, pc));
            Ok(())
        }
        other => bail!("unexpected debug payload: {other:?}"),
    }
}

fn handle_step_frame(kernel: &service_abi::KernelServiceHandle, group: u16) -> Result<()> {
    let reports = issue_debug_bulk(kernel, DebugCmd::StepFrame { group })?;
    for rep in &reports {
        if let KernelRep::LaneFrame { frame_id, .. } = rep {
            println!("Frame {frame_id} ready");
        }
    }
    if let Some(DebugRep::Stepped { pc, cycles, .. }) =
        reports.into_iter().find_map(|rep| match rep {
            KernelRep::Debug(debug) => Some(debug),
            _ => None,
        })
    {
        print!("{}", render::step_frame(cycles, pc));
        Ok(())
    } else {
        bail!("step-frame did not return debug payload");
    }
}

fn print_snapshot(vm: &InspectorVM) {
    print!("{}", render::snapshot(vm));
}

fn print_hexdump(base: u16, bytes: &[u8]) {
    print!("{}", render::hexdump(base, bytes));
}

fn issue_debug(kernel: &service_abi::KernelServiceHandle, cmd: DebugCmd) -> Result<DebugRep> {
    submit(kernel, KernelCmd::Debug(cmd))?;
    let reports = kernel.drain(32);
    reports
        .into_iter()
        .find_map(|rep| match rep {
            KernelRep::Debug(debug) => Some(debug),
            _ => None,
        })
        .ok_or_else(|| anyhow!("debug command produced no payload"))
}

fn issue_debug_bulk(
    kernel: &service_abi::KernelServiceHandle,
    cmd: DebugCmd,
) -> Result<Vec<KernelRep>> {
    submit(kernel, KernelCmd::Debug(cmd))?;
    Ok(kernel.drain(32).into_vec())
}

fn submit(kernel: &service_abi::KernelServiceHandle, cmd: KernelCmd) -> Result<()> {
    match kernel.try_submit(&cmd) {
        SubmitOutcome::Accepted | SubmitOutcome::Coalesced => Ok(()),
        SubmitOutcome::Dropped => bail!("command {cmd:?} dropped"),
        SubmitOutcome::WouldBlock => bail!("command {cmd:?} would block"),
        SubmitOutcome::Closed => bail!("kernel service closed"),
    }
}

fn parse_u16(input: &str) -> Result<u16, String> {
    if let Some(stripped) = input.strip_prefix("0x") {
        u16::from_str_radix(stripped, 16).map_err(|_| format!("invalid hex value '{input}'"))
    } else {
        input
            .parse::<u16>()
            .map_err(|_| format!("invalid number '{input}'"))
    }
}

fn parse_u32(input: &str) -> Result<u32, String> {
    if let Some(stripped) = input.strip_prefix("0x") {
        u32::from_str_radix(stripped, 16).map_err(|_| format!("invalid hex value '{input}'"))
    } else {
        input
            .parse::<u32>()
            .map_err(|_| format!("invalid number '{input}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::render;
    use inspector_vm::{InspectorVM, MemVM, PerfVM, PortMetricsVM, TransportVM};
    use insta::assert_snapshot;

    fn sample_vm() -> InspectorVM {
        let mut vm = InspectorVM::default();
        vm.cpu.a = 0x12;
        vm.cpu.f = 0xB0;
        vm.cpu.sp = 0xC000;
        vm.cpu.pc = 0x0200;
        vm.cpu.ime = true;
        vm.ppu.mode = 1;
        vm.ppu.lcdc = 0x91;
        vm.ppu.stat = 0x85;
        vm.timers.div = 0x12;
        vm.timers.tima = 0x34;
        vm.mem = MemVM::default();
        vm.perf = PerfVM {
            last_frame_id: 7,
            audio_underruns: 1,
        };
        vm.transport = TransportVM {
            kernel: PortMetricsVM {
                accepted: 1,
                coalesced: 2,
                dropped: 3,
                would_block: 4,
            },
            ..TransportVM::default()
        };
        vm
    }

    #[test]
    fn snapshot_render_matches_expectation() {
        assert_snapshot!("snapshot_render", render::snapshot(&sample_vm()));
    }

    #[test]
    fn hexdump_render_matches_expectation() {
        let bytes = [0x00u8, 0x11, 0x22, 0x33, 0x44];
        assert_snapshot!("hexdump_render", render::hexdump(0x8000, &bytes));
    }

    #[test]
    fn step_instruction_render_matches_expectation() {
        assert_snapshot!(
            "step_instruction_render",
            render::step_instruction(3, 12, 0x0150)
        );
    }
}
