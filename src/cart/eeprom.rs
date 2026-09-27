// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (core/m93c86.c); see CREDITS.md.

//! M93C86 serial EEPROM (Microwire, 2 KB organized as 1024 16-bit words),
//! used by the GMod2 for saves.
//!
//! Protocol: with CS high, every rising edge of CLK reads one bit from DI.
//! An instruction is a start bit (1; leading zeros are ignored), two opcode
//! bits and 10 address bits; writes append 16 data bits:
//!
//! | opcode | address   | instruction                   |
//! |--------|-----------|-------------------------------|
//! | 10     | A9-A0     | READ: a 0 bit, then the data  |
//! | 01     | A9-A0     | WRITE, followed by 16 bits    |
//! | 11     | A9-A0     | ERASE (word to $FFFF)         |
//! | 00     | 11xxxxxxxx| EWEN: enable writes           |
//! | 00     | 00xxxxxxxx| EWDS: disable writes          |
//! | 00     | 10xxxxxxxx| ERAL: everything to $FF       |
//! | 00     | 01xxxxxxxx| WRAL, followed by 16 bits     |
//!
//! READ: after the last address bit DO outputs a 0, then on every rising
//! edge one data bit, most significant first; with CS still high it goes on
//! with the following words. Writes start when CS goes low again; raising
//! CS again, DO shows the status (0 busy, 1 ready). Programming time is not
//! modeled: the first status check returns busy, the following ones ready,
//! as in VICE.
//!
//! Writes are disabled at power-on.

pub const SIZE: usize = 2048;
const WORDS: u16 = (SIZE / 2) as u16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Waiting for the start bit or receiving an instruction.
    Input,
    /// READ: DO outputs the data.
    Output,
    /// After a write, with CS high DO shows the status.
    Busy,
    Ready,
}

pub struct M93c86 {
    pub data: Box<[u8; SIZE]>,
    cs: bool,
    clk: bool,
    di: bool,
    dout: bool,
    phase: Phase,
    /// Received bits (start included) and their count.
    shift: u32,
    count: u8,
    /// Received write, executed when CS falls.
    pending: bool,
    write_enabled: bool,
    /// READ: current word and bits already output.
    addr: u16,
    out_bits: u8,
    /// The contents changed since the last save to file.
    pub dirty: bool,
}

impl Default for M93c86 {
    fn default() -> Self {
        Self {
            data: Box::new([0xFF; SIZE]),
            cs: false,
            clk: false,
            di: false,
            dout: false,
            phase: Phase::Input,
            shift: 0,
            count: 0,
            pending: false,
            write_enabled: false,
            addr: 0,
            out_bits: 0,
            dirty: false,
        }
    }
}

impl M93c86 {
    /// State of the three inputs, written together by the cartridge.
    pub fn set_lines(&mut self, cs: bool, clk: bool, di: bool) {
        self.di = di;
        if cs != self.cs {
            self.set_cs(cs);
        }
        if self.cs && clk && !self.clk {
            self.clock();
        }
        self.clk = clk;
    }

    fn set_cs(&mut self, cs: bool) {
        self.cs = cs;
        if cs {
            // New instruction (the status of a write stays readable
            // until the start bit)
            self.shift = 0;
            self.count = 0;
            if self.phase == Phase::Output {
                self.phase = Phase::Input;
            }
        } else {
            if self.pending {
                self.pending = false;
                self.phase = Phase::Busy;
            } else if self.phase == Phase::Output {
                self.phase = Phase::Input;
            }
            self.shift = 0;
            self.count = 0;
        }
    }

    /// DO output as read by the CPU: the first status check after a write
    /// consumes the busy state.
    pub fn read_do(&mut self) -> bool {
        let v = self.peek_do();
        if self.cs && self.phase == Phase::Busy {
            self.phase = Phase::Ready;
        }
        v
    }

    pub fn peek_do(&self) -> bool {
        if !self.cs {
            return false;
        }
        match self.phase {
            Phase::Busy => false,
            Phase::Ready => true,
            Phase::Input | Phase::Output => self.dout,
        }
    }

    fn word(&self, addr: u16) -> u16 {
        let i = addr as usize * 2;
        u16::from_be_bytes([self.data[i], self.data[i + 1]])
    }

    fn set_word(&mut self, addr: u16, v: u16) {
        let i = addr as usize * 2;
        self.data[i..i + 2].copy_from_slice(&v.to_be_bytes());
        self.dirty = true;
    }

    fn clock(&mut self) {
        if self.phase == Phase::Output {
            // Next bit of the word, then the next word
            if self.out_bits == 16 {
                self.addr = (self.addr + 1) % WORDS;
                self.out_bits = 0;
            }
            self.dout = self.word(self.addr) & (0x8000 >> self.out_bits) != 0;
            self.out_bits += 1;
            return;
        }
        if (self.count == 0 && !self.di) || self.count >= 29 {
            return; // zeros before the start bit, or instruction already complete
        }
        if matches!(self.phase, Phase::Busy | Phase::Ready) {
            self.phase = Phase::Input;
        }
        self.shift = (self.shift << 1) | self.di as u32;
        self.count += 1;
        // The two opcode bits follow the start bit: complete from bit 3 on,
        // they are needed only once the address (13) or the data (29) is in
        let opcode = || (self.shift >> (self.count - 3)) & 3;
        match self.count {
            13 => {
                let opcode = opcode();
                let addr = (self.shift & 0x3FF) as u16;
                match opcode {
                    0b10 => {
                        self.phase = Phase::Output;
                        self.addr = addr;
                        self.out_bits = 0;
                        self.dout = false; // dummy bit
                    }
                    0b11 if self.write_enabled => {
                        self.set_word(addr, 0xFFFF);
                        self.pending = true;
                    }
                    0b00 => match addr >> 8 {
                        0b11 => self.write_enabled = true,
                        0b00 => self.write_enabled = false,
                        0b10 if self.write_enabled => {
                            self.data.fill(0xFF);
                            self.dirty = true;
                            self.pending = true;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            29 => {
                let opcode = opcode();
                let addr = ((self.shift >> 16) & 0x3FF) as u16;
                let value = self.shift as u16;
                if self.write_enabled {
                    match opcode {
                        0b01 => {
                            self.set_word(addr, value);
                            self.pending = true;
                        }
                        0b00 if addr >> 8 == 0b01 => {
                            for a in 0..WORDS {
                                self.set_word(a, value);
                            }
                            self.pending = true;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

crate::snapshot::impl_state_enum!(Phase { Input, Output, Busy, Ready });

crate::snapshot::impl_state!(M93c86 {
    data, cs, clk, di, dout, phase, shift, count, pending, write_enabled, addr, out_bits, dirty,
});

#[cfg(test)]
mod tests {
    use super::*;

    fn send(e: &mut M93c86, bits: u32, n: u32) {
        for i in (0..n).rev() {
            let b = bits >> i & 1 != 0;
            e.set_lines(true, false, b);
            e.set_lines(true, true, b);
        }
    }

    fn read_word(e: &mut M93c86, addr: u16) -> u16 {
        e.set_lines(false, false, false);
        e.set_lines(true, false, false);
        send(e, 0b110 << 10 | addr as u32, 13);
        let mut v = 0;
        for _ in 0..16 {
            e.set_lines(true, false, false);
            e.set_lines(true, true, false);
            v = v << 1 | e.read_do() as u16;
        }
        e.set_lines(false, false, false);
        v
    }

    #[test]
    fn write_needs_ewen_then_reads_back() {
        let mut e = M93c86::default();
        e.set_lines(true, false, false);
        send(&mut e, 0b101 << 26 | 5 << 16 | 0x1234, 29);
        e.set_lines(false, false, false);
        assert_eq!(read_word(&mut e, 5), 0xFFFF, "write without EWEN");

        e.set_lines(true, false, false);
        send(&mut e, 0b10011 << 8, 13); // EWEN
        e.set_lines(false, false, false);
        e.set_lines(true, false, false);
        send(&mut e, 0b101 << 26 | 5 << 16 | 0xA55A, 29);
        e.set_lines(false, false, false);
        e.set_lines(true, false, false);
        assert!(!e.read_do(), "busy right after the write");
        assert!(e.read_do(), "then ready");
        assert_eq!(read_word(&mut e, 5), 0xA55A);
        assert_eq!(&e.data[10..12], &[0xA5, 0x5A]);
        assert!(e.dirty);
    }
}
