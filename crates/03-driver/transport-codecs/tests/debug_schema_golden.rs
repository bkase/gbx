//! Golden byte comparison for debug command/report schemas.

use std::sync::Arc;

use rkyv::util::AlignedVec;
use service_abi::{
    CpuVM, DebugCmd, DebugRep, InspectorVMMinimal, KernelCmd, KernelRep, MemSpace, PpuVM, StepKind,
    TimersVM,
};
use transport_codecs::KernelCodec;
use transport_fabric::Codec;

fn codec() -> KernelCodec {
    KernelCodec
}

#[test]
fn debug_rep_snapshot_matches_golden() {
    let rep = KernelRep::Debug(DebugRep::Snapshot(sample_snapshot()));
    let expected = include_bytes!("golden/debug_rep_0.bin");
    assert_encoded_matches(&rep, expected);
}

#[test]
fn debug_rep_mem_matches_golden() {
    let rep = KernelRep::Debug(DebugRep::MemWindow {
        space: MemSpace::Wram,
        base: 0xC100,
        bytes: Arc::from([0xDE, 0xAD, 0xBE, 0xEF].as_slice()),
    });
    let expected = include_bytes!("golden/debug_rep_1.bin");
    assert_encoded_matches(&rep, expected);
}

#[test]
fn debug_rep_step_matches_golden() {
    let rep = KernelRep::Debug(DebugRep::Stepped {
        kind: StepKind::Instruction,
        cycles: 8,
        pc: 0x0200,
        disasm: Some("LD A, (HL)".into()),
    });
    let expected = include_bytes!("golden/debug_rep_2.bin");
    assert_encoded_matches(&rep, expected);
}

#[test]
fn debug_cmd_snapshot_matches_golden() {
    let cmd = KernelCmd::Debug(DebugCmd::Snapshot { group: 2 });
    let expected = include_bytes!("golden/debug_cmd_0.bin");
    assert_encoded_matches_cmd(&cmd, expected);
}

#[test]
fn debug_cmd_mem_matches_golden() {
    let cmd = KernelCmd::Debug(DebugCmd::MemWindow {
        group: 3,
        space: MemSpace::Oam,
        base: 0xFE00,
        len: 0x10,
    });
    let expected = include_bytes!("golden/debug_cmd_1.bin");
    assert_encoded_matches_cmd(&cmd, expected);
}

#[test]
fn debug_cmd_step_instruction_matches_golden() {
    let cmd = KernelCmd::Debug(DebugCmd::StepInstruction { group: 4, count: 3 });
    let expected = include_bytes!("golden/debug_cmd_2.bin");
    assert_encoded_matches_cmd(&cmd, expected);
}

#[test]
fn debug_cmd_step_frame_matches_golden() {
    let cmd = KernelCmd::Debug(DebugCmd::StepFrame { group: 5 });
    let expected = include_bytes!("golden/debug_cmd_3.bin");
    assert_encoded_matches_cmd(&cmd, expected);
}

fn assert_encoded_matches(rep: &KernelRep, expected: &[u8]) {
    let codec = codec();
    let encoded = codec.encode_rep(rep).expect("encode");
    assert_eq!(expected, encoded.payload.as_slice(), "payload mismatch");

    let aligned = aligned_from_slice(expected);
    let decoded = codec
        .decode_rep(encoded.envelope, &aligned)
        .expect("decode");
    assert_eq!(rep, &decoded);
}

fn assert_encoded_matches_cmd(cmd: &KernelCmd, expected: &[u8]) {
    let codec = codec();
    let encoded = codec.encode_cmd(cmd).expect("encode");
    assert_eq!(expected, encoded.payload.as_slice(), "payload mismatch");

    let aligned = aligned_from_slice(expected);
    let decoded = codec
        .decode_cmd(encoded.envelope, &aligned)
        .expect("decode");
    assert_eq!(cmd, &decoded);
}

fn aligned_from_slice(bytes: &[u8]) -> AlignedVec {
    let mut aligned = AlignedVec::with_capacity(bytes.len());
    aligned.extend_from_slice(bytes);
    aligned
}

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
