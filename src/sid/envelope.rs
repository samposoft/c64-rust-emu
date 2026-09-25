// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from reSID 1.0 as shipped with VICE 3.10,
// Copyright (C) 2010 Dag Lem <resid@nimrod.no>.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! ADSR envelope generator (reSID 1.0 as shipped in VICE 3.10,
//! envelope.h/envelope.cc), clocked one cycle at a time.

use super::dac::build_dac_table;
use super::Model;

/// Rate counter period (cycles per step, minus 1) for attack, decay and
/// release: the same table for all three, as on the chip.
const RATE_PERIOD: [u16; 16] = [
    8, 31, 62, 94, 148, 219, 266, 312, 391, 976, 1953, 3125, 3906, 11719, 19531, 31250,
];

/// 8-bit envelope DAC (6581: 2R/R 2.20 without termination; 8580:
/// 2.00 with termination).
static MODEL_DAC: [[u16; 256]; 2] = [
    build_dac_table(8, 2.20, false),
    build_dac_table(8, 2.00, true),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvState { Attack, DecaySustain, Release }

/// Envelope generator, clocked every cycle with the chip's latencies
/// (behaviour verified on ENV3 as in reSID 1.0):
/// - a 15-bit rate counter advances up to the selected period; if the
///   period is lowered below the current value the counter keeps going
///   up to $7FFF and wraps to 0 ("ADSR delay bug");
/// - in decay and release an exponential counter further divides the
///   steps (1, 2, 4, 8, 16, 30 below $FF, $5D, $36, $1A, $0E, $06);
/// - the 8-bit counter step arrives 1-2 cycles late, and the state
///   change after a gate edge 2-3 cycles late;
/// - once at 0 the counter stays frozen until an attack restarts;
/// - ENV3 reads the value from the previous cycle.
pub struct Envelope {
    pub counter: u8,
    pub env3: u8,
    rate_counter: u16,
    rate_period: u16,
    reset_rate: bool,
    exp_counter: u8,
    exp_period: u8,
    // Cycles left before a counter step, an exponential counter step
    // and a state change (0 = none pending)
    env_pipeline: u8,
    exp_pipeline: u8,
    state_pipeline: u8,
    hold_zero: bool,
    state: EnvState,
    next_state: EnvState,
    gate: bool,
    attack: u8,
    decay: u8,
    sustain: u8,
    release: u8,
}

impl Envelope {
    pub fn new() -> Self {
        let mut e = Self {
            // At power-on the odd bits of the counter are high
            counter: 0xAA,
            env3: 0,
            rate_counter: 0,
            rate_period: RATE_PERIOD[0],
            reset_rate: false,
            exp_counter: 0,
            exp_period: 1,
            env_pipeline: 0,
            exp_pipeline: 0,
            state_pipeline: 0,
            hold_zero: false,
            state: EnvState::Release,
            next_state: EnvState::Release,
            gate: false,
            attack: 0, decay: 0, sustain: 0, release: 0,
        };
        e.reset();
        e
    }

    /// Chip reset: the counter does not change.
    pub fn reset(&mut self) {
        self.env_pipeline = 0;
        self.exp_pipeline = 0;
        self.state_pipeline = 0;
        self.attack = 0;
        self.decay = 0;
        self.sustain = 0;
        self.release = 0;
        self.gate = false;
        self.rate_counter = 0;
        self.exp_counter = 0;
        self.exp_period = 1;
        self.reset_rate = false;
        self.state = EnvState::Release;
        self.rate_period = RATE_PERIOD[self.release as usize];
        self.hold_zero = false;
    }

    fn sustain_level(&self) -> u8 {
        self.sustain * 0x11
    }

    pub fn write_control(&mut self, control: u8) {
        let gate = control & 0x01 != 0;
        if gate == self.gate {
            return;
        }
        self.gate = gate;
        if gate {
            // In the first attack cycle the decay register is erroneously
            // active; the real attack starts two cycles later
            self.next_state = EnvState::Attack;
            self.state = EnvState::DecaySustain;
            self.rate_period = RATE_PERIOD[self.decay as usize];
            self.state_pipeline = 2;
            if self.reset_rate || self.exp_pipeline == 2 {
                self.env_pipeline = if self.exp_period == 1 || self.exp_pipeline == 2 { 2 } else { 4 };
            } else if self.exp_pipeline == 1 {
                self.state_pipeline = 3;
            }
        } else {
            self.next_state = EnvState::Release;
            self.state_pipeline = if self.env_pipeline > 0 { 3 } else { 2 };
        }
    }

    pub fn write_attack_decay(&mut self, val: u8) {
        self.attack = val >> 4;
        self.decay = val & 0x0F;
        match self.state {
            EnvState::Attack => self.rate_period = RATE_PERIOD[self.attack as usize],
            EnvState::DecaySustain => self.rate_period = RATE_PERIOD[self.decay as usize],
            EnvState::Release => {}
        }
    }

    pub fn write_sustain_release(&mut self, val: u8) {
        self.sustain = val >> 4;
        self.release = val & 0x0F;
        if self.state == EnvState::Release {
            self.rate_period = RATE_PERIOD[self.release as usize];
        }
    }

    fn state_change(&mut self) {
        self.state_pipeline -= 1;
        match self.next_state {
            EnvState::Attack => {
                if self.state_pipeline == 0 {
                    self.state = EnvState::Attack;
                    self.rate_period = RATE_PERIOD[self.attack as usize];
                    self.hold_zero = false;
                }
            }
            EnvState::Release => {
                if (self.state == EnvState::Attack && self.state_pipeline == 0)
                    || (self.state == EnvState::DecaySustain && self.state_pipeline == 1)
                {
                    self.state = EnvState::Release;
                    self.rate_period = RATE_PERIOD[self.release as usize];
                }
            }
            EnvState::DecaySustain => {}
        }
    }

    /// Exponential counter period based on the level reached.
    fn update_exp_period(&mut self) {
        match self.counter {
            0xFF => self.exp_period = 1,
            0x5D => self.exp_period = 2,
            0x36 => self.exp_period = 4,
            0x1A => self.exp_period = 8,
            0x0E => self.exp_period = 16,
            0x06 => self.exp_period = 30,
            0x00 => {
                self.exp_period = 1;
                self.hold_zero = true;
            }
            _ => {}
        }
    }

    /// `cycles` clock cycles. In the common case (no delay pending, rate
    /// counter below the period) the only effect of each cycle is the
    /// rate counter increment: it is done in one go.
    pub fn clock_n(&mut self, mut cycles: u32) {
        while cycles > 0 {
            let idle = self.state_pipeline == 0 && self.env_pipeline == 0
                && self.exp_pipeline == 0 && !self.reset_rate
                && self.rate_counter < self.rate_period;
            if idle {
                let k = cycles.min((self.rate_period - self.rate_counter) as u32);
                self.rate_counter += k as u16;
                self.env3 = self.counter;
                cycles -= k;
            } else {
                self.clock();
                cycles -= 1;
            }
        }
    }

    #[inline]
    pub fn clock(&mut self) {
        self.env3 = self.counter;

        if self.state_pipeline != 0 {
            self.state_change();
        }

        if self.env_pipeline != 0 {
            self.env_pipeline -= 1;
            if self.env_pipeline == 0 && !self.hold_zero {
                match self.state {
                    EnvState::Attack => {
                        self.counter = self.counter.wrapping_add(1);
                        if self.counter == 0xFF {
                            self.state = EnvState::DecaySustain;
                            self.rate_period = RATE_PERIOD[self.decay as usize];
                        }
                    }
                    EnvState::DecaySustain | EnvState::Release => {
                        self.counter = self.counter.wrapping_sub(1);
                    }
                }
                self.update_exp_period();
            }
        }

        let exp_step = if self.exp_pipeline != 0 {
            self.exp_pipeline -= 1;
            self.exp_pipeline == 0
        } else {
            false
        };
        if exp_step {
            self.exp_counter = 0;
            if (self.state == EnvState::DecaySustain && self.counter != self.sustain_level())
                || self.state == EnvState::Release
            {
                self.env_pipeline = 1;
            }
        } else if self.reset_rate {
            self.rate_counter = 0;
            self.reset_rate = false;
            if self.state == EnvState::Attack {
                // In attack every rate step is a counter step
                self.exp_counter = 0;
                self.env_pipeline = 2;
            } else if !self.hold_zero {
                self.exp_counter = self.exp_counter.wrapping_add(1);
                if self.exp_counter == self.exp_period {
                    self.exp_pipeline = if self.exp_period != 1 { 2 } else { 1 };
                }
            }
        }

        if self.rate_counter != self.rate_period {
            self.rate_counter += 1;
            if self.rate_counter & 0x8000 != 0 {
                self.rate_counter = (self.rate_counter + 1) & 0x7FFF;
            }
        } else {
            self.reset_rate = true;
        }
    }

    /// 8-bit DAC output, with the model's imperfections.
    #[inline]
    pub fn output(&self, model: Model) -> i32 {
        MODEL_DAC[model as usize][self.counter as usize] as i32
    }
}

use crate::snapshot::{impl_state, impl_state_enum};

impl_state_enum!(EnvState { Attack, DecaySustain, Release });

impl_state!(Envelope {
    counter, env3, rate_counter, rate_period, reset_rate, exp_counter, exp_period,
    env_pipeline, exp_pipeline, state_pipeline, hold_zero, state, next_state, gate,
    attack, decay, sustain, release,
});
