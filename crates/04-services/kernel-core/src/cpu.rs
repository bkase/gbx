use crate::bus::Bus;
use crate::exec::{Exec, Flags};

/// CPU register file and execution state.
#[derive(Clone)]
pub struct Cpu<E: Exec> {
    pub a: E::U8,
    pub f: Flags<E::Mask>,
    pub b: E::U8,
    pub c: E::U8,
    pub d: E::U8,
    pub e: E::U8,
    pub h: E::U8,
    pub l: E::U8,
    pub sp: E::U16,
    pub pc: E::U16,
    pub ime: bool,
    pub halted: bool,
    pub enable_ime_pending: bool,
}

impl<E: Exec> Cpu<E> {
    /// Creates a CPU with power-on defaults.
    pub fn new() -> Self {
        Self {
            a: E::from_u8(0x01),
            f: {
                let mut flags = Flags::new();
                flags.from_byte(0xB0);
                flags
            },
            b: E::from_u8(0x00),
            c: E::from_u8(0x13),
            d: E::from_u8(0x00),
            e: E::from_u8(0xD8),
            h: E::from_u8(0x01),
            l: E::from_u8(0x4D),
            sp: E::from_u16(0xFFFE),
            pc: E::from_u16(0x0100),
            ime: false,
            halted: false,
            enable_ime_pending: false,
        }
    }

    /// Returns the current `HL` combined register.
    #[inline]
    pub fn hl(&self) -> E::U16 {
        E::combine_u16(self.h, self.l)
    }

    /// Updates the `HL` register pair.
    #[inline]
    pub fn set_hl(&mut self, value: E::U16) {
        let (h, l) = E::split_u16(value);
        self.h = h;
        self.l = l;
    }

    /// Returns the `BC` register pair.
    #[inline]
    pub fn bc(&self) -> E::U16 {
        E::combine_u16(self.b, self.c)
    }

    /// Updates the `BC` register pair.
    #[inline]
    pub fn set_bc(&mut self, value: E::U16) {
        let (b, c) = E::split_u16(value);
        self.b = b;
        self.c = c;
    }

    /// Returns the `DE` register pair.
    #[inline]
    pub fn de(&self) -> E::U16 {
        E::combine_u16(self.d, self.e)
    }

    /// Updates the `DE` register pair.
    #[inline]
    pub fn set_de(&mut self, value: E::U16) {
        let (d, e) = E::split_u16(value);
        self.d = d;
        self.e = e;
    }

    /// Returns the `AF` register pair.
    #[inline]
    pub fn af(&self) -> E::U16 {
        E::combine_u16(self.a, E::from_u8(self.f.to_byte()))
    }

    /// Sets the `AF` register pair.
    #[inline]
    pub fn set_af(&mut self, value: E::U16) {
        let (a, f) = E::split_u16(value);
        self.a = a;
        self.f.from_byte(E::to_u8(f) & 0xF0);
    }

    /// Reads the next byte and advances the program counter.
    pub fn fetch8<B: Bus<E>>(&mut self, bus: &mut B) -> E::U8 {
        let pc = self.pc;
        let byte = bus.read8(pc);
        let next = E::from_u16(E::to_u16(pc).wrapping_add(1));
        self.pc = next;
        byte
    }

    /// Reads the next two bytes as a little-endian 16-bit value.
    pub fn fetch16<B: Bus<E>>(&mut self, bus: &mut B) -> E::U16 {
        let lo = self.fetch8(bus);
        let hi = self.fetch8(bus);
        E::combine_u16(hi, lo)
    }

    /// Pushes a 16-bit value onto the stack.
    pub fn push16<B: Bus<E>>(&mut self, bus: &mut B, value: E::U16) {
        let mut sp = E::to_u16(self.sp);
        let (hi, lo) = E::split_u16(value);
        sp = sp.wrapping_sub(1);
        bus.write8(E::from_u16(sp), hi);
        sp = sp.wrapping_sub(1);
        bus.write8(E::from_u16(sp), lo);
        self.sp = E::from_u16(sp);
    }

    /// Pops a 16-bit value from the stack.
    pub fn pop16<B: Bus<E>>(&mut self, bus: &mut B) -> E::U16 {
        let mut sp = E::to_u16(self.sp);
        let lo = bus.read8(E::from_u16(sp));
        sp = sp.wrapping_add(1);
        let hi = bus.read8(E::from_u16(sp));
        sp = sp.wrapping_add(1);
        self.sp = E::from_u16(sp);
        E::combine_u16(hi, lo)
    }
}
