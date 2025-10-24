use crate::bus::{BusScalar, IoRegs};

/// Total CPU cycles per frame on the DMG.
pub const CYCLES_PER_FRAME: u32 = 70_224;

const DOTS_PER_LINE: u32 = 456;
const MODE2_CYCLES: u32 = 80;
const MODE3_CYCLES: u32 = 172;
const VBLANK_START_LINE: u8 = 144;
const TOTAL_LINES: u8 = 154;
const DMG_SHADES: [u8; 4] = [0xFF, 0xAA, 0x55, 0x00];

const LCDC_ENABLE: u8 = 0x80;
const STAT_MODE_MASK: u8 = 0x03;
const STAT_COINCIDENCE_BIT: u8 = 0x04;
const STAT_MODE0_INT: u8 = 0x08;
const STAT_MODE1_INT: u8 = 0x10;
const STAT_MODE2_INT: u8 = 0x20;
const STAT_LYC_INT: u8 = 0x40;
const IF_VBLANK: u8 = 0x01;
const IF_LCD_STAT: u8 = 0x02;

/// Minimal IO surface required by the PPU stub.
pub trait PpuIo {
    /// Reads an IO register by index.
    fn read_io(&self, idx: usize) -> u8;
    /// Writes an IO register by index.
    fn write_io(&mut self, idx: usize, value: u8);
    /// Reads the interrupt flag register.
    fn read_if(&self) -> u8;
    /// Writes the interrupt flag register.
    fn write_if(&mut self, value: u8);
}

impl PpuIo for BusScalar {
    #[inline]
    fn read_io(&self, idx: usize) -> u8 {
        self.io.read(idx)
    }

    #[inline]
    fn write_io(&mut self, idx: usize, value: u8) {
        self.io.write(idx, value);
    }

    #[inline]
    fn read_if(&self) -> u8 {
        self.io.if_reg()
    }

    #[inline]
    fn write_if(&mut self, value: u8) {
        self.io.set_if(value);
    }
}

/// Read-only view over the state required for background rendering.
pub trait PpuFrameSource {
    /// Returns a shared reference to the IO register block.
    fn ppu_io(&self) -> &IoRegs;
    /// Returns a shared reference to VRAM.
    fn ppu_vram(&self) -> &[u8; 0x2000];
    /// Returns a shared reference to OAM.
    fn ppu_oam(&self) -> &[u8; 0xA0];
}

impl PpuFrameSource for BusScalar {
    #[inline]
    fn ppu_io(&self) -> &IoRegs {
        &self.io
    }

    #[inline]
    fn ppu_vram(&self) -> &[u8; 0x2000] {
        &self.vram
    }

    #[inline]
    fn ppu_oam(&self) -> &[u8; 0xA0] {
        &self.oam
    }
}

/// Minimal PPU that models scanline timings, IO state, and interrupts.
#[derive(Clone, Default)]
pub struct PpuStub {
    pub(crate) dot_in_line: u32,
    pub(crate) ly: u8,
    pub(crate) mode: u8,
    pub(crate) lyc_equal: bool,
    pub(crate) frame_ready: bool,
    pub(crate) lcd_was_on: bool,
}

impl PpuStub {
    /// Creates a new PPU stub instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the PPU by the provided number of CPU cycles.
    pub fn step<I: PpuIo>(&mut self, mut cycles: u32, bus: &mut I) {
        if cycles == 0 {
            return;
        }

        if !self.ensure_lcd_on(bus) {
            return;
        }

        while cycles > 0 {
            if bus.read_io(IoRegs::LCDC) & LCDC_ENABLE == 0 {
                self.force_lcd_off(bus);
                return;
            }

            self.refresh_coincidence(bus);

            let remaining = self.cycles_until_next_event();
            if remaining == 0 {
                self.handle_event(bus);
                continue;
            }

            let step = remaining.min(cycles);
            self.dot_in_line = self.dot_in_line.wrapping_add(step);
            cycles -= step;

            if step == remaining {
                self.handle_event(bus);
            }
        }
    }

    /// Returns whether a new frame is available.
    pub fn frame_ready(&self) -> bool {
        self.frame_ready
    }

    /// Clears the ready flag after the frame has been consumed.
    pub fn clear_frame_ready(&mut self) {
        self.frame_ready = false;
    }

    /// Resets the PPU stub counters.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Renders a complete frame using background, window, and sprites.
    pub fn render_frame(
        &self,
        io: &IoRegs,
        vram: &[u8; 0x2000],
        oam: &[u8; 0xA0],
        out_rgba: &mut [u8],
        width: u16,
        height: u16,
    ) {
        let lcdc = io.read(IoRegs::LCDC);
        let lcd_on = lcdc & LCDC_ENABLE != 0;

        fill_solid(out_rgba, DMG_SHADES[0]);

        if !lcd_on {
            return;
        }

        let width_px = usize::from(width);
        let height_px = usize::from(height);

        let mut bg_indices = vec![0u8; width_px * height_px];

        // Background plane
        if (lcdc & 0x01) != 0 {
            let scy = io.read(IoRegs::SCY);
            let scx = io.read(IoRegs::SCX);
            let tile_data_unsigned = lcdc & 0x10 != 0;
            let map_base: u16 = if lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 };
            let palette = decode_bgp(io.read(IoRegs::BGP));

            for y in 0..height_px {
                let wy = scy.wrapping_add(y as u8);
                let tile_row = ((wy as usize) >> 3) & 31;
                let row_in_tile = (wy & 0x07) as usize;

                for x in 0..width_px {
                    let wx = scx.wrapping_add(x as u8);
                    let tile_col = ((wx as usize) >> 3) & 31;
                    let col_in_tile = (wx & 0x07) as usize;

                    let map_index = tile_row * 32 + tile_col;
                    let map_addr = map_base.wrapping_add(map_index as u16);
                    let tile_id = read_vram(vram, map_addr);

                    let tile_base = if tile_data_unsigned {
                        0x8000u16.wrapping_add(u16::from(tile_id) * 16)
                    } else {
                        let signed = tile_id as i8 as i16;
                        (0x9000i32 + i32::from(signed) * 16) as u16
                    };
                    let row_addr = tile_base.wrapping_add((row_in_tile as u16) * 2);
                    let lo = read_vram(vram, row_addr);
                    let hi = read_vram(vram, row_addr.wrapping_add(1));

                    let bit = 7 - col_in_tile as u8;
                    let color_id = ((hi >> bit) & 0x01) << 1 | ((lo >> bit) & 0x01);
                    bg_indices[y * width_px + x] = color_id;
                    let shade = palette[color_id as usize];

                    let dst_idx = (y * width_px + x) * 4;
                    out_rgba[dst_idx] = shade;
                    out_rgba[dst_idx + 1] = shade;
                    out_rgba[dst_idx + 2] = shade;
                    out_rgba[dst_idx + 3] = 0xFF;
                }
            }
        }

        // Window plane
        if (lcdc & 0x20) != 0 {
            let wy = io.read(IoRegs::WY);
            let wx = io.read(IoRegs::WX);
            let tile_data_unsigned = lcdc & 0x10 != 0;
            let map_base: u16 = if lcdc & 0x40 != 0 { 0x9C00 } else { 0x9800 };
            let palette = decode_bgp(io.read(IoRegs::BGP));

            for y in 0..height_px {
                if (y as u8) < wy {
                    continue;
                }

                let win_y = (y as u8).wrapping_sub(wy) as usize;
                let tile_row = (win_y >> 3) & 31;
                let row_in_tile = (win_y & 0x07) as usize;

                for x in 0..width_px {
                    let wx_adjusted = (wx as i16) - 7;
                    if (x as i16) < wx_adjusted {
                        continue;
                    }

                    let win_x = ((x as i16) - wx_adjusted) as usize;
                    let tile_col = (win_x >> 3) & 31;
                    let col_in_tile = (win_x & 0x07) as usize;

                    let map_index = tile_row * 32 + tile_col;
                    let map_addr = map_base.wrapping_add(map_index as u16);
                    let tile_id = read_vram(vram, map_addr);

                    let tile_base = if tile_data_unsigned {
                        0x8000u16.wrapping_add(u16::from(tile_id) * 16)
                    } else {
                        let signed = tile_id as i8 as i16;
                        (0x9000i32 + i32::from(signed) * 16) as u16
                    };
                    let row_addr = tile_base.wrapping_add((row_in_tile as u16) * 2);
                    let lo = read_vram(vram, row_addr);
                    let hi = read_vram(vram, row_addr.wrapping_add(1));

                    let bit = 7 - col_in_tile as u8;
                    let color_id = ((hi >> bit) & 0x01) << 1 | ((lo >> bit) & 0x01);
                    bg_indices[y * width_px + x] = color_id;
                    let shade = palette[color_id as usize];

                    let dst_idx = (y * width_px + x) * 4;
                    out_rgba[dst_idx] = shade;
                    out_rgba[dst_idx + 1] = shade;
                    out_rgba[dst_idx + 2] = shade;
                    out_rgba[dst_idx + 3] = 0xFF;
                }
            }
        }

        // Sprites
        if (lcdc & 0x02) != 0 {
            let sprite_height = if (lcdc & 0x04) != 0 { 16 } else { 8 };
            let obp0 = decode_bgp(io.read(IoRegs::OBP0));
            let obp1 = decode_bgp(io.read(IoRegs::OBP1));

            for sprite in (0..40).rev() {
                let base = sprite * 4;
                let y_pos = oam[base].wrapping_sub(16);
                let x_pos = oam[base + 1].wrapping_sub(8);
                let tile_id = oam[base + 2];
                let attrs = oam[base + 3];

                let palette = if (attrs & 0x10) != 0 { obp1 } else { obp0 };
                let flip_x = (attrs & 0x20) != 0;
                let flip_y = (attrs & 0x40) != 0;
                let bg_priority = (attrs & 0x80) != 0;

                for row in 0..sprite_height {
                    let screen_y = y_pos.wrapping_add(row as u8);
                    if screen_y as usize >= height_px {
                        continue;
                    }

                    let effective_row = if flip_y { sprite_height - 1 - row } else { row };

                    let tile_base = if sprite_height == 16 {
                        let base_id = tile_id & 0xFE;
                        let row_offset = ((effective_row % 8) * 2) as u16;
                        let slab_offset = ((effective_row / 8) * 32) as u16;
                        0x8000u16 + u16::from(base_id) * 16 + row_offset + slab_offset
                    } else {
                        let row_offset = (effective_row * 2) as u16;
                        0x8000u16 + u16::from(tile_id) * 16 + row_offset
                    };

                    let lo = read_vram(vram, tile_base);
                    let hi = read_vram(vram, tile_base.wrapping_add(1));

                    for col in 0..8 {
                        let screen_x = x_pos.wrapping_add(col as u8);
                        if screen_x as usize >= width_px {
                            continue;
                        }

                        let bit_index = if flip_x { col } else { 7 - col };
                        let bit = bit_index as u8;
                        let color_id = ((hi >> bit) & 0x01) << 1 | ((lo >> bit) & 0x01);
                        if color_id == 0 {
                            continue;
                        }

                        let idx = (screen_y as usize) * width_px + screen_x as usize;
                        if bg_priority && bg_indices[idx] != 0 {
                            continue;
                        }

                        let shade = palette[color_id as usize];
                        let dst_idx = idx * 4;
                        out_rgba[dst_idx] = shade;
                        out_rgba[dst_idx + 1] = shade;
                        out_rgba[dst_idx + 2] = shade;
                        out_rgba[dst_idx + 3] = 0xFF;
                    }
                }
            }
        }
    }

    fn ensure_lcd_on<I: PpuIo>(&mut self, bus: &mut I) -> bool {
        let lcdc = bus.read_io(IoRegs::LCDC);
        if lcdc & LCDC_ENABLE == 0 {
            self.force_lcd_off(bus);
            return false;
        }

        if !self.lcd_was_on {
            self.start_lcd(bus);
        }

        true
    }

    fn start_lcd<I: PpuIo>(&mut self, bus: &mut I) {
        self.lcd_was_on = true;
        self.dot_in_line = 0;
        self.ly = 0;
        self.mode = 0;
        self.lyc_equal = false;
        self.frame_ready = false;

        bus.write_io(IoRegs::LY, self.ly);
        self.set_mode(bus, 2);
    }

    fn force_lcd_off<I: PpuIo>(&mut self, bus: &mut I) {
        self.lcd_was_on = false;
        self.dot_in_line = 0;
        self.mode = 0;
        self.frame_ready = false;
        self.ly = 0;

        bus.write_io(IoRegs::LY, 0);

        let mut stat = bus.read_io(IoRegs::STAT);
        let lyc = bus.read_io(IoRegs::LYC);
        self.lyc_equal = lyc == 0;
        stat &= !0x07;
        if self.lyc_equal {
            stat |= STAT_COINCIDENCE_BIT;
        }
        bus.write_io(IoRegs::STAT, stat);
    }

    fn refresh_coincidence<I: PpuIo>(&mut self, bus: &mut I) {
        let stat_before = bus.read_io(IoRegs::STAT);
        let lyc = bus.read_io(IoRegs::LYC);
        let equal = self.ly == lyc;
        let prev_equal = self.lyc_equal;
        self.lyc_equal = equal;

        let mut stat_after = stat_before & !0x07;
        stat_after |= self.mode & STAT_MODE_MASK;
        if equal {
            stat_after |= STAT_COINCIDENCE_BIT;
        }
        bus.write_io(IoRegs::STAT, stat_after);

        if !prev_equal && equal && stat_before & STAT_LYC_INT != 0 {
            self.raise_stat_interrupt(bus);
        }
    }

    fn set_mode<I: PpuIo>(&mut self, bus: &mut I, new_mode: u8) {
        if self.mode == new_mode {
            return;
        }

        let stat_before = bus.read_io(IoRegs::STAT);
        let mut stat_after = stat_before & !0x07;
        stat_after |= new_mode & STAT_MODE_MASK;
        if self.lyc_equal {
            stat_after |= STAT_COINCIDENCE_BIT;
        }

        bus.write_io(IoRegs::STAT, stat_after);
        self.mode = new_mode;

        let enabled = match new_mode {
            0 => stat_before & STAT_MODE0_INT != 0,
            1 => stat_before & STAT_MODE1_INT != 0,
            2 => stat_before & STAT_MODE2_INT != 0,
            _ => false,
        };

        if enabled {
            self.raise_stat_interrupt(bus);
        }
    }

    fn cycles_until_next_event(&self) -> u32 {
        match self.mode {
            0 => DOTS_PER_LINE - self.dot_in_line,
            1 => DOTS_PER_LINE - self.dot_in_line,
            2 => MODE2_CYCLES - self.dot_in_line,
            3 => MODE2_CYCLES + MODE3_CYCLES - self.dot_in_line,
            _ => 0,
        }
    }

    fn handle_event<I: PpuIo>(&mut self, bus: &mut I) {
        match self.mode {
            2 => {
                self.set_mode(bus, 3);
            }
            3 => {
                self.set_mode(bus, 0);
            }
            0 | 1 => {
                if self.dot_in_line >= DOTS_PER_LINE {
                    self.dot_in_line -= DOTS_PER_LINE;
                } else {
                    self.dot_in_line = 0;
                }
                self.advance_line(bus);
            }
            _ => {}
        }
    }

    fn advance_line<I: PpuIo>(&mut self, bus: &mut I) {
        self.ly = self.ly.wrapping_add(1);
        if self.ly >= TOTAL_LINES {
            self.ly = 0;
        }

        bus.write_io(IoRegs::LY, self.ly);
        self.refresh_coincidence(bus);

        if self.ly == 0 {
            self.frame_ready = true;
            self.set_mode(bus, 2);
        } else if self.ly < VBLANK_START_LINE {
            self.set_mode(bus, 2);
        } else if self.ly == VBLANK_START_LINE {
            self.raise_vblank_interrupt(bus);
            self.set_mode(bus, 1);
        } else {
            // Lines 145..153 remain in mode 1.
        }
    }

    fn raise_vblank_interrupt<I: PpuIo>(&mut self, bus: &mut I) {
        let mut if_reg = bus.read_if();
        if_reg |= IF_VBLANK;
        bus.write_if(if_reg);
    }

    fn raise_stat_interrupt<I: PpuIo>(&mut self, bus: &mut I) {
        let mut if_reg = bus.read_if();
        if_reg |= IF_LCD_STAT;
        bus.write_if(if_reg);
    }
}

fn decode_bgp(bgp: u8) -> [u8; 4] {
    let mut shades = [0u8; 4];
    for cid in 0..4 {
        let shade_idx = (bgp >> (cid * 2)) & 0x03;
        shades[cid as usize] = DMG_SHADES[shade_idx as usize];
    }
    shades
}

fn fill_solid(out: &mut [u8], shade: u8) {
    for px in out.chunks_exact_mut(4) {
        px[0] = shade;
        px[1] = shade;
        px[2] = shade;
        px[3] = 0xFF;
    }
}

fn read_vram(vram: &[u8; 0x2000], addr: u16) -> u8 {
    debug_assert!((0x8000..0xA000).contains(&addr));
    let idx = usize::from(addr - 0x8000);
    vram[idx]
}
