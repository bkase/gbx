use core::fmt;

/// Trait describing a mask value used for flag lanes.
pub trait MaskValue: Copy + PartialEq {
    /// Returns the canonical false value.
    fn false_mask() -> Self;
    /// Returns the canonical true value.
    fn true_mask() -> Self;
    /// Converts the mask into a boolean, primarily used by scalar helpers.
    fn to_bool(self) -> bool;
    /// Produces a mask from a boolean.
    fn from_bool(value: bool) -> Self;
}

impl MaskValue for bool {
    #[inline]
    fn false_mask() -> Self {
        false
    }

    #[inline]
    fn true_mask() -> Self {
        true
    }

    #[inline]
    fn to_bool(self) -> bool {
        self
    }

    #[inline]
    fn from_bool(value: bool) -> Self {
        value
    }
}

/// Flag register representation parametrised by a mask type.
#[derive(Clone, Copy)]
pub struct Flags<M: MaskValue> {
    z: M,
    n: M,
    h: M,
    c: M,
}

impl<M: MaskValue> Default for Flags<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: MaskValue> Flags<M> {
    /// Creates flags with all bits cleared.
    #[inline]
    pub fn new() -> Self {
        Self {
            z: M::false_mask(),
            n: M::false_mask(),
            h: M::false_mask(),
            c: M::false_mask(),
        }
    }

    /// Sets the zero flag.
    #[inline]
    pub fn set_z(&mut self, value: bool) {
        self.z = M::from_bool(value);
    }

    /// Sets the subtract flag.
    #[inline]
    pub fn set_n(&mut self, value: bool) {
        self.n = M::from_bool(value);
    }

    /// Sets the half-carry flag.
    #[inline]
    pub fn set_h(&mut self, value: bool) {
        self.h = M::from_bool(value);
    }

    /// Sets the carry flag.
    #[inline]
    pub fn set_c(&mut self, value: bool) {
        self.c = M::from_bool(value);
    }

    /// Returns the zero flag as a boolean.
    #[inline]
    pub fn z(&self) -> bool {
        self.z.to_bool()
    }

    /// Returns the subtract flag.
    #[inline]
    pub fn n(&self) -> bool {
        self.n.to_bool()
    }

    /// Returns the half-carry flag.
    #[inline]
    pub fn h(&self) -> bool {
        self.h.to_bool()
    }

    /// Returns the carry flag.
    #[inline]
    pub fn c(&self) -> bool {
        self.c.to_bool()
    }

    /// Encodes the flag bits into the CPU's `F` register representation.
    #[inline]
    pub fn to_byte(&self) -> u8 {
        (u8::from(self.z()) << 7)
            | (u8::from(self.n()) << 6)
            | (u8::from(self.h()) << 5)
            | (u8::from(self.c()) << 4)
    }

    /// Restores flags from an `F` register value.
    #[inline]
    pub fn from_byte(&mut self, value: u8) {
        self.set_z(value & 0x80 != 0);
        self.set_n(value & 0x40 != 0);
        self.set_h(value & 0x20 != 0);
        self.set_c(value & 0x10 != 0);
    }
}

impl<M: MaskValue + fmt::Debug> fmt::Debug for Flags<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flags")
            .field("z", &self.z)
            .field("n", &self.n)
            .field("h", &self.h)
            .field("c", &self.c)
            .finish()
    }
}

/// Execution backend abstraction used by instruction handlers.
pub trait Exec: 'static + Copy {
    /// Element type representing an 8-bit lane.
    type U8: Copy;
    /// Element type representing a 16-bit lane.
    type U16: Copy;
    /// Mask type used for flag values.
    type Mask: MaskValue;

    /// Converts a native `u8` literal into the backend element.
    fn from_u8(value: u8) -> Self::U8;
    /// Converts a native `u16` literal into the backend element.
    fn from_u16(value: u16) -> Self::U16;
    /// Casts the backend element into a native `u8`.
    fn to_u8(value: Self::U8) -> u8;
    /// Casts the backend element into a native `u16`.
    fn to_u16(value: Self::U16) -> u16;
    /// Combines two 8-bit lanes into a 16-bit value.
    fn combine_u16(high: Self::U8, low: Self::U8) -> Self::U16;
    /// Splits a 16-bit lane into its `(high, low)` components.
    fn split_u16(value: Self::U16) -> (Self::U8, Self::U8);

    /// Bitwise AND.
    fn and(a: Self::U8, b: Self::U8) -> Self::U8;
    /// Bitwise OR.
    fn or(a: Self::U8, b: Self::U8) -> Self::U8;
    /// Bitwise XOR.
    fn xor(a: Self::U8, b: Self::U8) -> Self::U8;

    /// Adds two bytes, updating flags.
    fn add8(a: Self::U8, b: Self::U8, carry: bool, f: &mut Flags<Self::Mask>) -> Self::U8;
    /// Subtracts two bytes, updating flags.
    fn sub8(a: Self::U8, b: Self::U8, borrow: bool, f: &mut Flags<Self::Mask>) -> Self::U8;

    /// Equality comparison returning a mask.
    fn eq8(a: Self::U8, b: Self::U8) -> Self::Mask;
    /// Selects between two values using the mask.
    fn select8(mask: Self::Mask, if_true: Self::U8, if_false: Self::U8) -> Self::U8;
}

/// Scalar execution backend.
#[derive(Clone, Copy)]
pub enum Scalar {}

impl Exec for Scalar {
    type U8 = u8;
    type U16 = u16;
    type Mask = bool;

    #[inline]
    fn from_u8(value: u8) -> Self::U8 {
        value
    }

    #[inline]
    fn from_u16(value: u16) -> Self::U16 {
        value
    }

    #[inline]
    fn to_u8(value: Self::U8) -> u8 {
        value
    }

    #[inline]
    fn to_u16(value: Self::U16) -> u16 {
        value
    }

    #[inline]
    fn combine_u16(high: Self::U8, low: Self::U8) -> Self::U16 {
        u16::from(high) << 8 | u16::from(low)
    }

    #[inline]
    fn split_u16(value: Self::U16) -> (Self::U8, Self::U8) {
        ((value >> 8) as u8, (value & 0x00FF) as u8)
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
        let cin = if carry { 1 } else { 0 };
        let sum = a.wrapping_add(b).wrapping_add(cin);
        let half = ((a ^ b ^ sum) & 0x10) != 0;
        let carry_out = (((a & b) | ((a | b) & !sum)) & 0x80) != 0;

        f.set_z(sum == 0);
        f.set_n(false);
        f.set_h(half);
        f.set_c(carry_out);
        sum
    }

    #[inline]
    fn sub8(a: Self::U8, b: Self::U8, borrow: bool, f: &mut Flags<Self::Mask>) -> Self::U8 {
        let cin = if borrow { 1 } else { 0 };
        let diff = a.wrapping_sub(b).wrapping_sub(cin);
        let half = ((a ^ b ^ diff) & 0x10) != 0;
        let borrow_out = (((!a & b) | ((!(a ^ b)) & diff)) & 0x80) != 0;

        f.set_z(diff == 0);
        f.set_n(true);
        f.set_h(half);
        f.set_c(borrow_out);
        diff
    }

    #[inline]
    fn eq8(a: Self::U8, b: Self::U8) -> Self::Mask {
        a == b
    }

    #[inline]
    fn select8(mask: Self::Mask, if_true: Self::U8, if_false: Self::U8) -> Self::U8 {
        if mask {
            if_true
        } else {
            if_false
        }
    }
}
