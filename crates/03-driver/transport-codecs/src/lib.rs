//! Codec implementations for gbx service messages.
//!
//! This crate provides concrete codec implementations that serialize and deserialize
//! domain-specific command and report types (from `world`) to and from the wire format
//! used by the transport fabric.

#![allow(missing_docs)]

use rkyv::{
    api::high::{access, to_bytes, HighSerializer, HighValidator},
    bytecheck::CheckBytes,
    rancor::Error,
    ser::allocator::ArenaHandle,
    util::AlignedVec,
    Archive, Serialize,
};
use service_abi::{
    AudioCmd, AudioRep, AudioSpan, CpuVM, DebugCmd, DebugRep, FsCmd, FsRep, GpuCmd, GpuRep,
    InspectorVMMinimal, KernelCmd, KernelRep, MemSpace, PpuVM, SlotSpan, StepKind, SubmitPolicy,
    TickPurpose, TimersVM,
};
use std::sync::Arc;
use transport::schema::*;
use transport::Envelope;
use transport_fabric::{Codec, Encoded, FabricError, FabricResult, PortClass};

/// Codec for kernel service commands and reports.
#[derive(Clone, Default)]
pub struct KernelCodec;

impl Codec for KernelCodec {
    type Cmd = KernelCmd;
    type Rep = KernelRep;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded> {
        let tag = TAG_KERNEL_CMD;
        let policy = default_kernel_policy(cmd);
        let schema = match cmd {
            KernelCmd::Tick {
                purpose, budget, ..
            } => KernelCmdV1::Tick(KernelTickCmdV1 {
                purpose: match purpose {
                    TickPurpose::Display => TickPurposeV1::Display,
                    TickPurpose::Exploration => TickPurposeV1::Exploration,
                },
                budget: *budget,
            }),
            KernelCmd::LoadRom { bytes, .. } => KernelCmdV1::LoadRom(KernelLoadRomCmdV1 {
                bytes: bytes.iter().copied().collect(),
            }),
            KernelCmd::SetInputs {
                group,
                lanes_mask,
                joypad,
            } => KernelCmdV1::SetInputs(KernelSetInputsCmdV1 {
                group: *group,
                lanes_mask: *lanes_mask,
                joypad: *joypad,
            }),
            KernelCmd::Terminate { group } => {
                KernelCmdV1::Terminate(KernelTerminateCmdV1 { group: *group })
            }
            KernelCmd::Debug(debug) => KernelCmdV1::Debug(encode_debug_cmd(debug)),
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            policy,
            Envelope::new(tag, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd> {
        ensure_tag(envelope, TAG_KERNEL_CMD)?;
        let archived = archived_root::<KernelCmdV1>(payload)?;
        match archived {
            ArchivedKernelCmdV1::Tick(tick) => Ok(KernelCmd::Tick {
                group: 0,
                purpose: match tick.purpose {
                    ArchivedTickPurposeV1::Display => TickPurpose::Display,
                    ArchivedTickPurposeV1::Exploration => TickPurpose::Exploration,
                },
                budget: tick.budget.to_native(),
            }),
            ArchivedKernelCmdV1::LoadRom(load) => Ok(KernelCmd::LoadRom {
                group: 0,
                bytes: load.bytes.as_slice().into(),
            }),
            ArchivedKernelCmdV1::SetInputs(inputs) => Ok(KernelCmd::SetInputs {
                group: inputs.group.to_native(),
                lanes_mask: inputs.lanes_mask.to_native(),
                joypad: inputs.joypad,
            }),
            ArchivedKernelCmdV1::Terminate(term) => Ok(KernelCmd::Terminate {
                group: term.group.to_native(),
            }),
            ArchivedKernelCmdV1::Debug(debug) => Ok(KernelCmd::Debug(decode_debug_cmd(debug))),
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let tag = TAG_KERNEL_REP;
        let (class, schema) = match rep {
            KernelRep::TickDone { .. } => (
                PortClass::Lossless,
                KernelRepV1::TickDone {
                    purpose: TickPurposeV1::Display,
                    budget: 0,
                },
            ),
            KernelRep::LaneFrame {
                lane,
                frame_id,
                span,
                ..
            } => (
                PortClass::Lossless,
                KernelRepV1::LaneFrame {
                    lane: *lane,
                    frame_id: *frame_id,
                    span: encode_slot_span(span),
                    pixels: span.pixels.to_vec(),
                },
            ),
            KernelRep::RomLoaded { bytes_len, .. } => (
                PortClass::Lossless,
                KernelRepV1::RomLoaded {
                    bytes_len: *bytes_len as u32,
                },
            ),
            KernelRep::AudioReady { group, span, .. } => (
                PortClass::Lossless,
                KernelRepV1::AudioReady {
                    group: *group,
                    span: encode_audio_slot_span(span),
                },
            ),
            KernelRep::DroppedThumb { .. } => {
                return Err(FabricError::Unsupported(
                    "kernel codec does not yet support thumbnail reports",
                ))
            }
            KernelRep::Debug(rep) => {
                let class = match rep {
                    DebugRep::Snapshot(_) => PortClass::Coalesce,
                    DebugRep::MemWindow { .. } | DebugRep::Stepped { .. } => PortClass::Lossless,
                };
                (class, KernelRepV1::Debug(encode_debug_rep(rep)))
            }
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            class,
            Envelope::new(tag, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep> {
        ensure_tag(envelope, TAG_KERNEL_REP)?;
        let archived = archived_root::<KernelRepV1>(payload)?;
        let rep = match archived {
            ArchivedKernelRepV1::TickDone { .. } => KernelRep::TickDone {
                group: 0,
                lanes_mask: 0,
                cycles_done: 0,
            },
            ArchivedKernelRepV1::LaneFrame {
                lane,
                frame_id,
                span,
                pixels,
            } => KernelRep::LaneFrame {
                group: 0,
                lane: lane.to_native(),
                span: frame_span_from_parts(span, pixels),
                frame_id: frame_id.to_native(),
            },
            ArchivedKernelRepV1::RomLoaded { bytes_len } => KernelRep::RomLoaded {
                group: 0,
                bytes_len: bytes_len.to_native() as usize,
            },
            ArchivedKernelRepV1::AudioReady { group, span } => KernelRep::AudioReady {
                group: group.to_native(),
                span: audio_span_from_slot(span),
            },
            ArchivedKernelRepV1::Debug(rep) => KernelRep::Debug(decode_debug_rep(rep)),
        };
        Ok(rep)
    }
}

/// Codec for filesystem service commands and reports.
#[derive(Clone, Default)]
pub struct FsCodec;

impl Codec for FsCodec {
    type Cmd = FsCmd;
    type Rep = FsRep;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded> {
        let class = PortClass::Lossless;
        let schema = match cmd {
            FsCmd::Persist { path, bytes } => FsCmdV1::Persist(FsPersistCmdV1 {
                key: path.display().to_string(),
                payload: bytes.iter().copied().collect(),
            }),
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            class,
            Envelope::new(TAG_FS_CMD, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd> {
        ensure_tag(envelope, TAG_FS_CMD)?;
        let archived = archived_root::<FsCmdV1>(payload)?;
        match archived {
            ArchivedFsCmdV1::Persist(persist) => Ok(FsCmd::Persist {
                path: persist.key.as_str().into(),
                bytes: persist.payload.as_slice().into(),
            }),
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let schema = match rep {
            FsRep::Saved { path, ok } => FsRepV1::Saved {
                key: path.display().to_string(),
                ok: *ok,
            },
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(TAG_FS_REP, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep> {
        ensure_tag(envelope, TAG_FS_REP)?;
        let archived = archived_root::<FsRepV1>(payload)?;
        match archived {
            ArchivedFsRepV1::Saved { key, ok } => Ok(FsRep::Saved {
                path: key.as_str().into(),
                ok: *ok,
            }),
        }
    }
}

/// Codec for GPU service commands and reports.
#[derive(Clone, Default)]
pub struct GpuCodec;

impl Codec for GpuCodec {
    type Cmd = GpuCmd;
    type Rep = GpuRep;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded> {
        let schema = match cmd {
            GpuCmd::UploadFrame { lane, .. } => GpuCmdV1::UploadFrame {
                lane: *lane,
                frame_id: 0,
            },
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(TAG_GPU_CMD, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd> {
        ensure_tag(envelope, TAG_GPU_CMD)?;
        let archived = archived_root::<GpuCmdV1>(payload)?;
        match archived {
            ArchivedGpuCmdV1::UploadFrame { lane, .. } => Ok(GpuCmd::UploadFrame {
                lane: lane.to_native(),
                span: default_frame_span(),
            }),
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let schema = match rep {
            GpuRep::FrameShown { lane, frame_id } => GpuRepV1::FramePresented {
                lane: *lane,
                frame_id: *frame_id,
            },
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(TAG_GPU_REP, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep> {
        ensure_tag(envelope, TAG_GPU_REP)?;
        let archived = archived_root::<GpuRepV1>(payload)?;
        match archived {
            ArchivedGpuRepV1::FramePresented { lane, frame_id } => Ok(GpuRep::FrameShown {
                lane: lane.to_native(),
                frame_id: frame_id.to_native(),
            }),
        }
    }
}

/// Codec for audio service commands and reports.
#[derive(Clone, Default)]
pub struct AudioCodec;

impl Codec for AudioCodec {
    type Cmd = AudioCmd;
    type Rep = AudioRep;

    fn encode_cmd(&self, cmd: &Self::Cmd) -> FabricResult<Encoded> {
        let schema = match cmd {
            AudioCmd::Submit { span } => AudioCmdV1::SubmitSamples {
                frames: audio_frames(span) as u32,
            },
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(TAG_AUDIO_CMD, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_cmd(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Cmd> {
        ensure_tag(envelope, TAG_AUDIO_CMD)?;
        let archived = archived_root::<AudioCmdV1>(payload)?;
        match archived {
            ArchivedAudioCmdV1::SubmitSamples { frames } => Ok(AudioCmd::Submit {
                span: default_audio_span(frames.to_native() as usize),
            }),
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let schema = match rep {
            AudioRep::Played { frames } => AudioRepV1::Played {
                frames: *frames as u32,
            },
            AudioRep::Underrun => AudioRepV1::Underrun,
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            PortClass::Lossless,
            Envelope::new(TAG_AUDIO_REP, SCHEMA_VERSION_V1),
            payload,
        ))
    }

    fn decode_rep(&self, envelope: Envelope, payload: &[u8]) -> FabricResult<Self::Rep> {
        ensure_tag(envelope, TAG_AUDIO_REP)?;
        let archived = archived_root::<AudioRepV1>(payload)?;
        match archived {
            ArchivedAudioRepV1::Played { frames } => Ok(AudioRep::Played {
                frames: frames.to_native() as usize,
            }),
            ArchivedAudioRepV1::Underrun => Ok(AudioRep::Underrun),
        }
    }
}

fn encode_debug_cmd(cmd: &DebugCmd) -> KernelDebugCmdV1 {
    match cmd {
        DebugCmd::Snapshot { group } => KernelDebugCmdV1::Snapshot { group: *group },
        DebugCmd::MemWindow {
            group,
            space,
            base,
            len,
        } => KernelDebugCmdV1::MemWindow {
            group: *group,
            space: encode_mem_space(*space),
            base: *base,
            len: *len,
        },
        DebugCmd::StepInstruction { group, count } => KernelDebugCmdV1::StepInstruction {
            group: *group,
            count: *count,
        },
        DebugCmd::StepFrame { group } => KernelDebugCmdV1::StepFrame { group: *group },
    }
}

fn decode_debug_cmd(cmd: &ArchivedKernelDebugCmdV1) -> DebugCmd {
    match cmd {
        ArchivedKernelDebugCmdV1::Snapshot { group } => DebugCmd::Snapshot {
            group: group.to_native(),
        },
        ArchivedKernelDebugCmdV1::MemWindow {
            group,
            space,
            base,
            len,
        } => DebugCmd::MemWindow {
            group: group.to_native(),
            space: decode_mem_space(space),
            base: base.to_native(),
            len: len.to_native(),
        },
        ArchivedKernelDebugCmdV1::StepInstruction { group, count } => DebugCmd::StepInstruction {
            group: group.to_native(),
            count: count.to_native(),
        },
        ArchivedKernelDebugCmdV1::StepFrame { group } => DebugCmd::StepFrame {
            group: group.to_native(),
        },
    }
}

fn encode_debug_rep(rep: &DebugRep) -> KernelDebugRepV1 {
    match rep {
        DebugRep::Snapshot(snapshot) => KernelDebugRepV1::Snapshot(encode_snapshot(snapshot)),
        DebugRep::MemWindow { space, base, bytes } => KernelDebugRepV1::MemWindow {
            space: encode_mem_space(*space),
            base: *base,
            bytes: bytes.as_ref().to_vec(),
        },
        DebugRep::Stepped {
            kind,
            cycles,
            pc,
            disasm,
        } => KernelDebugRepV1::Stepped {
            kind: encode_step_kind(*kind),
            cycles: *cycles,
            pc: *pc,
            disasm: disasm.clone(),
        },
    }
}

fn decode_debug_rep(rep: &ArchivedKernelDebugRepV1) -> DebugRep {
    match rep {
        ArchivedKernelDebugRepV1::Snapshot(snapshot) => {
            DebugRep::Snapshot(decode_snapshot(snapshot))
        }
        ArchivedKernelDebugRepV1::MemWindow { space, base, bytes } => DebugRep::MemWindow {
            space: decode_mem_space(space),
            base: base.to_native(),
            bytes: Arc::<[u8]>::from(bytes.as_slice()),
        },
        ArchivedKernelDebugRepV1::Stepped {
            kind,
            cycles,
            pc,
            disasm,
        } => DebugRep::Stepped {
            kind: decode_step_kind(kind),
            cycles: cycles.to_native(),
            pc: pc.to_native(),
            disasm: disasm.as_ref().map(|arch| arch.as_str().to_string()),
        },
    }
}

fn encode_snapshot(snapshot: &InspectorVMMinimal) -> KernelDebugSnapshotV1 {
    KernelDebugSnapshotV1 {
        cpu: encode_cpu(&snapshot.cpu),
        ppu: encode_ppu(&snapshot.ppu),
        timers: encode_timers(&snapshot.timers),
        io: snapshot.io.clone(),
    }
}

fn decode_snapshot(snapshot: &ArchivedKernelDebugSnapshotV1) -> InspectorVMMinimal {
    InspectorVMMinimal {
        cpu: decode_cpu(&snapshot.cpu),
        ppu: decode_ppu(&snapshot.ppu),
        timers: decode_timers(&snapshot.timers),
        io: snapshot.io.as_slice().to_vec(),
    }
}

fn encode_cpu(cpu: &CpuVM) -> KernelDebugCpuVmV1 {
    KernelDebugCpuVmV1 {
        a: cpu.a,
        f: cpu.f,
        b: cpu.b,
        c: cpu.c,
        d: cpu.d,
        e: cpu.e,
        h: cpu.h,
        l: cpu.l,
        sp: cpu.sp,
        pc: cpu.pc,
        ime: cpu.ime,
        halted: cpu.halted,
    }
}

fn decode_cpu(cpu: &ArchivedKernelDebugCpuVmV1) -> CpuVM {
    CpuVM {
        a: cpu.a,
        f: cpu.f,
        b: cpu.b,
        c: cpu.c,
        d: cpu.d,
        e: cpu.e,
        h: cpu.h,
        l: cpu.l,
        sp: cpu.sp.to_native(),
        pc: cpu.pc.to_native(),
        ime: cpu.ime,
        halted: cpu.halted,
    }
}

fn encode_ppu(ppu: &PpuVM) -> KernelDebugPpuVmV1 {
    KernelDebugPpuVmV1 {
        ly: ppu.ly,
        mode: ppu.mode,
        stat: ppu.stat,
        lcdc: ppu.lcdc,
        scx: ppu.scx,
        scy: ppu.scy,
        wy: ppu.wy,
        wx: ppu.wx,
        bgp: ppu.bgp,
        frame_ready: ppu.frame_ready,
    }
}

fn decode_ppu(ppu: &ArchivedKernelDebugPpuVmV1) -> PpuVM {
    PpuVM {
        ly: ppu.ly,
        mode: ppu.mode,
        stat: ppu.stat,
        lcdc: ppu.lcdc,
        scx: ppu.scx,
        scy: ppu.scy,
        wy: ppu.wy,
        wx: ppu.wx,
        bgp: ppu.bgp,
        frame_ready: ppu.frame_ready,
    }
}

fn encode_timers(timers: &TimersVM) -> KernelDebugTimersVmV1 {
    KernelDebugTimersVmV1 {
        div: timers.div,
        tima: timers.tima,
        tma: timers.tma,
        tac: timers.tac,
    }
}

fn decode_timers(timers: &ArchivedKernelDebugTimersVmV1) -> TimersVM {
    TimersVM {
        div: timers.div,
        tima: timers.tima,
        tma: timers.tma,
        tac: timers.tac,
    }
}

fn encode_mem_space(space: MemSpace) -> KernelDebugMemSpaceV1 {
    match space {
        MemSpace::Vram => KernelDebugMemSpaceV1::Vram,
        MemSpace::Wram => KernelDebugMemSpaceV1::Wram,
        MemSpace::Oam => KernelDebugMemSpaceV1::Oam,
        MemSpace::Io => KernelDebugMemSpaceV1::Io,
    }
}

fn decode_mem_space(space: &ArchivedKernelDebugMemSpaceV1) -> MemSpace {
    match space {
        ArchivedKernelDebugMemSpaceV1::Vram => MemSpace::Vram,
        ArchivedKernelDebugMemSpaceV1::Wram => MemSpace::Wram,
        ArchivedKernelDebugMemSpaceV1::Oam => MemSpace::Oam,
        ArchivedKernelDebugMemSpaceV1::Io => MemSpace::Io,
    }
}

fn encode_step_kind(kind: StepKind) -> KernelDebugStepKindV1 {
    match kind {
        StepKind::Instruction => KernelDebugStepKindV1::Instruction,
        StepKind::Frame => KernelDebugStepKindV1::Frame,
    }
}

fn decode_step_kind(kind: &ArchivedKernelDebugStepKindV1) -> StepKind {
    match kind {
        ArchivedKernelDebugStepKindV1::Instruction => StepKind::Instruction,
        ArchivedKernelDebugStepKindV1::Frame => StepKind::Frame,
    }
}

fn serialize<T>(value: &T) -> FabricResult<Vec<u8>>
where
    T: Archive,
    T: for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, Error>>,
{
    to_bytes::<Error>(value)
        .map(|aligned| aligned.into_vec())
        .map_err(|err| FabricError::codec(format!("serialize failure: {err}")))
}

fn default_kernel_policy(cmd: &KernelCmd) -> PortClass {
    match cmd {
        KernelCmd::Tick { purpose, .. } => match purpose {
            TickPurpose::Display => PortClass::Coalesce,
            TickPurpose::Exploration => PortClass::BestEffort,
        },
        KernelCmd::LoadRom { .. } => PortClass::Lossless,
        KernelCmd::SetInputs { .. } => PortClass::Lossless,
        KernelCmd::Terminate { .. } => PortClass::Lossless,
        KernelCmd::Debug(cmd) => class_from_policy(cmd.submit_policy()),
    }
}

fn class_from_policy(policy: SubmitPolicy) -> PortClass {
    match policy {
        SubmitPolicy::Must | SubmitPolicy::Lossless => PortClass::Lossless,
        SubmitPolicy::Coalesce => PortClass::Coalesce,
        SubmitPolicy::BestEffort => PortClass::BestEffort,
    }
}

fn ensure_tag(envelope: Envelope, expected: u8) -> FabricResult<()> {
    if envelope.tag != expected {
        return Err(FabricError::codec(format!(
            "unexpected envelope tag {} (expected {})",
            envelope.tag, expected
        )));
    }
    if envelope.ver != SCHEMA_VERSION_V1 {
        return Err(FabricError::codec(format!(
            "schema version mismatch: {} vs {}",
            envelope.ver, SCHEMA_VERSION_V1
        )));
    }
    Ok(())
}

fn archived_root<T>(payload: &[u8]) -> FabricResult<&rkyv::Archived<T>>
where
    T: Archive,
    T::Archived: for<'a> CheckBytes<HighValidator<'a, Error>>,
{
    access::<T::Archived, Error>(payload)
        .map_err(|err| FabricError::codec(format!("validation failure: {err}")))
}

/// Returns a default (empty) frame span placeholder.
pub fn default_frame_span() -> service_abi::FrameSpan {
    service_abi::FrameSpan::default()
}

fn encode_slot_span(span: &service_abi::FrameSpan) -> SlotSpanV1 {
    match span.slot_span.as_ref() {
        Some(slot) => SlotSpanV1 {
            start_idx: slot.start_idx,
            count: slot.count,
        },
        None => SlotSpanV1 {
            start_idx: 0,
            count: 0,
        },
    }
}

fn frame_span_from_parts(
    span: &ArchivedSlotSpanV1,
    pixels: &rkyv::Archived<Vec<u8>>,
) -> service_abi::FrameSpan {
    let start_idx = span.start_idx.to_native();
    let count = span.count.to_native();
    let slot_span = if count == 0 {
        None
    } else {
        Some(SlotSpan { start_idx, count })
    };
    let mut frame_span = default_frame_span();
    if frame_span.width == 0 {
        frame_span.width = 160;
    }
    if frame_span.height == 0 {
        frame_span.height = 144;
    }
    frame_span.slot_span = slot_span;
    if !pixels.is_empty() {
        let owned = pixels.as_slice().to_vec().into_boxed_slice();
        frame_span.pixels = Arc::from(owned);
    }
    frame_span
}

fn encode_audio_slot_span(span: &AudioSpan) -> SlotSpanV1 {
    match span.slot_span.as_ref() {
        Some(slot) => SlotSpanV1 {
            start_idx: slot.start_idx,
            count: slot.count,
        },
        None => SlotSpanV1 {
            start_idx: 0,
            count: 0,
        },
    }
}

fn audio_span_from_slot(span: &ArchivedSlotSpanV1) -> AudioSpan {
    let start_idx = span.start_idx.to_native();
    let count = span.count.to_native();
    let slot_span = if count == 0 {
        None
    } else {
        Some(SlotSpan { start_idx, count })
    };
    let mut audio_span = default_audio_span(0);
    audio_span.slot_span = slot_span;
    audio_span
}

fn default_audio_span(_frames: usize) -> AudioSpan {
    AudioSpan::empty()
}

fn audio_frames(span: &AudioSpan) -> usize {
    let channels = span.channels.max(1) as usize;
    span.samples.len() / channels
}
