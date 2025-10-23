use crate::bus::BusScalar;

/// Trait capturing the minimal IO interface used by the timers.
pub trait TimerIo {
    /// Reads the divider register.
    fn read_div(&self) -> u8;
    /// Writes the divider register.
    fn write_div(&mut self, value: u8);
    /// Returns whether the divider was reset since the last poll.
    fn take_div_reset(&mut self) -> bool;
    /// Returns the most recent TIMA write value if one occurred.
    fn take_tima_write(&mut self) -> Option<u8>;
    /// Returns the most recent TMA write value if one occurred.
    fn take_tma_write(&mut self) -> Option<u8>;
    /// Returns the most recent TAC write value if one occurred.
    fn take_tac_write(&mut self) -> Option<(u8, u8)>;
    /// Reads the TIMA counter.
    fn read_tima(&self) -> u8;
    /// Writes the TIMA counter.
    fn write_tima(&mut self, value: u8);
    /// Reads the timer modulo.
    fn read_tma(&self) -> u8;
    /// Reads the timer control register.
    fn read_tac(&self) -> u8;
    /// Reads the interrupt flag register.
    fn read_if(&self) -> u8;
    /// Writes the interrupt flag register.
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
    fn take_div_reset(&mut self) -> bool {
        let pending = self.timer_div_reset;
        if pending {
            self.timer_div_reset = false;
        }
        pending
    }

    #[inline]
    fn take_tima_write(&mut self) -> Option<u8> {
        self.timer_tima_write.take()
    }

    #[inline]
    fn take_tma_write(&mut self) -> Option<u8> {
        self.timer_tma_write.take()
    }

    #[inline]
    fn take_tac_write(&mut self) -> Option<(u8, u8)> {
        self.timer_tac_write.take()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimaState {
    Running,
    Reloading,
    Reloaded,
}

impl Default for TimaState {
    fn default() -> Self {
        TimaState::Running
    }
}

/// Game Boy timer block (DIV + TIMA/TMA/TAC).
#[derive(Default, Clone)]
pub struct Timers {
    pub(crate) div_counter: u16,
    pub(crate) timer_input: bool,
    pub(crate) tima_state: TimaState,
}

impl Timers {
    /// Creates a fresh timer instance.
    pub fn new() -> Self {
        Self {
            div_counter: 8,
            timer_input: false,
            tima_state: TimaState::Running,
        }
    }

    /// Resets counters to their initial values.
    pub fn reset(&mut self) {
        self.div_counter = 8;
        self.timer_input = false;
        self.tima_state = TimaState::Running;
    }

    /// Initializes the divider register to the post-boot hardware value.
    pub fn initialize_post_boot<T: TimerIo>(&mut self, io: &mut T) {
        let low = std::env::var("GBX_TIMER_SYNC_LOW")
            .ok()
            .and_then(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(8);
        self.div_counter = ((0x03u16) << 8) | u16::from(low);
        self.timer_input = self.compute_timer_input(io.read_tac());
        self.tima_state = TimaState::Running;
        io.write_div((self.div_counter >> 8) as u8);
    }

    /// Returns internal counters for diagnostics.
    pub fn debug_state(&self) -> (u16, bool, bool, u8) {
        (
            self.div_counter,
            self.timer_input,
            matches!(self.tima_state, TimaState::Reloading),
            match self.tima_state {
                TimaState::Running => 0,
                TimaState::Reloading => 1,
                TimaState::Reloaded => 2,
            },
        )
    }

    /// Updates the divider counter while preserving the lower-phase approximation used by the bootstrap ROM.
    pub fn sync_div_from_high_byte(&mut self, high: u8) {
        let low = std::env::var("GBX_TIMER_SYNC_LOW")
            .ok()
            .and_then(|s| u8::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(8);
        self.div_counter = ((high as u16) << 8) | u16::from(low);
    }

    /// Steps the timer block by `cycles`.
    pub fn step<T: TimerIo>(&mut self, cycles: u32, io: &mut T) {
        self.process_register_writes(io);

        let mut remaining = cycles;
        while remaining > 0 {
            self.advance_tima_state(io);

            let step = remaining.min(4);
            self.apply_div_increment(step as u16, io);

            remaining = remaining.saturating_sub(step);
        }
    }

    fn process_register_writes<T: TimerIo>(&mut self, io: &mut T) {
        while io.take_div_reset() {
            let prev_input = self.timer_input;
            self.div_counter = 0;
            io.write_div(0);
            let tac = io.read_tac();
            let new_input = self.compute_timer_input(tac);
            if prev_input && !new_input {
                self.increment_tima(io);
                if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                    eprintln!("DIV reset triggered TIMA increment");
                }
            }
            self.timer_input = new_input;
        }

        if let Some(_) = io.take_tima_write() {
            if matches!(self.tima_state, TimaState::Reloaded) {
                // Writes ignored per hardware, state unchanged.
            }
        }

        while let Some(value) = io.take_tma_write() {
            if !matches!(self.tima_state, TimaState::Running) {
                io.write_tima(value);
            }
        }

        while let Some((old_tac, new_tac)) = io.take_tac_write() {
            if old_tac & 0x04 != 0 {
                let old_mask = Self::tac_trigger_bit(old_tac);
                let new_mask = Self::tac_trigger_bit(new_tac);
                if self.div_counter & old_mask != 0 {
                    if new_tac & 0x04 == 0 || self.div_counter & new_mask == 0 {
                        self.increment_tima(io);
                        if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                            eprintln!("TAC write triggered TIMA increment");
                        }
                    }
                }
            }
            self.timer_input = self.compute_timer_input(new_tac);
        }
    }

    fn compute_timer_input(&self, tac: u8) -> bool {
        if tac & 0x04 == 0 {
            return false;
        }
        let bit = match tac & 0x03 {
            0x00 => 9,
            0x01 => 3,
            0x02 => 5,
            0x03 => 7,
            _ => unreachable!(),
        };
        ((self.div_counter >> bit) & 0x01) != 0
    }

    fn increment_tima<T: TimerIo>(&mut self, io: &mut T) {
        let tima = io.read_tima();
        if tima == 0xFF {
            io.write_tima(io.read_tma());
            self.tima_state = TimaState::Reloading;
            if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                eprintln!(
                    "TIMA overflow: scheduling reload with TMA={:02X}",
                    io.read_tma()
                );
            }
        } else {
            io.write_tima(tima.wrapping_add(1));
            if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                eprintln!("TIMA incremented to {:02X}", io.read_tima());
            }
        }
    }

    fn advance_tima_state<T: TimerIo>(&mut self, io: &mut T) {
        match self.tima_state {
            TimaState::Running => {}
            TimaState::Reloading => {
                let mut if_reg = io.read_if();
                if_reg |= 0x04;
                io.write_if(if_reg);
                if std::env::var_os("GBX_TRACE_TIMER").is_some() {
                    eprintln!(
                        "TIMA reloaded to {:02X}; IF now {:02X}",
                        io.read_tima(),
                        if_reg
                    );
                }
                self.tima_state = TimaState::Reloaded;
            }
            TimaState::Reloaded => {
                self.tima_state = TimaState::Running;
            }
        }
    }

    fn tac_trigger_bit(tac: u8) -> u16 {
        match tac & 0x03 {
            0x00 => 1 << 9,
            0x01 => 1 << 3,
            0x02 => 1 << 5,
            _ => 1 << 7,
        }
    }

    fn apply_div_increment<T: TimerIo>(&mut self, delta: u16, io: &mut T) {
        if delta == 0 {
            return;
        }

        let old = self.div_counter;
        let new = old.wrapping_add(delta);
        self.div_counter = new;
        io.write_div((self.div_counter >> 8) as u8);

        let triggers = old & !new;
        let tac = io.read_tac();
        if tac & 0x04 != 0 {
            let mask = Self::tac_trigger_bit(tac);
            if triggers & mask != 0 {
                self.increment_tima(io);
            }
        }

        self.timer_input = self.compute_timer_input(tac);
    }
}
