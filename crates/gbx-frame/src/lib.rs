//! Frame encoding and decoding utilities for RGBA frame data.
//!
//! This crate provides application-level frame encoding/decoding utilities
//! with no dependency on the transport layer. It operates purely on byte slices.

/// Size of the frame header (4 bytes frame_id + 2 bytes width + 2 bytes height).
pub const FRAME_HEADER: usize = 8;

/// Writes a checkerboard RGBA frame into a slot with the given frame ID and dimensions.
///
/// # Layout
/// - Bytes 0..4: u32 frame_id (little-endian)
/// - Bytes 4..6: u16 width (little-endian)
/// - Bytes 6..8: u16 height (little-endian)
/// - Bytes 8..: width * height * 4 bytes of RGBA8888 data
///
/// Returns `true` if the slot was large enough and the frame was written successfully.
#[inline]
pub fn write_checkerboard_rgba(slot: &mut [u8], frame_id: u32, w: u16, h: u16) -> bool {
    let need = FRAME_HEADER + (w as usize) * (h as usize) * 4;
    if slot.len() < need {
        return false;
    }

    // Write header
    slot[0..4].copy_from_slice(&frame_id.to_le_bytes());
    slot[4..6].copy_from_slice(&w.to_le_bytes());
    slot[6..8].copy_from_slice(&h.to_le_bytes());

    // Write checkerboard pattern
    let mut o = FRAME_HEADER;
    for y in 0..(h as usize) {
        for x in 0..(w as usize) {
            let tile = ((x >> 3) ^ (y >> 3)) & 1;
            let v = if tile == 0 { 0x20 } else { 0xE0 };
            slot[o] = v;
            slot[o + 1] = v;
            slot[o + 2] = v;
            slot[o + 3] = 0xFF;
            o += 4;
        }
    }
    true
}

/// Decodes the frame header from a slot, returning (frame_id, width, height).
///
/// Returns `None` if the slot is too small to contain a valid header.
#[inline]
pub fn decode_header(slot: &[u8]) -> Option<(u32, u16, u16)> {
    if slot.len() < FRAME_HEADER {
        return None;
    }
    let id = u32::from_le_bytes(slot[0..4].try_into().ok()?);
    let w = u16::from_le_bytes(slot[4..6].try_into().ok()?);
    let h = u16::from_le_bytes(slot[6..8].try_into().ok()?);
    Some((id, w, h))
}
