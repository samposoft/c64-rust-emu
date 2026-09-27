// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (core/ciacore.c, core/ciatimer.c); see CREDITS.md.

//! MOS 6526 CIA ("old" model, as `-ciamodel 0` in VICE): timers A/B
//! with start, load and one-shot latencies, B cascaded, timer outputs
//! on PB6/PB7, interrupt register with its delays, TOD clock
//! with alarm and read latch, serial port output. Advances one cycle
//! at a time with `tick`; CPU reads and writes in the same cycle
//! see the state after the tick.
//!
//! Timers and ICR follow the chip's behaviour as modelled by VICE
//! (ciatimer.h, ciacore.c; written from the description, not copied).


// ── Timer ────────────────────────────────────────────────────────────────────

// Timer state bits. The first four come from the control register;
// the others are pipeline stages that advance by one cycle.
const CR_START: u16 = 0x001;
const COUNT2: u16 = 0x002;
/// A pulse to count (timer B cascaded on A's underflow).
const STEP: u16 = 0x004;
const CR_ONESHOT: u16 = 0x008;
const CR_FLOAD: u16 = 0x010;
/// Counts phi2 (control register bit 5 at 0).
const PHI2IN: u16 = 0x020;
const COUNT3: u16 = 0x040;
const LOAD1: u16 = 0x080;
const ONESHOT0: u16 = 0x100;
/// The counter is loaded from the latch in this cycle.
const LOAD: u16 = 0x200;
/// Underflow in this cycle (PB6/PB7 output in pulse mode).
const OUT: u16 = 0x400;
const COUNT: u16 = 0x800;
const ONESHOT: u16 = 0x1000;
const CR_MASK: u16 = CR_START | CR_ONESHOT | CR_FLOAD | PHI2IN;

pub struct Timer {
    state: u16,
    pub cnt: u16,
    pub latch: u16,
}

impl Timer {
    fn new() -> Self {
        Self { state: 0, cnt: 0xFFFF, latch: 0xFFFF }
    }

    /// Next cycle's state: start+phi2 becomes counting after three
    /// stages, force load after two, one-shot after two.
    fn next_state(t: u16) -> u16 {
        let mut n = t & (CR_START | CR_ONESHOT | PHI2IN);
        if t & CR_START != 0 && t & PHI2IN != 0 { n |= COUNT2; }
        if t & COUNT2 != 0 || (t & STEP != 0 && t & CR_START != 0) { n |= COUNT3; }
        if t & COUNT3 != 0 { n |= COUNT; }
        if t & CR_FLOAD != 0 { n |= LOAD1; }
        if t & LOAD1 != 0 { n |= LOAD; }
        if t & CR_ONESHOT != 0 { n |= ONESHOT0; }
        if t & ONESHOT0 != 0 { n |= ONESHOT; }
        n
    }

    /// One cycle. Returns true on underflow: the counter reaches 0 and
    /// is reloaded in the same cycle (so 0 is never read in continuous
    /// mode; period = latch + 1).
    #[inline(always)]
    fn step(&mut self) -> bool {
        let t = self.state;
        // Common cases with no pipeline stages in flight (one-shot must be
        // settled): timer stopped, or counting far from zero.
        // The only effect of the cycle is then clearing OUT and decrementing.
        let oneshot = t & (CR_ONESHOT | ONESHOT0 | ONESHOT);
        if oneshot == 0 || oneshot == CR_ONESHOT | ONESHOT0 | ONESHOT {
            const MOVING: u16 = CR_START | COUNT2 | COUNT3 | COUNT | CR_FLOAD | LOAD1 | LOAD | STEP;
            match t & (MOVING | PHI2IN) {
                x if x & MOVING == 0 => {
                    self.state = t & !OUT;
                    return false;
                }
                x if x == CR_START | PHI2IN | COUNT2 | COUNT3 | COUNT && self.cnt > 1 => {
                    self.cnt -= 1;
                    self.state = t & !OUT;
                    return false;
                }
                _ => {}
            }
        }
        if self.cnt != 0 && t & COUNT3 != 0 {
            self.cnt -= 1;
        }
        let mut t = Self::next_state(t);
        let under = self.cnt == 0 && t & COUNT3 != 0;
        if under {
            t |= LOAD | OUT;
        }
        if t & LOAD != 0 {
            self.cnt = self.latch;
            t &= !COUNT3;
        }
        if t & OUT != 0 && t & (ONESHOT | ONESHOT0) != 0 {
            t &= !(CR_START | COUNT2);
        }
        self.state = t;
        under
    }

    /// Control register write (bits 0, 3, 4, 5).
    fn set_ctrl(&mut self, byte: u8) {
        self.state = (self.state & !CR_MASK) | ((byte as u16 & CR_MASK) ^ PHI2IN);
    }

    fn set_latch_lo(&mut self, v: u8) {
        self.latch = (self.latch & 0xFF00) | v as u16;
        if self.state & LOAD != 0 {
            self.cnt = (self.cnt & 0xFF00) | v as u16;
        }
    }

    /// With the timer stopped (or loading), writing the high byte loads the counter.
    fn set_latch_hi(&mut self, v: u8) {
        self.latch = (self.latch & 0x00FF) | ((v as u16) << 8);
        if self.state & LOAD != 0 || self.state & CR_START == 0 {
            self.cnt = self.latch;
        }
    }

    /// A pulse to count on the next cycle (cascade).
    fn single_step(&mut self) {
        if self.state & CR_START != 0 {
            self.state |= STEP;
        }
    }

    pub fn running(&self) -> bool {
        self.state & CR_START != 0
    }

    fn underflow_now(&self) -> bool {
        self.state & OUT != 0
    }
}

// ── Interrupt register ───────────────────────────────────────────────────────

// ICR delay line: each "1" bit becomes "0" on the next cycle.
const IRQ_ACK1: u16 = 0x0001;
const IRQ_ACK0: u16 = 0x0002;
const IRQ_ACK_1: u16 = 0x0004;
const IRQ_ACK_2: u16 = 0x0008;
const IRQ_D7SET1: u16 = 0x0010;
const IRQ_D7SET0: u16 = 0x0020;
const IRQ_D7SET_1: u16 = 0x0040;
const IRQ_RAISE1: u16 = 0x0100;
const IRQ_RAISE0: u16 = 0x0200;
const IRQ_RAISE_1: u16 = 0x0400;
const IRQ_READ0: u16 = 0x1000;
const IRQ_READ1: u16 = 0x2000;
const IRQ_READ2: u16 = 0x4000;
const IRQ_CLEAR: u16 = IRQ_ACK_2 | IRQ_D7SET_1 | IRQ_RAISE_1 | IRQ_READ2;

/// ICR bit 7 (some enabled source has its flag set).
const IM_SET: u16 = 0x80;
/// Timer B underflowed the cycle after an ICR read: its
/// flag is lost on the next read (6526 bug).
const IM_TBB: u16 = 0x100;

/// CIA model (VICE's `CIA_MODEL_6526` and `CIA_MODEL_6526A`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Model {
    /// The NMOS 6526 of the first C64s.
    #[default]
    Mos6526,
    /// The HMOS 6526A (8521) of the C64C: an interrupt raises the line in
    /// the cycle its flag is set, one cycle before the 6526; an ICR read
    /// clears the flags one cycle later; no timer B bug.
    Mos6526A,
}

impl Model {
    pub fn parse(s: &str) -> Option<Model> {
        match s.to_ascii_lowercase().as_str() {
            "6526" | "old" => Some(Model::Mos6526),
            "6526a" | "8521" | "new" => Some(Model::Mos6526A),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Model::Mos6526 => "6526",
            Model::Mos6526A => "6526A",
        }
    }
}

crate::snapshot::impl_state_enum!(Model { Mos6526, Mos6526A });

pub struct CiaState {
    /// Chip model (`set_model`), kept by reset.
    pub model: Model,
    /// "Raw" registers: ports, DDR, CRA/CRB as written (bit 4 at 0; bit 0
    /// does not follow the one-shot stop, which is read from the timer).
    pub regs: [u8; 16],
    pub ta: Timer,
    pub tb: Timer,
    /// PB6/PB7 toggle outputs: high at start, inverted at each underflow.
    tat: bool,
    tbt: bool,

    /// ICR flags (bits 0-4), bit 7 and the internal timer B bug bit.
    pub icr_flags: u16,
    /// Mask of the enabled interrupts.
    pub icr_mask: u8,
    /// Flags set in this cycle, not yet passed to the delay line.
    new_flags: u16,
    /// Flags an ICR read clears a cycle later (6526A).
    ack_flags: u16,
    ifr_delay: u16,
    /// IRQ (CIA1) or NMI (CIA2) line towards the CPU.
    irq_line: bool,
    /// Cycles since power-on and cycle of the last ICR read.
    clock: u64,
    icr_read_clock: u64,

    /// TOD: [tenths, seconds, minutes, hours] in BCD, hours with bit 7 = PM.
    pub tod: [u8; 4],
    pub tod_alarm: [u8; 4],
    /// Values frozen from the hours read until the tenths read.
    tod_latch: Option<[u8; 4]>,
    /// Clock stopped from the hours write until the tenths write.
    pub tod_stopped: bool,
    tod_cycles: u32,
    /// Cycles per period of the mains, the TOD input: 50 Hz (PAL) or 60
    /// Hz (NTSC), `set_tod_input`.
    tod_tick_cycles: u32,
    tod_prescale: u8,

    /// Serial data register and output shift state.
    pub sdr: u8,
    sdr_shifting: bool,
    sdr_pending: bool,
    sdr_count: u8,
}

impl Default for CiaState {
    fn default() -> Self { Self::new() }
}

fn bcd_inc(v: u8) -> u8 {
    if v & 0x0F == 9 { (v & 0xF0) + 0x10 } else { v + 1 }
}

impl CiaState {
    pub fn new() -> Self {
        Self {
            model: Model::Mos6526,
            regs: [0u8; 16],
            ta: Timer::new(),
            tb: Timer::new(),
            tat: false,
            tbt: false,
            icr_flags: 0,
            icr_mask: 0,
            new_flags: 0,
            ack_flags: 0,
            ifr_delay: 0,
            irq_line: false,
            clock: 0,
            icr_read_clock: u64::MAX,
            tod: [0, 0, 0, 0x01],
            tod_alarm: [0, 0, 0, 0],
            tod_latch: None,
            tod_stopped: false,
            tod_cycles: 0,
            tod_tick_cycles: crate::timing::Standard::Pal.clock_hz() as u32 / 50,
            tod_prescale: 0,
            sdr: 0,
            sdr_shifting: false,
            sdr_pending: false,
            sdr_count: 0,
        }
    }

    /// RES line (C64 reset): registers, timers, interrupts, serial port and
    /// TOD as after power-on, as VICE's ciacore_reset, which also stops the
    /// TOD until the tenths are written; the cycle counter goes on.
    /// Frequency of the mains at the TOD input (50 or 60 Hz) with the CPU
    /// clock `clock_hz`: the TOD counts one tenth of a second every 5 or 6
    /// periods, according to CRA bit 7.
    pub fn set_tod_input(&mut self, clock_hz: u64, mains_hz: u32) {
        self.tod_tick_cycles = (clock_hz / mains_hz as u64) as u32;
    }

    pub fn reset(&mut self) {
        let (clock, tod_tick_cycles, model) = (self.clock, self.tod_tick_cycles, self.model);
        *self = Self::new();
        self.clock = clock;
        self.model = model;
        // The mains at the TOD pin is part of the board, not of the chip
        self.tod_tick_cycles = tod_tick_cycles;
        self.tod_stopped = true;
    }

    // ── Ports ─────────────────────────────────────────────────────────────────

    /// PB bits forced by the timer outputs (PBON): (mask, value).
    pub fn pb_timer_bits(&self) -> (u8, u8) {
        let mut mask = 0u8;
        let mut val = 0u8;
        let outs = [
            (self.regs[0x0E], 0x40u8, self.tat, self.ta.underflow_now()),
            (self.regs[0x0F], 0x80u8, self.tbt, self.tb.underflow_now()),
        ];
        for (cr, bit, toggle, pulse) in outs {
            if cr & 0x02 != 0 {
                mask |= bit;
                let on = if cr & 0x04 != 0 { toggle } else { pulse };
                if on { val |= bit; }
            }
        }
        (mask, val)
    }

    /// Generic port B (CIA2, user port): output bits from the register, input
    /// bits at 1 (pull-up), plus the timer outputs.
    fn port_b(&self) -> u8 {
        let ddr = self.regs[3];
        let pb = (self.regs[1] & ddr) | !ddr;
        let (m, v) = self.pb_timer_bits();
        (pb & !m) | v
    }

    // ── Read ──────────────────────────────────────────────────────────────────

    fn tod_read(&self, i: usize) -> u8 {
        self.tod_latch.map_or(self.tod[i], |l| l[i])
    }

    /// Read without side effects (debugger and peek).
    pub fn read(&self, reg: u8) -> u8 {
        match reg & 0x0F {
            0x00 => self.regs[0],
            0x01 => self.port_b(),
            0x02 => self.regs[2],
            0x03 => self.regs[3],
            0x04 => (self.ta.cnt & 0xFF) as u8,
            0x05 => (self.ta.cnt >> 8) as u8,
            0x06 => (self.tb.cnt & 0xFF) as u8,
            0x07 => (self.tb.cnt >> 8) as u8,
            0x08 => self.tod_read(0),
            0x09 => self.tod_read(1),
            0x0A => self.tod_read(2),
            0x0B => self.tod_read(3),
            0x0C => self.sdr,
            0x0D => self.icr_flags as u8,
            0x0E => (self.regs[0x0E] & !0x01) | self.ta.running() as u8,
            _ => (self.regs[0x0F] & !0x01) | self.tb.running() as u8,
        }
    }

    /// CPU read with side effects: ICR cleared, TOD latch/unlatch.
    pub fn read_mut(&mut self, reg: u8) -> u8 {
        match reg & 0x0F {
            0x0D => self.read_icr(),
            0x08 => {
                let v = self.tod_read(0);
                self.tod_latch = None;
                v
            }
            0x0B => {
                if self.tod_latch.is_none() { self.tod_latch = Some(self.tod); }
                self.tod_read(3)
            }
            r => self.read(r),
        }
    }

    /// ICR read: returns flags and bit 7 and releases the IRQ line. The
    /// 6526 clears the flags at once and bit 7 two cycles later; the 6526A
    /// clears flags and bit 7 on the next cycle (those set meanwhile stay),
    /// and returns bit 7 also for an interrupt raised in this cycle.
    fn read_icr(&mut self) -> u8 {
        if self.icr_flags & IM_TBB != 0 {
            self.icr_flags &= !(IM_TBB | 0x02);
        }
        let result;
        if self.model == Model::Mos6526A {
            if self.ifr_delay & IRQ_RAISE0 != 0 && self.icr_flags & 0x1F != 0 {
                self.icr_flags |= IM_SET;
            }
            if self.icr_flags & 0x9F != 0 {
                self.ack_flags |= (self.icr_flags & 0x9F) | IM_SET;
            }
            self.ifr_delay |= IRQ_ACK1;
            self.ifr_delay &= !(IRQ_RAISE0 | IRQ_D7SET0);
            result = self.icr_flags as u8;
        } else {
            self.ifr_delay |= IRQ_ACK1;
            self.ifr_delay &= !IRQ_RAISE0;
            result = self.icr_flags as u8;
            self.icr_flags &= IM_SET;
            self.new_flags = 0;
        }
        self.ifr_delay |= IRQ_READ0;
        self.irq_line = false;
        self.icr_read_clock = self.clock;
        result
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    pub fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x0F {
            0x04 => self.ta.set_latch_lo(val),
            0x05 => self.ta.set_latch_hi(val),
            0x06 => self.tb.set_latch_lo(val),
            0x07 => self.tb.set_latch_hi(val),
            0x08..=0x0B => {
                let i = (reg & 0x0F) as usize - 8;
                let v = match i { 0 => val & 0x0F, 3 => val & 0x9F, _ => val & 0x7F };
                if self.regs[0x0F] & 0x80 != 0 {
                    self.tod_alarm[i] = v;
                } else {
                    self.tod[i] = v;
                    // Writing the hours stops the clock, writing the tenths restarts it
                    if i == 3 { self.tod_stopped = true; }
                    if i == 0 { self.tod_stopped = false; self.tod_prescale = 0; }
                }
            }
            0x0C => {
                self.sdr = val;
                if self.regs[0x0E] & 0x40 != 0 {
                    if self.sdr_shifting { self.sdr_pending = true; }
                    else { self.sdr_shifting = true; self.sdr_count = 0; }
                }
            }
            0x0D => {
                // ICR mask: bit7=1 → set bits, bit7=0 → clear bits
                if val & 0x80 != 0 {
                    self.icr_mask |= val & 0x7F;
                } else {
                    self.icr_mask &= !(val & 0x7F);
                }
                if self.icr_flags & self.icr_mask as u16 & 0x7F != 0 {
                    // A flag already set and now enabled: bit 7 and IRQ after the
                    // usual delays (an already active line stays as it is): on
                    // the 6526A at once, unless the ICR was read a cycle before
                    if !self.irq_line {
                        if self.model == Model::Mos6526A {
                            if self.ifr_delay & IRQ_READ1 == 0 {
                                self.ifr_delay |= IRQ_RAISE0 | IRQ_D7SET0;
                            }
                        } else {
                            self.ifr_delay |= IRQ_RAISE1 | IRQ_D7SET1;
                        }
                    }
                } else if self.model == Model::Mos6526 && self.ifr_delay & IRQ_ACK_1 != 0 {
                    self.ifr_delay &= !(IRQ_RAISE0 | IRQ_D7SET0);
                }
            }
            0x0E => {
                if val & 0x01 != 0 && self.regs[0x0E] & 0x01 == 0 {
                    self.tat = true; // the toggle output starts high on start
                }
                if (val ^ self.regs[0x0E]) & 0x40 != 0 {
                    // Serial port direction change
                    self.sdr_shifting = false;
                    self.sdr_pending = false;
                    self.sdr_count = 0;
                }
                self.ta.set_ctrl(val);
                self.regs[0x0E] = val & !0x10;
            }
            0x0F => {
                if val & 0x01 != 0 && self.regs[0x0F] & 0x01 == 0 {
                    self.tbt = true;
                }
                // Modes 2 and 3 (A underflow, CNT high): no phi2, only STEP
                self.tb.set_ctrl(if val & 0x40 != 0 { val | 0x20 } else { val });
                self.regs[0x0F] = val & !0x10;
            }
            r => self.regs[r as usize] = val,
        }
    }

    /// IRQ (CIA1) or NMI (CIA2) line towards the CPU.
    pub fn irq_active(&self) -> bool {
        self.irq_line
    }

    // ── Clock ─────────────────────────────────────────────────────────────────

    /// Falling edge on the FLAG line (tape read on
    /// CIA1): ICR bit 4, with the interrupt if enabled.
    pub fn flag_edge(&mut self) {
        self.set_flag(0x10);
    }

    fn set_flag(&mut self, bits: u16) {
        self.icr_flags |= bits;
        self.new_flags |= bits;
        // A flag set after a read is not cleared by it (6526A)
        self.ack_flags &= !bits;
    }

    fn tod_tick(&mut self) {
        // Prescaler: the input is 50 Hz; CRA bit 7 tells the chip whether to expect
        // 50 Hz (divide by 5) or 60 Hz (divide by 6, and the clock runs slow).
        self.tod_prescale += 1;
        let div = if self.regs[0x0E] & 0x80 != 0 { 5 } else { 6 };
        if self.tod_prescale < div { return; }
        self.tod_prescale = 0;

        let t = &mut self.tod;
        t[0] += 1;
        if t[0] > 9 {
            t[0] = 0;
            t[1] = bcd_inc(t[1]);
            if t[1] > 0x59 {
                t[1] = 0;
                t[2] = bcd_inc(t[2]);
                if t[2] > 0x59 {
                    t[2] = 0;
                    let pm = t[3] & 0x80;
                    let mut h = bcd_inc(t[3] & 0x1F);
                    let mut pm2 = pm;
                    if h == 0x12 { pm2 ^= 0x80; }      // 11:59 → 12:00 flips AM/PM
                    if h == 0x13 { h = 0x01; }         // 12:59 → 1:00
                    t[3] = h | pm2;
                }
            }
        }
        if self.tod == self.tod_alarm {
            self.set_flag(0x04);
        }
    }

    /// Advances by `cycles` cycles (one at a time).
    /// Returns `true` if the IRQ line is active after advancing.
    #[inline]
    pub fn tick(&mut self, cycles: u8) -> bool {
        for _ in 0..cycles { self.tick1(); }
        self.irq_active()
    }

    #[inline(always)]
    fn tick1(&mut self) {
        self.clock += 1;

        // Timer A (phi2; CNT is tied to the pull-up and never pulses)
        let ta_under = self.ta.step();
        if ta_under {
            self.set_flag(0x01);
            self.tat = !self.tat;
            // Serial port output: one bit every two underflows, IRQ after 8 bits
            if self.regs[0x0E] & 0x40 != 0 && self.sdr_shifting {
                self.sdr_count += 1;
                if self.sdr_count >= 16 {
                    self.set_flag(0x08);
                    self.sdr_count = 0;
                    self.sdr_shifting = self.sdr_pending;
                    self.sdr_pending = false;
                }
            }
        }

        // Timer B; when cascaded it receives a pulse from A's underflow, which
        // counts from the next cycle
        let tb_under = self.tb.step();
        if ta_under && self.regs[0x0F] & 0x41 == 0x41 {
            self.tb.single_step();
        }
        if tb_under {
            self.set_flag(0x02);
            self.tbt = !self.tbt;
            if self.model == Model::Mos6526 && self.icr_read_clock == self.clock - 1 {
                self.icr_flags |= IM_TBB;
            } else {
                self.icr_flags &= !IM_TBB;
            }
        }

        // TOD
        self.tod_cycles += 1;
        if self.tod_cycles >= self.tod_tick_cycles {
            self.tod_cycles = 0;
            if !self.tod_stopped { self.tod_tick(); }
        }

        self.ifr_cycle();
    }

    /// One cycle of the ICR delay line (VICE's cia_run_ifr_cycle): an
    /// enabled flag set in this cycle brings bit 7 and IRQ on the next cycle
    /// (6526A: in this one, unless the ICR was read in the previous cycle);
    /// a read clears bit 7 two cycles later (6526A: flags and bit 7 one
    /// cycle later).
    fn ifr_cycle(&mut self) {
        if self.ifr_delay == 0 && self.new_flags == 0 {
            return;
        }
        let new_model = self.model == Model::Mos6526A;
        let mut d = self.ifr_delay;
        if d & IRQ_ACK0 != 0 {
            if new_model {
                self.icr_flags &= !self.ack_flags;
            } else {
                self.icr_flags &= !IM_SET;
            }
            self.ack_flags = 0;
        }
        if self.new_flags & self.icr_mask as u16 & 0x1F != 0 {
            if new_model && self.icr_read_clock.wrapping_add(1) != self.clock {
                d |= IRQ_RAISE0 | IRQ_D7SET0;
            } else {
                d |= IRQ_RAISE1 | IRQ_D7SET1;
            }
        }
        if d & IRQ_D7SET0 != 0 {
            self.icr_flags |= IM_SET;
        }
        if d & IRQ_RAISE0 != 0 {
            self.irq_line = true;
        }
        self.new_flags = 0;
        self.ifr_delay = (d << 1) & !IRQ_CLEAR;
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

use crate::snapshot::impl_state;

impl_state!(Timer { state, cnt, latch });

impl_state!(CiaState {
    model, regs, ta, tb, tat, tbt, icr_flags, icr_mask, new_flags, ack_flags, ifr_delay, irq_line,
    clock, icr_read_clock,
    tod, tod_alarm, tod_latch, tod_stopped, tod_cycles, tod_prescale,
    sdr, sdr_shifting, sdr_pending, sdr_count,
} skip { tod_tick_cycles });
