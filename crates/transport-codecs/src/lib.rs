//! Codec implementations for gbx service messages.
//!
//! This crate provides concrete codec implementations that serialize and deserialize
//! domain-specific command and report types (from `world`) to and from the wire format
//! used by the transport fabric.

#![allow(missing_docs)]

use hub::SubmitPolicy;
use rkyv::{
    api::high::{access, to_bytes, HighSerializer, HighValidator},
    bytecheck::CheckBytes,
    rancor::Error,
    ser::allocator::ArenaHandle,
    util::AlignedVec,
    Archive, Serialize,
};
use transport::schema::*;
use transport::Envelope;
use transport_fabric::{Codec, Encoded, FabricError, FabricResult};
use world::{
    AudioCmd, AudioRep, AudioSpan, FsCmd, FsRep, GpuCmd, GpuRep, KernelCmd, KernelRep, TickPurpose,
};

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
            KernelCmd::SetInputs { .. } | KernelCmd::Terminate { .. } => {
                return Err(FabricError::Unsupported(
                    "kernel codec does not yet support SetInputs/Terminate",
                ))
            }
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
        }
    }

    fn encode_rep(&self, rep: &Self::Rep) -> FabricResult<Encoded> {
        let tag = TAG_KERNEL_REP;
        let schema = match rep {
            KernelRep::TickDone { .. } => KernelRepV1::TickDone {
                purpose: TickPurposeV1::Display,
                budget: 0,
            },
            KernelRep::LaneFrame { lane, frame_id, .. } => KernelRepV1::LaneFrame {
                lane: *lane,
                frame_id: *frame_id,
            },
            KernelRep::RomLoaded { bytes_len, .. } => KernelRepV1::RomLoaded {
                bytes_len: *bytes_len as u32,
            },
            KernelRep::AudioReady { .. } | KernelRep::DroppedThumb { .. } => {
                return Err(FabricError::Unsupported(
                    "kernel codec does not yet support audio/thumb reports",
                ))
            }
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            SubmitPolicy::Lossless,
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
            ArchivedKernelRepV1::LaneFrame { lane, frame_id } => KernelRep::LaneFrame {
                group: 0,
                lane: lane.to_native(),
                span: default_frame_span(),
                frame_id: frame_id.to_native(),
            },
            ArchivedKernelRepV1::RomLoaded { bytes_len } => KernelRep::RomLoaded {
                group: 0,
                bytes_len: bytes_len.to_native() as usize,
            },
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
        let policy = SubmitPolicy::Lossless;
        let schema = match cmd {
            FsCmd::Persist { path, bytes } => FsCmdV1::Persist(FsPersistCmdV1 {
                key: path.display().to_string(),
                payload: bytes.iter().copied().collect(),
            }),
        };
        let payload = serialize(&schema)?;
        Ok(Encoded::new(
            policy,
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
            SubmitPolicy::Lossless,
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
            SubmitPolicy::Must,
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
            SubmitPolicy::Lossless,
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
            SubmitPolicy::Must,
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
            SubmitPolicy::Lossless,
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

fn serialize<T>(value: &T) -> FabricResult<Vec<u8>>
where
    T: Archive,
    T: for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, Error>>,
{
    to_bytes::<Error>(value)
        .map(|aligned| aligned.into_vec())
        .map_err(|err| FabricError::codec(format!("serialize failure: {err}")))
}

fn default_kernel_policy(cmd: &KernelCmd) -> SubmitPolicy {
    match cmd {
        KernelCmd::Tick { purpose, .. } => match purpose {
            TickPurpose::Display => SubmitPolicy::Coalesce,
            TickPurpose::Exploration => SubmitPolicy::BestEffort,
        },
        KernelCmd::LoadRom { .. } => SubmitPolicy::Lossless,
        KernelCmd::SetInputs { .. } => SubmitPolicy::Lossless,
        KernelCmd::Terminate { .. } => SubmitPolicy::Lossless,
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
pub fn default_frame_span() -> world::FrameSpan {
    world::FrameSpan::default()
}

fn default_audio_span(_frames: usize) -> AudioSpan {
    AudioSpan::empty()
}

fn audio_frames(span: &AudioSpan) -> usize {
    let channels = span.channels.max(1) as usize;
    span.samples.len() / channels
}
