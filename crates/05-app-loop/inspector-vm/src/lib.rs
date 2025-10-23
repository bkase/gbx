//! Inspector view-model structures shared by CLI, logging, and web debug front-ends.

use serde::Serialize;
use service_abi::{
    CpuVM, DebugRep, InspectorVMMinimal, MemSpace, PpuVM, StepKind, TimersVM, TraceVM,
};

/// Inspector view-model shared across CLI, logfile, and web frontends.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InspectorVM {
    /// CPU register and control state.
    pub cpu: CpuVM,
    /// PPU registers and raster state.
    pub ppu: PpuVM,
    /// Timer register snapshot.
    pub timers: TimersVM,
    /// Memory windows exposed to the front ends.
    pub mem: MemVM,
    /// Aggregated world performance counters.
    pub perf: PerfVM,
    /// Transport metrics for each service endpoint.
    pub transport: TransportVM,
    /// Last disassembly trace emitted by stepping.
    pub disasm: Option<TraceVM>,
}

impl Default for InspectorVM {
    fn default() -> Self {
        Self {
            cpu: CpuVM {
                a: 0,
                f: 0,
                b: 0,
                c: 0,
                d: 0,
                e: 0,
                h: 0,
                l: 0,
                sp: 0,
                pc: 0,
                ime: false,
                halted: false,
            },
            ppu: PpuVM {
                ly: 0,
                mode: 0,
                stat: 0,
                lcdc: 0,
                scx: 0,
                scy: 0,
                wy: 0,
                wx: 0,
                bgp: 0,
                frame_ready: false,
            },
            timers: TimersVM {
                div: 0,
                tima: 0,
                tma: 0,
                tac: 0,
            },
            mem: MemVM::default(),
            perf: PerfVM::default(),
            transport: TransportVM::default(),
            disasm: None,
        }
    }
}

impl InspectorVM {
    /// Construct a new inspector view-model with zeroed state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a minimal snapshot emitted by the kernel.
    pub fn apply_snapshot(&mut self, snapshot: &InspectorVMMinimal) {
        self.cpu = snapshot.cpu.clone();
        self.ppu = snapshot.ppu.clone();
        self.timers = snapshot.timers.clone();
        if self.mem.io.len() != 0x80 {
            self.mem.io.resize(0x80, 0);
        }
        for byte in self.mem.io.iter_mut() {
            *byte = 0;
        }
        for (dst, src) in self.mem.io.iter_mut().zip(snapshot.io.iter()) {
            *dst = *src;
        }
    }

    /// Applies a debug report payload to the view-model.
    pub fn apply_debug_rep(&mut self, rep: &DebugRep) {
        match rep {
            DebugRep::Snapshot(snapshot) => self.apply_snapshot(snapshot),
            DebugRep::MemWindow { space, base, bytes } => {
                self.mem.apply_window(*space, *base, bytes.as_ref());
            }
            DebugRep::Stepped {
                kind,
                cycles,
                pc,
                disasm,
            } => {
                let trace = TraceVM {
                    last_pc: *pc,
                    disasm_line: disasm.clone().unwrap_or_default(),
                    cycles: *cycles,
                };
                match kind {
                    StepKind::Instruction | StepKind::Frame => {
                        self.disasm = Some(trace);
                    }
                }
            }
        }
    }

    /// Serializes the view-model to a single NDJSON line.
    pub fn to_ndjson_line(&self) -> serde_json::Result<String> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }
}

/// Memory view captured by the inspector.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemVM {
    /// Snapshot of the IO register region.
    pub io: Vec<u8>,
    /// Optional VRAM window requested by the UI.
    pub vram_window: Option<MemWindow>,
    /// Optional OAM window requested by the UI.
    pub oam_window: Option<MemWindow>,
    /// Optional WRAM window requested by the UI.
    pub wram_window: Option<MemWindow>,
}

impl MemVM {
    /// Apply a memory window update for the provided address space.
    pub fn apply_window(&mut self, space: MemSpace, base: u16, bytes: &[u8]) {
        let window = MemWindow {
            base,
            bytes: bytes.to_vec(),
        };
        match space {
            MemSpace::Vram => self.vram_window = Some(window),
            MemSpace::Wram => self.wram_window = Some(window),
            MemSpace::Oam => self.oam_window = Some(window),
            MemSpace::Io => {
                let start = base.saturating_sub(0xFF00) as usize;
                for (idx, value) in bytes.iter().enumerate() {
                    let slot = start + idx;
                    if slot >= self.io.len() {
                        self.io.resize(slot + 1, 0);
                    }
                    self.io[slot] = *value;
                }
            }
        }
    }
}

impl Default for MemVM {
    fn default() -> Self {
        Self {
            io: vec![0; 0x80],
            vram_window: None,
            oam_window: None,
            wram_window: None,
        }
    }
}

/// Memory window slice returned by the kernel.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemWindow {
    /// Base address of the window.
    pub base: u16,
    /// Raw bytes captured for the requested region.
    pub bytes: Vec<u8>,
}

/// Aggregated performance counters from the world.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PerfVM {
    /// Last frame identifier presented on the display lane.
    pub last_frame_id: u64,
    /// Total audio underruns observed since startup.
    pub audio_underruns: u64,
}

/// Transport metrics tracked per service port.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TransportVM {
    /// Kernel transport counters.
    pub kernel: PortMetricsVM,
    /// Filesystem transport counters.
    pub fs: PortMetricsVM,
    /// GPU transport counters.
    pub gpu: PortMetricsVM,
    /// Audio transport counters.
    pub audio: PortMetricsVM,
}

/// Per-port transport counters.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PortMetricsVM {
    /// Number of commands accepted.
    pub accepted: u32,
    /// Number of commands coalesced.
    pub coalesced: u32,
    /// Number of commands dropped.
    pub dropped: u32,
    /// Number of commands that would block.
    pub would_block: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_abi::{DebugRep, MemSpace};

    fn sample_snapshot() -> InspectorVMMinimal {
        InspectorVMMinimal {
            cpu: CpuVM {
                a: 0x12,
                f: 0xB0,
                b: 0x01,
                c: 0x02,
                d: 0x03,
                e: 0x04,
                h: 0x05,
                l: 0x06,
                sp: 0xC000,
                pc: 0x0100,
                ime: true,
                halted: false,
            },
            ppu: PpuVM {
                ly: 0x90,
                mode: 1,
                stat: 0x85,
                lcdc: 0x91,
                scx: 0x10,
                scy: 0x20,
                wy: 0x00,
                wx: 0x07,
                bgp: 0xE4,
                frame_ready: true,
            },
            timers: TimersVM {
                div: 0x12,
                tima: 0x34,
                tma: 0x56,
                tac: 0x07,
            },
            io: vec![0xAA; 0x80],
        }
    }

    #[test]
    fn apply_snapshot_replaces_core_sections() {
        let snapshot = sample_snapshot();
        let mut vm = InspectorVM::default();
        vm.mem.io.fill(0);

        vm.apply_snapshot(&snapshot);

        assert_eq!(vm.cpu.pc, 0x0100);
        assert_eq!(vm.ppu.lcdc, 0x91);
        assert_eq!(vm.timers.tima, 0x34);
        assert_eq!(vm.mem.io[0], 0xAA);
    }

    #[test]
    fn apply_mem_window_tracks_spaces() {
        let mut mem = MemVM::default();
        mem.apply_window(MemSpace::Vram, 0x8000, &[1, 2, 3]);
        mem.apply_window(MemSpace::Oam, 0xFE00, &[4, 5]);
        mem.apply_window(MemSpace::Io, 0xFF10, &[6, 7]);

        assert_eq!(mem.vram_window.as_ref().unwrap().bytes, vec![1, 2, 3]);
        assert_eq!(mem.oam_window.as_ref().unwrap().base, 0xFE00);
        assert_eq!(mem.io[0x10], 6);
        assert_eq!(mem.io[0x11], 7);
    }

    #[test]
    fn apply_debug_rep_updates_disasm_and_mem() {
        let mut vm = InspectorVM::default();
        vm.apply_debug_rep(&DebugRep::MemWindow {
            space: MemSpace::Wram,
            base: 0xC000,
            bytes: vec![0x11, 0x22].into(),
        });

        assert_eq!(vm.mem.wram_window.as_ref().unwrap().bytes, vec![0x11, 0x22]);

        vm.apply_debug_rep(&DebugRep::Stepped {
            kind: StepKind::Instruction,
            cycles: 4,
            pc: 0x0150,
            disasm: Some("NOP".into()),
        });

        let trace = vm.disasm.as_ref().expect("trace");
        assert_eq!(trace.last_pc, 0x0150);
        assert_eq!(trace.disasm_line, "NOP");
    }

    #[test]
    fn to_ndjson_line_contains_newline() {
        let line = InspectorVM::default().to_ndjson_line().unwrap();
        assert!(line.ends_with('\n'));
    }
}
