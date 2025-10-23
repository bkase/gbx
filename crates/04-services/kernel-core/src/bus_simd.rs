//! SIMD-aware bus that multiplexes the scalar implementation across lanes.

use crate::bus::{Bus, BusScalar, InterruptCtrl, SerialIo};
use crate::exec::Exec;
use crate::exec_simd::SimdExec;
use crate::mmu;
use crate::ppu_stub::{PpuFrameSource, PpuIo};
use crate::timers::TimerIo;
use core::simd::{LaneCount, Simd, SupportedLaneCount};
use std::array;
use std::sync::Arc;

/// Bus implementation that keeps one scalar bus per SIMD lane.
pub struct BusSimd<const LANES: usize>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    lanes: [BusScalar; LANES],
}

impl<const LANES: usize> BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    /// Creates a SIMD bus by cloning the provided ROM into each lane.
    pub fn new(rom: Arc<[u8]>) -> Self {
        assert!(LANES > 0, "SIMD bus requires at least one lane");
        Self {
            lanes: array::from_fn(|_| BusScalar::new(Arc::clone(&rom))),
        }
    }

    /// Returns the number of active lanes.
    #[inline]
    pub fn lane_count(&self) -> usize {
        LANES
    }

    /// Replaces the ROM contents for every lane.
    pub fn load_rom(&mut self, rom: Arc<[u8]>) {
        for lane in &mut self.lanes {
            lane.load_rom(Arc::clone(&rom));
        }
    }

    /// Provides immutable access to a specific lane for diagnostics and tests.
    pub fn lane(&self, lane: usize) -> &BusScalar {
        &self.lanes[lane]
    }

    /// Provides mutable access to a specific lane for diagnostics and tests.
    pub fn lane_mut(&mut self, lane: usize) -> &mut BusScalar {
        &mut self.lanes[lane]
    }

    /// Applies joypad inputs across lanes, keeping them in lockstep.
    pub fn set_inputs(&mut self, joypad: u8) {
        for lane in &mut self.lanes {
            lane.set_inputs(joypad);
        }
    }

    fn canonical_lane(&self) -> &BusScalar {
        &self.lanes[0]
    }
}

impl<const LANES: usize> Bus<SimdExec<LANES>> for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn read8(&mut self, addr: <SimdExec<LANES> as Exec>::U16) -> <SimdExec<LANES> as Exec>::U8 {
        let addresses = addr.to_array();
        let mut bytes = [0u8; LANES];
        for lane in 0..LANES {
            bytes[lane] = mmu::read8_scalar(&mut self.lanes[lane], addresses[lane]);
        }
        Simd::from_array(bytes)
    }

    #[inline]
    fn write8(
        &mut self,
        addr: <SimdExec<LANES> as Exec>::U16,
        value: <SimdExec<LANES> as Exec>::U8,
    ) {
        let addresses = addr.to_array();
        let values = value.to_array();
        for lane in 0..LANES {
            mmu::write8_scalar(&mut self.lanes[lane], addresses[lane], values[lane]);
        }
    }
}

impl<const LANES: usize> InterruptCtrl for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn read_ie(&self) -> u8 {
        let canonical = self.canonical_lane().ie;
        debug_assert!(
            self.lanes.iter().all(|lane| lane.ie == canonical),
            "interrupt enable diverged between lanes"
        );
        canonical
    }
}

impl<const LANES: usize> SerialIo for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn step_serial(&mut self, cycles: u32) {
        for lane in &mut self.lanes {
            lane.step_serial(cycles);
        }
    }
}

impl<const LANES: usize> TimerIo for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn read_div(&self) -> u8 {
        let canonical = self.canonical_lane().io.div();
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.div() == canonical),
            "DIV register diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn write_div(&mut self, value: u8) {
        for lane in &mut self.lanes {
            lane.io.set_div(value);
        }
    }

    #[inline]
    fn take_div_reset(&mut self) -> bool {
        let mut any = false;
        for lane in &mut self.lanes {
            any |= lane.take_div_reset();
        }
        any
    }

    #[inline]
    fn take_tima_write(&mut self) -> Option<u8> {
        let mut value = None;
        for lane in &mut self.lanes {
            let lane_value = lane.take_tima_write();
            debug_assert!(
                value.is_none() || lane_value == value,
                "TIMA write diverged between lanes"
            );
            if value.is_none() && lane_value.is_some() {
                value = lane_value;
            }
        }
        value
    }

    #[inline]
    fn take_tma_write(&mut self) -> Option<u8> {
        let mut value = None;
        for lane in &mut self.lanes {
            let lane_value = lane.take_tma_write();
            debug_assert!(
                value.is_none() || lane_value == value,
                "TMA write diverged between lanes"
            );
            if value.is_none() && lane_value.is_some() {
                value = lane_value;
            }
        }
        value
    }

    #[inline]
    fn take_tac_write(&mut self) -> Option<(u8, u8)> {
        let mut value = None;
        for lane in &mut self.lanes {
            let lane_value = lane.take_tac_write();
            debug_assert!(
                value.is_none() || lane_value == value,
                "TAC write diverged between lanes"
            );
            if value.is_none() && lane_value.is_some() {
                value = lane_value;
            }
        }
        value
    }

    #[inline]
    fn read_tima(&self) -> u8 {
        let canonical = self.canonical_lane().io.tima();
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.tima() == canonical),
            "TIMA diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn write_tima(&mut self, value: u8) {
        for lane in &mut self.lanes {
            lane.io.set_tima(value);
        }
    }

    #[inline]
    fn read_tma(&self) -> u8 {
        let canonical = self.canonical_lane().io.tma();
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.tma() == canonical),
            "TMA diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn read_tac(&self) -> u8 {
        let canonical = self.canonical_lane().io.tac();
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.tac() == canonical),
            "TAC diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn read_if(&self) -> u8 {
        let canonical = self.canonical_lane().io.if_reg();
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.if_reg() == canonical),
            "IF register diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn write_if(&mut self, value: u8) {
        for lane in &mut self.lanes {
            lane.io.set_if(value);
        }
    }
}

impl<const LANES: usize> PpuIo for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn read_io(&self, idx: usize) -> u8 {
        let canonical = self.canonical_lane().io.read(idx);
        debug_assert!(
            self.lanes.iter().all(|lane| lane.io.read(idx) == canonical),
            "PPU IO diverged between lanes"
        );
        canonical
    }

    #[inline]
    fn write_io(&mut self, idx: usize, value: u8) {
        for lane in &mut self.lanes {
            lane.io.write(idx, value);
        }
    }

    #[inline]
    fn read_if(&self) -> u8 {
        <Self as TimerIo>::read_if(self)
    }

    #[inline]
    fn write_if(&mut self, value: u8) {
        <Self as TimerIo>::write_if(self, value);
    }
}

impl<const LANES: usize> PpuFrameSource for BusSimd<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn ppu_io(&self) -> &crate::bus::IoRegs {
        self.canonical_lane().ppu_io()
    }

    #[inline]
    fn ppu_vram(&self) -> &[u8; 0x2000] {
        self.canonical_lane().ppu_vram()
    }
}
