//! SIMD execution backend built on top of `std::simd`.

use crate::exec::{Exec, Flags, MaskValue};
use core::simd::cmp::SimdPartialEq;
use core::simd::{LaneCount, Mask, Simd, SupportedLaneCount};
use std::simd::num::SimdUint;

/// Mask wrapper used by the SIMD backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneMask<const LANES: usize>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    inner: Mask<i8, LANES>,
}

impl<const LANES: usize> LaneMask<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    /// Creates a mask from the provided `std::simd` mask.
    #[inline]
    pub fn new(inner: Mask<i8, LANES>) -> Self {
        Self { inner }
    }

    /// Returns the underlying mask value.
    #[inline]
    pub fn into_inner(self) -> Mask<i8, LANES> {
        self.inner
    }

    /// Returns the boolean value of the provided lane.
    #[inline]
    pub fn lane(&self, lane: usize) -> bool {
        self.inner.test(lane)
    }
}

impl<const LANES: usize> MaskValue for LaneMask<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    #[inline]
    fn false_mask() -> Self {
        Self {
            inner: Mask::splat(false),
        }
    }

    #[inline]
    fn true_mask() -> Self {
        Self {
            inner: Mask::splat(true),
        }
    }

    #[inline]
    fn to_bool(self) -> bool {
        self.inner.test(0)
    }

    #[inline]
    fn from_bool(value: bool) -> Self {
        Self {
            inner: Mask::splat(value),
        }
    }
}

/// SIMD execution backend using `LANES` parallel Game Boy instances.
/// Zero-sized backend marker mirroring the scalar backend.
#[derive(Clone, Copy, Debug)]
pub enum SimdExec<const LANES: usize>
where
    LaneCount<LANES>: SupportedLaneCount, {}

impl<const LANES: usize> Exec for SimdExec<LANES>
where
    LaneCount<LANES>: SupportedLaneCount,
{
    type U8 = Simd<u8, LANES>;
    type U16 = Simd<u16, LANES>;
    type Mask = LaneMask<LANES>;

    #[inline]
    fn from_u8(value: u8) -> Self::U8 {
        Simd::splat(value)
    }

    #[inline]
    fn from_u16(value: u16) -> Self::U16 {
        Simd::splat(value)
    }

    #[inline]
    fn to_u8(value: Self::U8) -> u8 {
        value.to_array()[0]
    }

    #[inline]
    fn to_u16(value: Self::U16) -> u16 {
        value.to_array()[0]
    }

    #[inline]
    fn combine_u16(high: Self::U8, low: Self::U8) -> Self::U16 {
        (high.cast::<u16>() << Simd::splat(8)) | low.cast::<u16>()
    }

    #[inline]
    fn split_u16(value: Self::U16) -> (Self::U8, Self::U8) {
        let hi = (value >> Simd::splat(8)).cast::<u8>();
        let lo = (value & Simd::splat(0x00FF)).cast::<u8>();
        (hi, lo)
    }

    #[inline]
    fn and(a: Self::U8, b: Self::U8) -> Self::U8 {
        a & b
    }

    #[inline]
    fn or(a: Self::U8, b: Self::U8) -> Self::U8 {
        a | b
    }

    #[inline]
    fn xor(a: Self::U8, b: Self::U8) -> Self::U8 {
        a ^ b
    }

    #[inline]
    fn add8(a: Self::U8, b: Self::U8, carry: bool, f: &mut Flags<Self::Mask>) -> Self::U8 {
        let carry_in = Simd::splat(if carry { 1 } else { 0 });
        let sum = a + b + carry_in;

        let zero = sum.simd_eq(Simd::splat(0)).all();
        let half_mask = ((a ^ b ^ sum) & Simd::splat(0x10)).simd_ne(Simd::splat(0));
        let carry_mask = (((a & b) | ((a | b) & !sum)) & Simd::splat(0x80)).simd_ne(Simd::splat(0));

        f.set_z(zero);
        f.set_n(false);
        f.set_h(half_mask.test(0));
        f.set_c(carry_mask.test(0));

        sum
    }

    #[inline]
    fn sub8(a: Self::U8, b: Self::U8, borrow: bool, f: &mut Flags<Self::Mask>) -> Self::U8 {
        let borrow_in = Simd::splat(if borrow { 1 } else { 0 });
        let diff = a - b - borrow_in;

        let zero = diff.simd_eq(Simd::splat(0)).all();
        let half_mask = ((a ^ b ^ diff) & Simd::splat(0x10)).simd_ne(Simd::splat(0));
        let borrow_mask =
            (((!a & b) | ((!(a ^ b)) & diff)) & Simd::splat(0x80)).simd_ne(Simd::splat(0));

        f.set_z(zero);
        f.set_n(true);
        f.set_h(half_mask.test(0));
        f.set_c(borrow_mask.test(0));

        diff
    }

    #[inline]
    fn eq8(a: Self::U8, b: Self::U8) -> Self::Mask {
        LaneMask::new(a.simd_eq(b))
    }

    #[inline]
    fn select8(mask: Self::Mask, if_true: Self::U8, if_false: Self::U8) -> Self::U8 {
        mask.into_inner().select(if_true, if_false)
    }
}
