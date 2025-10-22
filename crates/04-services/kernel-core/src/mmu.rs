use crate::bus::{BusScalar, IoRegs};
use crate::exec::{Exec, Scalar};

/// Reads a byte from the scalar bus.
pub fn read8_scalar(bus: &mut BusScalar, addr: <Scalar as Exec>::U16) -> <Scalar as Exec>::U8 {
    let addr = <Scalar as Exec>::to_u16(addr);
    match addr {
        0x0000..=0x7FFF => bus.rom.get(addr as usize).copied().unwrap_or(0xFF),
        0x8000..=0x9FFF => bus.vram[(addr - 0x8000) as usize],
        0xA000..=0xBFFF => 0xFF, // No MBC for M1 scope.
        0xC000..=0xDFFF => bus.wram[(addr - 0xC000) as usize],
        0xE000..=0xFDFF => bus.wram[(addr - 0xE000) as usize],
        0xFE00..=0xFE9F => bus.oam[(addr - 0xFE00) as usize],
        0xFEA0..=0xFEFF => 0xFF,
        0xFF00..=0xFF7F => {
            let idx = (addr - 0xFF00) as usize;
            match idx {
                IoRegs::IF => bus.io.if_reg(),
                IoRegs::LY => bus.lockstep_ly_override.unwrap_or_else(|| bus.io.read(idx)),
                _ => bus.io.read(idx),
            }
        }
        0xFF80..=0xFFFE => bus.hram[(addr - 0xFF80) as usize],
        0xFFFF => bus.ie,
    }
}

/// Writes a byte to the scalar bus.
pub fn write8_scalar(
    bus: &mut BusScalar,
    addr: <Scalar as Exec>::U16,
    value: <Scalar as Exec>::U8,
) {
    let addr = <Scalar as Exec>::to_u16(addr);
    match addr {
        0x0000..=0x7FFF => {
            // ROM is immutable; ignore writes for the no-MBC configuration.
        }
        0x8000..=0x9FFF => {
            bus.vram[(addr - 0x8000) as usize] = value;
        }
        0xA000..=0xBFFF => {
            // No cartridge RAM in scope.
        }
        0xC000..=0xDFFF => {
            bus.wram[(addr - 0xC000) as usize] = value;
        }
        0xE000..=0xFDFF => {
            bus.wram[(addr - 0xE000) as usize] = value;
        }
        0xFE00..=0xFE9F => {
            bus.oam[(addr - 0xFE00) as usize] = value;
        }
        0xFEA0..=0xFEFF => {}
        0xFF00..=0xFF7F => {
            let idx = (addr - 0xFF00) as usize;
            if idx == BusScalar::io_div_index() {
                bus.io.set_div(0);
            } else if idx == BusScalar::io_ly_index() {
                bus.io.write(idx, 0);
            } else {
                if idx == BusScalar::io_sc_index() {
                    bus.write_serial_control(value);
                    return;
                }
                if idx == IoRegs::IF {
                    bus.io.set_if(value);
                } else {
                    bus.io.write(idx, value);
                }
            }
        }
        0xFF80..=0xFFFE => {
            bus.hram[(addr - 0xFF80) as usize] = value;
        }
        0xFFFF => {
            bus.ie = value;
        }
    }
}

impl BusScalar {
    pub(crate) fn io_div_index() -> usize {
        crate::bus::IoRegs::DIV
    }

    pub(crate) fn io_ly_index() -> usize {
        crate::bus::IoRegs::LY
    }
}
