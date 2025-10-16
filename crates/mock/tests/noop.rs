//! Smoke test ensuring no-op service handles accept work and drain cleanly.

use std::sync::Arc;

use hub::{Service, SubmitOutcome, TickPurpose};
use mock::{NoopAudio, NoopFs, NoopGpu, NoopKernel};
use world::{AudioCmd, AudioSpan, FrameSpan, FsCmd, GpuCmd, KernelCmd};

/// Confirms the mock service handles always accept input and yield empty drains.
#[test]
fn noop_services_submit_and_drain() {
    let kernel = NoopKernel;
    let kernel_cmd = KernelCmd::Tick {
        group: 0,
        purpose: TickPurpose::Display,
        budget: 0,
    };
    assert_eq!(kernel.try_submit(&kernel_cmd), SubmitOutcome::Accepted);
    assert!(kernel.drain(4).is_empty());

    let fs = NoopFs;
    let fs_cmd = FsCmd::Persist {
        path: "slot".into(),
        bytes: Arc::<[u8]>::from(&b""[..]),
    };
    assert_eq!(fs.try_submit(&fs_cmd), SubmitOutcome::Accepted);
    assert!(fs.drain(4).is_empty());

    let gpu = NoopGpu;
    let gpu_cmd = GpuCmd::UploadFrame {
        lane: 0,
        span: FrameSpan::default(),
    };
    assert_eq!(gpu.try_submit(&gpu_cmd), SubmitOutcome::Accepted);
    assert!(gpu.drain(4).is_empty());

    let audio = NoopAudio;
    let audio_cmd = AudioCmd::Submit {
        span: AudioSpan::default(),
    };
    assert_eq!(audio.try_submit(&audio_cmd), SubmitOutcome::Accepted);
    assert!(audio.drain(4).is_empty());
}
