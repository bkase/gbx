use crate::bus::{BusScalar, IoRegs};
use crate::exec::{Exec, Scalar};

/// Reads a byte from the scalar bus.
pub fn read8_scalar(bus: &mut BusScalar, addr: <Scalar as Exec>::U16) -> <Scalar as Exec>::U8 {
    let addr = <Scalar as Exec>::to_u16(addr);
    match addr {
        0x0000..=0x3FFF => {
            if let Some(byte) = bus.boot_rom_byte(addr) {
                byte
            } else {
                bus.rom.get(addr as usize).copied().unwrap_or(0xFF)
            }
        }
        0x4000..=0x7FFF => {
            let bank = bus.rom_bank;
            let offset = (addr - 0x4000) as usize;
            let index = bank.saturating_mul(0x4000).saturating_add(offset);
            bus.rom.get(index).copied().unwrap_or(0xFF)
        }
        0x8000..=0x9FFF => bus.vram[(addr - 0x8000) as usize],
        0xA000..=0xBFFF => 0xFF, // No MBC for M1 scope.
        0xC000..=0xDFFF => bus.wram[(addr - 0xC000) as usize],
        0xE000..=0xFDFF => bus.wram[(addr - 0xE000) as usize],
        0xFE00..=0xFE9F => bus.oam[(addr - 0xFE00) as usize],
        0xFEA0..=0xFEFF => 0xFF,
        0xFF00..=0xFF7F => {
            let idx = (addr - 0xFF00) as usize;
            match idx {
                IoRegs::JOYP => {
                    let sel = bus.joyp_select;
                    let mut lo = 0x0F;
                    if sel & 0x10 == 0 {
                        lo &= bus.joyp_dpad;
                    }
                    if sel & 0x20 == 0 {
                        lo &= bus.joyp_buttons;
                    }
                    0xC0 | sel | (!lo & 0x0F)
                }
                IoRegs::IF => bus.io.if_reg(),
                IoRegs::LY => bus.lockstep_ly_override.unwrap_or_else(|| bus.io.read(idx)),
                0x50 => {
                    if bus.boot_rom_enabled() {
                        0x00
                    } else {
                        0x01
                    }
                }
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
        0x0000..=0x1FFF => {
            // Cartridge RAM enable (unsupported)
        }
        0x2000..=0x3FFF => {
            let value = <Scalar as Exec>::to_u8(value);
            let bank = (value & 0x1F) as usize;
            bus.set_rom_bank(bank);
        }
        0x4000..=0x5FFF => {
            // High ROM bank bits / RAM bank (ignored for current cartridges)
        }
        0x6000..=0x7FFF => {
            // Banking mode select (ignored)
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
            if idx == 0x46 {
                let base = u16::from(value) << 8;
                for offset in 0..=0x9F {
                    let src = Scalar::from_u16(base.wrapping_add(offset as u16));
                    let byte = read8_scalar(bus, src);
                    bus.oam[offset as usize] = Scalar::to_u8(byte);
                }
                return;
            }
            if idx == 0x50 {
                if value & 0x01 != 0 {
                    bus.disable_boot_rom();
                }
                return;
            }
            if idx == IoRegs::JOYP {
                bus.joyp_select = value & 0x30;
                let current = bus.io.read(idx);
                bus.io.write(idx, (current & !0x30) | bus.joyp_select);
                return;
            }
            if idx == BusScalar::io_div_index() {
                bus.io.set_div(0);
                bus.timer_div_reset = true;
            } else if idx == IoRegs::TIMA {
                bus.io.set_tima(value);
                bus.timer_tima_write = Some(value);
            } else if idx == IoRegs::TMA {
                bus.io.set_tma(value);
                bus.timer_tma_write = Some(value);
            } else if idx == IoRegs::TAC {
                let masked = value & 0x07;
                let previous = bus.io.tac();
                bus.io.set_tac(masked);
                bus.timer_tac_write = Some((previous, masked));
            } else if idx == BusScalar::io_ly_index() {
                bus.io.write(idx, 0);
            } else {
                if idx == BusScalar::io_sc_index() {
                    bus.write_serial_control(value);
                    return;
                }
                if idx == IoRegs::IF {
                    if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                        eprintln!("IF write {:02X}", value);
                    }
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
            if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                eprintln!("IE write {:02X}", value);
            }
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
