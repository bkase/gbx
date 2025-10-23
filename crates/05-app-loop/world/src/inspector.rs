use inspector_vm::{InspectorVM, PortMetricsVM};
use service_abi::DebugRep;

/// State container for the inspector view-model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorState {
    /// Latest inspector view-model snapshot.
    pub vm: InspectorVM,
    /// Transport metrics for the kernel service endpoint.
    pub transport_kernel: PortMetricsVM,
    /// Transport metrics for the filesystem endpoint.
    pub transport_fs: PortMetricsVM,
    /// Transport metrics for the GPU endpoint.
    pub transport_gpu: PortMetricsVM,
    /// Transport metrics for the audio endpoint.
    pub transport_audio: PortMetricsVM,
}

impl InspectorState {
    /// Construct a fresh inspector state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a debug report to the underlying view-model.
    pub fn apply_debug_rep(&mut self, rep: &DebugRep) {
        self.vm.apply_debug_rep(rep);
    }

    /// Synchronize world performance counters into the inspector view.
    pub fn sync_perf(&mut self, perf: &crate::world::WorldPerf) {
        self.vm.perf.last_frame_id = perf.last_frame_id;
        self.vm.perf.audio_underruns = perf.audio_underruns;
    }

    /// Update kernel transport metrics in the inspector view.
    pub fn update_transport_kernel(&mut self, metrics: PortMetricsVM) {
        self.transport_kernel = metrics.clone();
        self.vm.transport.kernel = metrics;
    }

    /// Update filesystem transport metrics in the inspector view.
    pub fn update_transport_fs(&mut self, metrics: PortMetricsVM) {
        self.transport_fs = metrics.clone();
        self.vm.transport.fs = metrics;
    }

    /// Update GPU transport metrics in the inspector view.
    pub fn update_transport_gpu(&mut self, metrics: PortMetricsVM) {
        self.transport_gpu = metrics.clone();
        self.vm.transport.gpu = metrics;
    }

    /// Update audio transport metrics in the inspector view.
    pub fn update_transport_audio(&mut self, metrics: PortMetricsVM) {
        self.transport_audio = metrics.clone();
        self.vm.transport.audio = metrics;
    }

    /// Borrow the underlying inspector view-model.
    pub fn vm(&self) -> &InspectorVM {
        &self.vm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_abi::{CpuVM, DebugRep, InspectorVMMinimal, PpuVM, StepKind, TimersVM};

    fn sample_snapshot() -> DebugRep {
        DebugRep::Snapshot(InspectorVMMinimal {
            cpu: CpuVM {
                a: 0x10,
                f: 0xB0,
                b: 0,
                c: 0,
                d: 0,
                e: 0,
                h: 0,
                l: 0,
                sp: 0xC000,
                pc: 0x0200,
                ime: false,
                halted: false,
            },
            ppu: PpuVM {
                ly: 0,
                mode: 0,
                stat: 0x80,
                lcdc: 0x91,
                scx: 0,
                scy: 0,
                wy: 0,
                wx: 0,
                bgp: 0,
                frame_ready: false,
            },
            timers: TimersVM {
                div: 0x12,
                tima: 0x34,
                tma: 0x56,
                tac: 0x07,
            },
            io: vec![0; 0x80],
        })
    }

    #[test]
    fn apply_debug_rep_delegates_to_view_model() {
        let mut state = InspectorState::new();
        state.apply_debug_rep(&sample_snapshot());
        assert_eq!(state.vm.cpu.pc, 0x0200);

        state.apply_debug_rep(&DebugRep::Stepped {
            kind: StepKind::Instruction,
            cycles: 8,
            pc: 0x0204,
            disasm: Some("LD A, (HL)".into()),
        });

        assert_eq!(state.vm.disasm.as_ref().unwrap().last_pc, 0x0204);
    }

    #[test]
    fn sync_perf_updates_view_model_counters() {
        let mut state = InspectorState::new();
        let perf = crate::world::WorldPerf {
            last_frame_id: 77,
            audio_underruns: 3,
        };
        state.sync_perf(&perf);

        assert_eq!(state.vm.perf.last_frame_id, 77);
        assert_eq!(state.vm.perf.audio_underruns, 3);
    }

    #[test]
    fn transport_updates_are_mirrored() {
        let mut state = InspectorState::new();
        let metrics = PortMetricsVM {
            accepted: 1,
            coalesced: 2,
            dropped: 3,
            would_block: 4,
        };
        state.update_transport_gpu(metrics.clone());
        assert_eq!(state.transport_gpu, metrics);
        assert_eq!(state.vm.transport.gpu, metrics);
    }
}
