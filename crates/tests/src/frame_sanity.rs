#![cfg(all(test, not(target_arch = "wasm32")))]
//! Frame pixel sanity tests.

use gbx_frame::{write_checkerboard_rgba, FRAME_HEADER};

#[test]
fn checkerboard_pixels_look_ok() {
    const W: u16 = 160;
    const H: u16 = 144;
    const FRAME_ID: u32 = 42;

    let need = FRAME_HEADER + (W as usize) * (H as usize) * 4;
    let mut slot = vec![0u8; need];

    // Write checkerboard into the slot
    let ok = write_checkerboard_rgba(&mut slot, FRAME_ID, W, H);
    assert!(ok, "write should succeed with sufficient space");

    // Check header
    let stored_frame_id = u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]);
    assert_eq!(stored_frame_id, FRAME_ID, "frame_id in header");

    let w = u16::from_le_bytes([slot[4], slot[5]]) as usize;
    let h = u16::from_le_bytes([slot[6], slot[7]]) as usize;
    assert_eq!(w, W as usize, "width in header");
    assert_eq!(h, H as usize, "height in header");

    // Check pixel data
    let px = &slot[FRAME_HEADER..FRAME_HEADER + w * h * 4];
    assert_eq!(px[3], 0xFF, "alpha channel");

    // Sample two tiles that should have opposite patterns
    // Tile (0,0): (0>>3 ^ 0>>3) & 1 = (0 ^ 0) & 1 = 0 -> v = 0x20 (dark)
    let idx_0_0 = 0;
    assert_eq!(px[idx_0_0], 0x20);
    assert_eq!(px[idx_0_0 + 1], 0x20);
    assert_eq!(px[idx_0_0 + 2], 0x20);
    assert_eq!(px[idx_0_0 + 3], 0xFF);

    // Tile (8,0): (8>>3 ^ 0>>3) & 1 = (1 ^ 0) & 1 = 1 -> v = 0xE0 (light)
    let idx_8_0 = 8 * 4;
    assert_eq!(px[idx_8_0], 0xE0);
    assert_eq!(px[idx_8_0 + 1], 0xE0);
    assert_eq!(px[idx_8_0 + 2], 0xE0);
    assert_eq!(px[idx_8_0 + 3], 0xFF);

    // Verify tiles in the same row differ (checkerboard pattern)
    assert_ne!(px[idx_0_0], px[idx_8_0], "adjacent tiles should differ");
}

#[test]
fn checkerboard_fails_with_insufficient_space() {
    const W: u16 = 160;
    const H: u16 = 144;
    let need = FRAME_HEADER + (W as usize) * (H as usize) * 4;
    let mut slot = vec![0u8; need - 1]; // One byte too small

    let ok = write_checkerboard_rgba(&mut slot, 0, W, H);
    assert!(!ok, "write should fail with insufficient space");
}
