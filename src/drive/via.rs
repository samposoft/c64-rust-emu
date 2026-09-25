// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (core/viacore.c); see CREDITS.md.

//! VIA 6522 (the 1541's two), cycle-based.
//!
//! Registers: 0 ORB/IRB, 1 ORA/IRA, 2 DDRB, 3 DDRA, 4-5 T1 counter, 6-7 T1
//! latch, 8-9 T2, A SR, B ACR, C PCR, D IFR, E IER, F ORA without handshake.
//!
//! Timers, as in VICE (viacore): after writing the high byte the counter is
//! N from the next cycle, then counts down; after 0 it reads $FFFF and in that
//! cycle the interrupt flag is already set. T1 reloads from the latch the next
//! cycle ($FFFF, N, N-1, ...: period N+2), in one-shot mode too, where however
//! the interrupt fires only on the first underflow after each write. T2 keeps
//! counting down without reloading and gives one interrupt per write. Reading
//! T1L/T2L clears their flag, writing T1H (counter or latch) clears T1's.
//!
//! PA reads the pins, PB the register for output bits (on output lines
//! PB7 can be the T1 output). CA1/CB1 are edge inputs, CA2/CB2 the outputs
//! programmed by the PCR (low/high level or pulse). The shift register and
//! T2 pulse counting are not needed by the 1541 and are not emulated (the
//! SR register can be read and written).

use crate::snapshot::impl_state;

pub const IFR_CA2: u8 = 0x01;
pub const IFR_CA1: u8 = 0x02;
pub const IFR_SR: u8 = 0x04;
pub const IFR_CB2: u8 = 0x08;
pub const IFR_CB1: u8 = 0x10;
pub const IFR_T2: u8 = 0x20;
pub const IFR_T1: u8 = 0x40;

#[derive(Default)]
pub struct Via {
    pub ora: u8,
    pub orb: u8,
    pub ddra: u8,
    pub ddrb: u8,
    t1: u16,
    t1_latch: u16,
    /// The counter loads from the latch at the end of the cycle.
    t1_load: bool,
    /// T1 interrupt still pending (one-shot).
    t1_armed: bool,
    /// T1 output on PB7.
    t1_pb7: bool,
    t2: u16,
    t2_latch_lo: u8,
    t2_load: bool,
    t2_armed: bool,
    pub sr: u8,
    pub acr: u8,
    pub pcr: u8,
    pub ifr: u8,
    pub ier: u8,
    /// Levels of the CA1 and CB1 inputs.
    ca1: bool,
    cb1: bool,
}

impl Via {
    pub fn new() -> Self {
        let mut v = Self::default();
        v.reset();
        v
    }

    /// Reset: ports as inputs, interrupts off; timers and latches are kept.
    pub fn reset(&mut self) {
        self.ora = 0;
        self.orb = 0;
        self.ddra = 0;
        self.ddrb = 0;
        self.acr = 0;
        self.pcr = 0;
        self.ifr = 0;
        self.ier = 0;
        self.t1_latch = 0xFFFF;
        self.t1_armed = false;
        self.t1_pb7 = true;
        self.t2_armed = false;
    }

    /// IRQ line (asserted).
    pub fn irq(&self) -> bool {
        self.ifr & self.ier & 0x7F != 0
    }

    /// Port A outputs: output bits from the register, the others at 1
    /// (pull-up resistors).
    pub fn pa_out(&self) -> u8 {
        self.ora | !self.ddra
    }

    pub fn pb_out(&self) -> u8 {
        let v = self.orb | !self.ddrb;
        if self.acr & 0x80 != 0 { (v & 0x7F) | (self.t1_pb7 as u8) << 7 } else { v }
    }

    /// CA2 output level: low only if the PCR programs it low.
    pub fn ca2_out(&self) -> bool {
        self.pcr & 0x0E != 0x0C
    }

    pub fn cb2_out(&self) -> bool {
        self.pcr & 0xE0 != 0xC0
    }

    /// Port A read with pins `pins` (input bits).
    fn read_pa(&self, pins: u8) -> u8 {
        (self.ora & self.ddra) | (pins & !self.ddra)
    }

    fn read_pb(&self, pins: u8) -> u8 {
        let v = (self.orb & self.ddrb) | (pins & !self.ddrb);
        if self.acr & 0x80 != 0 { (v & 0x7F) | (self.t1_pb7 as u8) << 7 } else { v }
    }

    /// CA1 input: the edge selected by the PCR (bit 0: 1 rising) sets the flag.
    pub fn set_ca1(&mut self, level: bool) {
        if level != self.ca1 && level == (self.pcr & 0x01 != 0) {
            self.ifr |= IFR_CA1;
        }
        self.ca1 = level;
    }

    pub fn set_cb1(&mut self, level: bool) {
        if level != self.cb1 && level == (self.pcr & 0x10 != 0) {
            self.ifr |= IFR_CB1;
        }
        self.cb1 = level;
    }

    /// CPU read; `pa_pins`/`pb_pins`: level of the input pins.
    pub fn read(&mut self, reg: u8, pa_pins: u8, pb_pins: u8) -> u8 {
        match reg & 0x0F {
            0x0 => {
                self.ifr &= !IFR_CB1;
                if self.pcr & 0xA0 != 0x20 {
                    self.ifr &= !IFR_CB2;
                }
                self.read_pb(pb_pins)
            }
            0x1 => {
                self.ifr &= !IFR_CA1;
                if self.pcr & 0x0A != 0x02 {
                    self.ifr &= !IFR_CA2;
                }
                self.read_pa(pa_pins)
            }
            0x4 => {
                self.ifr &= !IFR_T1;
                self.t1 as u8
            }
            0x8 => {
                self.ifr &= !IFR_T2;
                self.t2 as u8
            }
            0xA => {
                self.ifr &= !IFR_SR;
                self.sr
            }
            r => self.peek(r, pa_pins, pb_pins),
        }
    }

    /// Read without side effects.
    pub fn peek(&self, reg: u8, pa_pins: u8, pb_pins: u8) -> u8 {
        match reg & 0x0F {
            0x0 => self.read_pb(pb_pins),
            0x1 | 0xF => self.read_pa(pa_pins),
            0x2 => self.ddrb,
            0x3 => self.ddra,
            0x4 => self.t1 as u8,
            0x5 => (self.t1 >> 8) as u8,
            0x6 => self.t1_latch as u8,
            0x7 => (self.t1_latch >> 8) as u8,
            0x8 => self.t2 as u8,
            0x9 => (self.t2 >> 8) as u8,
            0xA => self.sr,
            0xB => self.acr,
            0xC => self.pcr,
            0xD => self.ifr | if self.irq() { 0x80 } else { 0 },
            _ => self.ier | 0x80,
        }
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        match reg & 0x0F {
            0x0 => {
                self.ifr &= !IFR_CB1;
                if self.pcr & 0xA0 != 0x20 {
                    self.ifr &= !IFR_CB2;
                }
                self.orb = v;
            }
            0x1 => {
                self.ifr &= !IFR_CA1;
                if self.pcr & 0x0A != 0x02 {
                    self.ifr &= !IFR_CA2;
                }
                self.ora = v;
            }
            0xF => self.ora = v,
            0x2 => self.ddrb = v,
            0x3 => self.ddra = v,
            0x4 | 0x6 => self.t1_latch = (self.t1_latch & 0xFF00) | v as u16,
            0x5 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | (v as u16) << 8;
                self.t1_load = true;
                self.t1_armed = true;
                self.t1_pb7 = false;
                self.ifr &= !IFR_T1;
            }
            0x7 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | (v as u16) << 8;
                self.ifr &= !IFR_T1;
            }
            0x8 => self.t2_latch_lo = v,
            0x9 => {
                self.t2 = (v as u16) << 8 | self.t2_latch_lo as u16;
                self.t2_load = true;
                self.t2_armed = true;
                self.ifr &= !IFR_T2;
            }
            0xA => {
                self.sr = v;
                self.ifr &= !IFR_SR;
            }
            0xB => {
                if (self.acr ^ v) & 0x80 != 0 && v & 0x80 != 0 {
                    self.t1_pb7 = true;
                }
                self.acr = v;
            }
            0xC => self.pcr = v,
            0xD => self.ifr &= !v,
            _ => {
                if v & 0x80 != 0 {
                    self.ier |= v & 0x7F;
                } else {
                    self.ier &= !v;
                }
            }
        }
    }

    /// End of a cycle: the timers advance.
    #[inline]
    pub fn tick(&mut self) {
        if self.t1_load {
            self.t1_load = false;
            self.t1 = self.t1_latch;
        } else if self.t1 == 0 {
            self.t1 = 0xFFFF;
            // Reload from the latch on the next cycle, in every mode
            self.t1_load = true;
            if self.acr & 0x40 != 0 {
                self.ifr |= IFR_T1;
                self.t1_pb7 = !self.t1_pb7;
            } else if self.t1_armed {
                self.t1_armed = false;
                self.ifr |= IFR_T1;
                self.t1_pb7 = true;
            }
        } else {
            self.t1 -= 1;
        }
        if self.t2_load {
            // The written value is visible from the next cycle
            self.t2_load = false;
        } else if self.acr & 0x20 == 0 {
            if self.t2 == 0 && self.t2_armed {
                self.t2_armed = false;
                self.ifr |= IFR_T2;
            }
            self.t2 = self.t2.wrapping_sub(1);
        }
    }
}

impl_state!(Via {
    ora, orb, ddra, ddrb, t1, t1_latch, t1_load, t1_armed, t1_pb7, t2, t2_latch_lo, t2_load,
    t2_armed, sr, acr, pcr, ifr, ier, ca1, cb1,
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_free_running_period_and_flag() {
        let mut v = Via::new();
        v.write(0xB, 0x40); // free-run
        v.write(0x4, 3);
        v.write(0x5, 0); // at cycle W
        v.tick();
        let mut seen = Vec::new();
        for _ in 0..7 {
            seen.push((v.peek(4, 0, 0), v.ifr & IFR_T1 != 0));
            v.tick();
        }
        // 3, 2, 1, 0, $FF (flag), 3 ...: period N+2
        assert_eq!(seen, [(3, false), (2, false), (1, false), (0, false), (0xFF, true), (3, true), (2, true)]);
        assert_eq!(v.read(4, 0, 0), 1);
        assert_eq!(v.ifr & IFR_T1, 0, "T1L read: flag cleared");
    }

    #[test]
    fn t2_one_shot_keeps_counting_without_second_irq() {
        let mut v = Via::new();
        v.write(0x8, 1);
        v.write(0x9, 0);
        v.tick();
        assert_eq!(v.peek(8, 0, 0), 1);
        v.tick();
        v.tick();
        assert_eq!((v.peek(8, 0, 0), v.peek(9, 0, 0), v.ifr & IFR_T2 != 0), (0xFF, 0xFF, true));
        v.write(0xD, IFR_T2);
        for _ in 0..0x10000 {
            v.tick();
        }
        assert_eq!(v.ifr & IFR_T2, 0);
    }

    #[test]
    fn ca1_edge_and_irq() {
        let mut v = Via::new();
        v.write(0xC, 0x01); // CA1 on the rising edge
        v.write(0xE, 0x80 | IFR_CA1);
        v.set_ca1(false);
        assert!(!v.irq());
        v.set_ca1(true);
        assert!(v.irq());
        assert_eq!(v.peek(0xD, 0, 0), 0x80 | IFR_CA1);
        v.read(1, 0, 0);
        assert!(!v.irq());
    }
}
