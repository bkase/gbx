//! Round-trip coverage for debug codec encode/decode paths.

use std::sync::Arc;

use service_abi::{
    CpuVM, DebugCmd, DebugRep, InspectorVMMinimal, KernelCmd, KernelRep, MemSpace, PpuVM, StepKind,
    TimersVM,
};
use transport_codecs::KernelCodec;
use transport_fabric::{Codec, PortClass};

fn roundtrip_cmd(cmd: KernelCmd) {
    let codec = KernelCodec;
    let encoded = codec.encode_cmd(&cmd).expect("encode");
    assert_eq!(port_class_for(&cmd), encoded.class, "port class mismatch");
    let decoded = codec
        .decode_cmd(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert_eq!(cmd, decoded);
}

fn roundtrip_rep(rep: KernelRep) {
    let codec = KernelCodec;
    let encoded = codec.encode_rep(&rep).expect("encode");
    let decoded = codec
        .decode_rep(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert_eq!(rep, decoded);
}

fn port_class_for(cmd: &KernelCmd) -> PortClass {
    match cmd {
        KernelCmd::Tick { .. } => PortClass::Coalesce,
        KernelCmd::LoadRom { .. } => PortClass::Lossless,
        KernelCmd::SetInputs { .. } => PortClass::Lossless,
        KernelCmd::Terminate { .. } => PortClass::Lossless,
        KernelCmd::Debug(DebugCmd::Snapshot { .. }) => PortClass::Coalesce,
        KernelCmd::Debug(_) => PortClass::Lossless,
    }
}

#[test]
fn debug_cmds_roundtrip() {
    roundtrip_cmd(KernelCmd::Debug(DebugCmd::Snapshot { group: 7 }));
    roundtrip_cmd(KernelCmd::Debug(DebugCmd::MemWindow {
        group: 3,
        space: MemSpace::Vram,
        base: 0x8123,
        len: 0x20,
    }));
    roundtrip_cmd(KernelCmd::Debug(DebugCmd::StepInstruction {
        group: 1,
        count: 17,
    }));
    roundtrip_cmd(KernelCmd::Debug(DebugCmd::StepFrame { group: 9 }));
}

#[test]
fn debug_reps_roundtrip() {
    let snapshot = DebugRep::Snapshot(InspectorVMMinimal {
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
    });

    let mem_window = DebugRep::MemWindow {
        space: MemSpace::Oam,
        base: 0xFE00,
        bytes: Arc::from([0x01, 0x02, 0x03, 0x04].as_slice()),
    };

    let stepped = DebugRep::Stepped {
        kind: StepKind::Instruction,
        cycles: 12,
        pc: 0x1234,
        disasm: Some("NOP".to_string()),
    };

    roundtrip_rep(KernelRep::Debug(snapshot));
    roundtrip_rep(KernelRep::Debug(mem_window));
    roundtrip_rep(KernelRep::Debug(stepped));
}

#[test]
fn legacy_non_debug_roundtrip() {
    let cmd = KernelCmd::Tick {
        group: 0,
        purpose: service_abi::TickPurpose::Display,
        budget: 10,
    };
    let codec = KernelCodec;
    let encoded = codec.encode_cmd(&cmd).expect("encode");
    let decoded = codec
        .decode_cmd(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert!(matches!(decoded, KernelCmd::Tick { .. }));

    roundtrip_cmd(KernelCmd::Terminate { group: 4 });

    let rep = KernelRep::TickDone {
        group: 0,
        lanes_mask: 1,
        cycles_done: 101,
    };
    let encoded = codec.encode_rep(&rep).expect("encode");
    let decoded = codec
        .decode_rep(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert!(matches!(decoded, KernelRep::TickDone { .. }));
}

#[test]
fn envelope_metadata_is_preserved() {
    let cmd = KernelCmd::Debug(DebugCmd::StepFrame { group: 2 });
    let codec = KernelCodec;
    let encoded = codec.encode_cmd(&cmd).expect("encode");
    assert_eq!(transport::schema::TAG_KERNEL_CMD, encoded.envelope.tag);
    assert_eq!(transport::schema::SCHEMA_VERSION_V1, encoded.envelope.ver);

    let decoded = codec
        .decode_cmd(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert_eq!(cmd, decoded);
}

#[test]
fn debug_rep_contains_trace_defaults() {
    let rep = KernelRep::Debug(DebugRep::Stepped {
        kind: StepKind::Frame,
        cycles: 70_224,
        pc: 0x2000,
        disasm: None,
    });
    let codec = KernelCodec;
    let encoded = codec.encode_rep(&rep).expect("encode");
    let decoded = codec
        .decode_rep(encoded.envelope, &encoded.payload)
        .expect("decode");
    assert_eq!(rep, decoded);
}
