use crate::bus::BusScalar;

/// Trait capturing the minimal IO interface used by the timers.
pub trait TimerIo {
    fn read_div(&self) -> u8;
    fn write_div(&mut self, value: u8);
    fn read_tima(&self) -> u8;
    fn write_tima(&mut self, value: u8);
    fn read_tma(&self) -> u8;
    fn read_tac(&self) -> u8;
    fn read_if(&self) -> u8;
    fn write_if(&mut self, value: u8);
}

impl TimerIo for BusScalar {
    #[inline]
    fn read_div(&self) -> u8 {
        self.io.div()
    }

    #[inline]
    fn write_div(&mut self, value: u8) {
        self.io.set_div(value);
    }

    #[inline]
    fn read_tima(&self) -> u8 {
        self.io.tima()
    }

    #[inline]
    fn write_tima(&mut self, value: u8) {
        self.io.set_tima(value);
    }

    #[inline]
    fn read_tma(&self) -> u8 {
        self.io.tma()
    }

    #[inline]
    fn read_tac(&self) -> u8 {
        self.io.tac()
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

/// Game Boy timer block (DIV + TIMA/TMA/TAC).
#[derive(Default, Clone)]
pub struct Timers {
    pub(crate) div_counter: u32,
    pub(crate) tima_counter: u32,
}

impl Timers {
    /// Creates a fresh timer instance.
    pub fn new() -> Self {
        Self {
            div_counter: 0,
            tima_counter: 0,
        }
    }

    /// Resets counters to their initial values.
    pub fn reset(&mut self) {
        self.div_counter = 0;
        self.tima_counter = 0;
    }

    /// Steps the timer block by `cycles`.
    pub fn step<T: TimerIo>(&mut self, cycles: u32, io: &mut T) {
        self.div_counter = self.div_counter.wrapping_add(cycles);
        io.write_div((self.div_counter >> 8) as u8);

        let tac = io.read_tac();
        if tac & 0x04 == 0 {
            return;
        }

        let period = match tac & 0x03 {
            0x00 => 1024,
            0x01 => 16,
            0x02 => 64,
            0x03 => 256,
            _ => unreachable!(),
        };

        self.tima_counter = self.tima_counter.wrapping_add(cycles);
        while self.tima_counter >= period {
            self.tima_counter -= period;
            let tima = io.read_tima();
            if tima == 0xFF {
                io.write_tima(io.read_tma());
                let mut if_reg = io.read_if();
                if_reg |= 0x04;
                io.write_if(if_reg);
            } else {
                io.write_tima(tima.wrapping_add(1));
            }
        }
    }
}
