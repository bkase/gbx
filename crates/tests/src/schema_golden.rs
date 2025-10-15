//! Golden tests that freeze the rkyv ABI for transport-visible messages.

use bytecheck::CheckBytes;
use rkyv::{
    ser::serializers::AllocSerializer, validation::validators::DefaultValidator, AlignedVec,
    Archive, Serialize,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use transport::schema::*;

const SCRATCH: usize = 256;

#[test]
fn transport_schema_goldens_v1() {
    assert_golden(
        "kernel_cmd_tick_v1",
        &KernelCmdV1::Tick(KernelTickCmdV1 {
            purpose: TickPurposeV1::Display,
            budget: 32_768,
        }),
    );

    assert_golden(
        "kernel_cmd_load_rom_v1",
        &KernelCmdV1::LoadRom(KernelLoadRomCmdV1 {
            bytes: vec![0x00, 0x01, 0x02, 0x03, 0xFE, 0xFF],
        }),
    );

    assert_golden(
        "kernel_rep_tick_done_v1",
        &KernelRepV1::TickDone {
            purpose: TickPurposeV1::Exploration,
            budget: 40_000,
        },
    );

    assert_golden(
        "kernel_rep_lane_frame_v1",
        &KernelRepV1::LaneFrame {
            lane: 3,
            frame_id: 0x0123_4567_89AB_CDEF,
        },
    );

    assert_golden(
        "kernel_rep_rom_loaded_v1",
        &KernelRepV1::RomLoaded { bytes_len: 65_536 },
    );

    assert_golden(
        "fs_cmd_persist_v1",
        &FsCmdV1::Persist(FsPersistCmdV1 {
            key: "autosave".to_string(),
            payload: vec![0xAA, 0xBB, 0xCC],
        }),
    );

    assert_golden(
        "fs_rep_saved_true_v1",
        &FsRepV1::Saved {
            key: "autosave".to_string(),
            ok: true,
        },
    );

    assert_golden(
        "fs_rep_saved_false_v1",
        &FsRepV1::Saved {
            key: "autosave".to_string(),
            ok: false,
        },
    );

    assert_golden(
        "gpu_cmd_upload_frame_v1",
        &GpuCmdV1::UploadFrame {
            lane: 7,
            frame_id: 0xDEAD_BEEF_F00D,
        },
    );

    assert_golden(
        "gpu_rep_frame_presented_v1",
        &GpuRepV1::FramePresented {
            lane: 7,
            frame_id: 0xDEAD_BEEF_F00D,
        },
    );

    assert_golden(
        "audio_cmd_submit_samples_v1",
        &AudioCmdV1::SubmitSamples { frames: 960 },
    );

    assert_golden("audio_rep_played_v1", &AudioRepV1::Played { frames: 960 });

    assert_golden("audio_rep_underrun_v1", &AudioRepV1::Underrun);
}

fn assert_golden<T>(stem: &str, value: &T)
where
    T: Archive,
    T: Serialize<AllocSerializer<SCRATCH>>,
    for<'a> T::Archived: CheckBytes<DefaultValidator<'a>>,
{
    let bytes = serialize(value);
    rkyv::check_archived_root::<T>(&bytes).expect("byte validation failed");
    verify_or_update(stem, bytes.as_ref());
}

fn serialize<T>(value: &T) -> AlignedVec
where
    T: Archive,
    T: Serialize<AllocSerializer<SCRATCH>>,
{
    rkyv::to_bytes::<_, SCRATCH>(value).expect("serialize value")
}

fn verify_or_update(stem: &str, bytes: &[u8]) {
    let path = golden_path(stem);
    if update_mode() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden dir");
        }
        fs::write(&path, bytes).expect("write golden file");
    } else {
        let expected = fs::read(&path)
            .unwrap_or_else(|_| panic!("missing golden fixture: {}", path.display()));
        if expected.as_slice() != bytes {
            panic!("golden drift detected for {stem}; rerun with UPDATE_GOLDEN=1 to regenerate");
        }
    }
}

fn golden_path(stem: &str) -> PathBuf {
    manifest_dir().join("golden").join(format!("{stem}.bin"))
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn update_mode() -> bool {
    matches!(env::var("UPDATE_GOLDEN"), Ok(ref v) if v == "1")
}
