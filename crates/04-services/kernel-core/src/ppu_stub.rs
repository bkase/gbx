/// Total CPU cycles per frame on the DMG.
pub const CYCLES_PER_FRAME: u32 = 70_224;

/// Minimal PPU stub that only tracks frame boundaries.
#[derive(Default, Clone)]
pub struct PpuStub {
    pub(crate) cycles: u32,
    pub(crate) frame_ready: bool,
}

impl PpuStub {
    /// Creates a new PPU stub instance.
    pub fn new() -> Self {
        Self {
            cycles: 0,
            frame_ready: false,
        }
    }

    /// Advances the PPU by the provided number of CPU cycles.
    pub fn step(&mut self, cycles: u32) {
        self.cycles = self.cycles.wrapping_add(cycles);
        if self.cycles >= CYCLES_PER_FRAME {
            self.cycles -= CYCLES_PER_FRAME;
            self.frame_ready = true;
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
        self.cycles = 0;
        self.frame_ready = false;
    }
}
