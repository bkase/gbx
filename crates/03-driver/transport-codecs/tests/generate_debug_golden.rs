#![cfg(test)]
//! Helper to regenerate debug codec golden binaries.

use std::sync::Arc;

use service_abi::{
    CpuVM, DebugCmd, DebugRep, InspectorVMMinimal, MemSpace, PpuVM, StepKind, TimersVM,
};
use transport_codecs::KernelCodec;
use transport_fabric::Codec;

#[test]
#[ignore]
fn dump_debug_goldens() {
    let codec = KernelCodec;

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

    let mem = DebugRep::MemWindow {
        space: MemSpace::Wram,
        base: 0xC100,
        bytes: Arc::from([0xDE, 0xAD, 0xBE, 0xEF].as_slice()),
    };

    let step = DebugRep::Stepped {
        kind: StepKind::Instruction,
        cycles: 8,
        pc: 0x0200,
        disasm: Some("LD A, (HL)".to_string()),
    };

    let rep_payloads = [snapshot, mem, step];

    for rep in rep_payloads {
        let encoded = codec
            .encode_rep(&service_abi::KernelRep::Debug(rep))
            .unwrap();
        print_bytes("rep", &encoded.payload);
    }

    let cmds = [
        DebugCmd::Snapshot { group: 2 },
        DebugCmd::MemWindow {
            group: 3,
            space: MemSpace::Oam,
            base: 0xFE00,
            len: 0x10,
        },
        DebugCmd::StepInstruction { group: 4, count: 3 },
        DebugCmd::StepFrame { group: 5 },
    ];

    for cmd in cmds {
        let encoded = codec
            .encode_cmd(&service_abi::KernelCmd::Debug(cmd))
            .unwrap();
        print_bytes("cmd", &encoded.payload);
    }
}

fn print_bytes(label: &str, bytes: &[u8]) {
    print!("{label}:");
    for byte in bytes {
        print!(" {byte:02X}");
    }
    println!();
}
