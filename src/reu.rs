// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Behaviour modelled on VICE 3.10 (c64/cart/reu.c); see CREDITS.md.

//! RAM Expansion Unit (Commodore 1700/1764/1750 and larger REUs up to
//! 16 MB), with the REC 8726 controller.
//!
//! Registers at $DF00-$DF0A, mirrored every 32 bytes in $DF00-$DFFF:
//!
//! | reg | contents                                                         |
//! |-----|------------------------------------------------------------------|
//! | 00  | status (read only): 7 IRQ, 6 end of block, 5 verify error, 4 256K chips; bits 7-5 are cleared on read |
//! | 01  | command: 7 execute, 5 autoload, 4 no trigger at $FF00, 1-0 type (C64→REU, REU→C64, swap, verify) |
//! | 02-03 | C64 address                                                    |
//! | 04-06 | REU address (bank in 06; bits 3-7 read as 1)                   |
//! | 07-08 | length (0 = 65536)                                             |
//! | 09  | IRQ: 7 enable, 6 on end of block, 5 on verify error              |
//! | 0A  | 7 fixed C64 address, 6 fixed REU address                         |
//!
//! The command starts immediately (bit 4 set) or on the first CPU write to
//! $FF00. The transfer begins on the next CPU read, and the CPU stays halted
//! for its whole duration: the REU uses one cycle per byte (two for swap)
//! and waits when the VIC requests the bus. REU reads stop as soon as BA is
//! low; a write still goes through in the first cycle with BA low. Accesses
//! to C64 memory go through the bus (I/O included), except $00/$01, which
//! are always RAM.
//!
//! Writes to half of the address and length registers: the written byte is
//! combined with the autoload shadow register, not with the current value
//! (the REC "Half-Autoload-Bug"). Without autoload, at the end of the
//! transfer the addresses point past the last byte and the length is 1.
//! The behaviour, timing included, is that of VICE (x64sc), which it is
//! compared against in tests/vice_mem.rs.

use crate::mem::Bus;
use crate::snapshot::{Reader, Result as StateResult, State, Writer};

const STATUS_IRQ: u8 = 0x80;
const STATUS_EOB: u8 = 0x40;
const STATUS_VERIFY: u8 = 0x20;
const STATUS_256K: u8 = 0x10;
const CMD_EXECUTE: u8 = 0x80;
const CMD_AUTOLOAD: u8 = 0x20;
const CMD_NO_FF00: u8 = 0x10;
const IRQ_ENABLE: u8 = 0x80;
const IRQ_ON_EOB: u8 = 0x40;
const IRQ_ON_VERIFY: u8 = 0x20;
const FIX_C64: u8 = 0x80;
const FIX_REU: u8 = 0x40;

/// Allowed sizes, in KB.
pub const SIZES: [u32; 8] = [128, 256, 512, 1024, 2048, 4096, 8192, 16384];

/// DRAM contents at power-on, observed on a 1764 (the same as VICE):
/// alternating $FF/$00 byte pairs, inverted every 256 bytes, plus some
/// inverted stripes in every group of four 64K banks.
fn power_on_ram(size: usize) -> Vec<u8> {
    let mut ram: Vec<u8> = (0..size)
        .map(|i| {
            let pairs = if ((i + 1) / 2) & 1 != 0 { 0xFF } else { 0x00 };
            let pages = if (i / 0x100) & 1 != 0 { 0xFF } else { 0x00 };
            0xFF ^ pairs ^ pages
        })
        .collect();
    let mut invert = |start: usize, len: usize| {
        for b in ram.iter_mut().take(start + len).skip(start) {
            *b ^= 0xFF;
        }
    };
    for bank in (0..size >> 16).step_by(4) {
        for i in 0..2 {
            let base = (bank + i) << 16;
            invert(base + 0x2A00, 0x2A00);
            invert(base + 0x8000, 0x2C00);
            invert(base + 0xD600, 0x2A00);
            invert(base + 0x20000, 0x2A00);
            invert(base + 0x25400, 0x2C00);
            invert(base + 0x2AC00, 0x2A00);
        }
    }
    ram
}

/// Outcome of a DMA cycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaCycle {
    /// The REU owns the bus, the CPU is halted.
    Busy,
    /// No transfer: the cycle belongs to the CPU.
    Free,
    /// Transfer just finished: the CPU uses the cycle and completes its
    /// access even if the VIC has already pulled BA low.
    FreeIgnoreBa,
}

/// Condition for the next REU access.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gate {
    /// Immediately.
    Now,
    /// On the first cycle with BA high (after a read, and at the start).
    BaHigh,
    /// After a write: with BA low one more access goes through; on the
    /// second cycle of BA low the REU stops until BA is high.
    AfterWrite,
    /// Stopped after a write, until BA is high.
    Stolen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    /// C64 → REU, REU → C64, verify: one byte per cycle.
    Stash,
    Fetch,
    Verify,
    /// Swap: read from the C64, then write in the next cycle.
    SwapRead,
    SwapWrite,
    /// Extra cycle without access (failed verify, write finished while
    /// the REU was stopped), then end.
    Extra,
    /// End: registers updated, the CPU uses this cycle.
    End,
}

#[derive(Clone, Copy, Debug)]
struct Dma {
    op: Op,
    gate: Gate,
    host: u16,
    reu: u32,
    host_step: u16,
    reu_step: u32,
    /// Bytes still to transfer.
    len: u32,
    /// Consecutive cycles with BA low seen after writes.
    delay: u8,
    /// The last write stopped the REU: one extra cycle at end of block.
    last_cycle: bool,
    /// Swap: byte read from the C64, to be written to the REU.
    c64_value: u8,
    /// Status bits at end of transfer.
    status: u8,
}

pub struct Reu {
    ram: Vec<u8>,
    // Configuration, from the size
    wrap_around: u32,
    dram_wrap: u32,
    store_mask: u32,
    bank_unused: u8,
    status_preset: u8,
    // Registers
    status: u8,
    command: u8,
    c64_addr: u16,
    reu_addr: u16,
    bank: u8,
    length: u16,
    int_mask: u8,
    addr_ctrl: u8,
    c64_shadow: u16,
    reu_shadow: u16,
    bank_shadow: u8,
    length_shadow: u16,
    /// Command waiting for the write to $FF00.
    armed: bool,
    /// Transfer requested: it starts on the next CPU read.
    requested: bool,
    dma: Option<Dma>,
    /// Last byte through the REU bus latch: it is read back from
    /// addresses without DRAM.
    floating: u8,
    irq: bool,
}

impl Default for Reu {
    fn default() -> Self {
        Self::new(512)
    }
}

impl Reu {
    /// Size in KB.
    pub fn kb(&self) -> u32 {
        (self.ram.len() / 1024) as u32
    }

    /// REU of `kb` KB (128 = 1700, 256 = 1764, 512 = 1750, above: modified
    /// REUs and 1750 XL, with the high bank bits outside the REC).
    pub fn new(kb: u32) -> Self {
        assert!(SIZES.contains(&kb), "invalid REU size: {kb} KB");
        let size = kb * 1024;
        let (mut wrap_around, mut dram_wrap, mut store_mask) = (0x80000, 0x80000, 0x7FFFF);
        let (mut bank_unused, mut status_preset) = (0xF8, STATUS_256K);
        if kb == 128 {
            // 1700: 64K chips, the address wraps at 128K
            status_preset = 0;
            wrap_around = 0x20000;
            dram_wrap = 0x20000;
        } else if kb > 512 {
            bank_unused = 0;
            dram_wrap = size;
            store_mask = size - 1;
        }
        let mut r = Self {
            ram: power_on_ram(size as usize),
            wrap_around,
            dram_wrap,
            store_mask,
            bank_unused,
            status_preset,
            status: 0,
            command: 0,
            c64_addr: 0,
            reu_addr: 0,
            bank: 0,
            length: 0,
            int_mask: 0,
            addr_ctrl: 0,
            c64_shadow: 0,
            reu_shadow: 0,
            bank_shadow: 0,
            length_shadow: 0,
            armed: false,
            requested: false,
            dma: None,
            floating: 0xFF,
            irq: false,
        };
        r.reset();
        r
    }

    pub fn size_kb(&self) -> u32 {
        self.ram.len() as u32 / 1024
    }

    /// Reset: registers as at power-on, the RAM is kept.
    pub fn reset(&mut self) {
        self.status = self.status_preset;
        self.command = CMD_NO_FF00;
        self.c64_addr = 0;
        self.c64_shadow = 0;
        self.reu_addr = 0;
        self.reu_shadow = 0;
        self.bank = self.bank_unused;
        self.bank_shadow = self.bank_unused;
        self.length = 0xFFFF;
        self.length_shadow = 0xFFFF;
        self.int_mask = 0x1F;
        self.addr_ctrl = 0x3F;
        self.armed = false;
        self.requested = false;
        self.dma = None;
        self.irq = false;
    }

    /// IRQ line to the CPU.
    pub fn irq(&self) -> bool {
        self.irq
    }

    // ── Registers ─────────────────────────────────────────────────────────────

    /// Read without side effects (debugger).
    pub fn peek(&self, off: u8) -> u8 {
        match off & 0x1F {
            0x00 => self.status,
            0x01 => self.command,
            0x02 => self.c64_addr as u8,
            0x03 => (self.c64_addr >> 8) as u8,
            0x04 => self.reu_addr as u8,
            0x05 => (self.reu_addr >> 8) as u8,
            0x06 => self.bank | self.bank_unused,
            0x07 => self.length as u8,
            0x08 => (self.length >> 8) as u8,
            0x09 => self.int_mask,
            0x0A => self.addr_ctrl,
            _ => 0xFF,
        }
    }

    /// CPU read: reading the status clears bits 7-5 and the IRQ.
    pub fn read(&mut self, off: u8) -> u8 {
        let v = self.peek(off);
        match off & 0x1F {
            0x00 => {
                self.status &= !(STATUS_IRQ | STATUS_EOB | STATUS_VERIFY);
                self.irq = false;
                v
            }
            // On modified REUs the high bank bits do not read back
            0x06 => v | 0xF8,
            _ => v,
        }
    }

    pub fn write(&mut self, off: u8, v: u8) {
        let lo = |r: u16| (r & 0xFF00) | v as u16;
        let hi = |r: u16| (r & 0x00FF) | (v as u16) << 8;
        match off & 0x1F {
            0x01 => {
                self.command = v;
                if v & CMD_EXECUTE != 0 {
                    if v & CMD_NO_FF00 != 0 {
                        self.armed = false;
                        self.requested = true;
                    } else {
                        self.armed = true;
                    }
                }
            }
            0x02 => { self.c64_shadow = lo(self.c64_shadow); self.c64_addr = self.c64_shadow; }
            0x03 => { self.c64_shadow = hi(self.c64_shadow); self.c64_addr = self.c64_shadow; }
            0x04 => { self.reu_shadow = lo(self.reu_shadow); self.reu_addr = self.reu_shadow; }
            0x05 => { self.reu_shadow = hi(self.reu_shadow); self.reu_addr = self.reu_shadow; }
            0x06 => { self.bank_shadow = v & !self.bank_unused; self.bank = self.bank_shadow; }
            0x07 => { self.length_shadow = lo(self.length_shadow); self.length = self.length_shadow; }
            0x08 => { self.length_shadow = hi(self.length_shadow); self.length = self.length_shadow; }
            0x09 => {
                self.int_mask = v | 0x1F;
                // An event that already happened raises the IRQ as soon as it is enabled
                self.check_irq(self.status);
            }
            0x0A => self.addr_ctrl = v | 0x3F,
            _ => {}
        }
    }

    fn check_irq(&mut self, events: u8) {
        let on = |bit: u8| self.int_mask & (IRQ_ENABLE | bit) == IRQ_ENABLE | bit;
        if (events & STATUS_EOB != 0 && on(IRQ_ON_EOB)) || (events & STATUS_VERIFY != 0 && on(IRQ_ON_VERIFY)) {
            self.status |= STATUS_IRQ;
            self.irq = true;
        }
    }

    /// CPU write to $FF00: starts the pending command.
    /// Returns true if it started it.
    pub fn ff00_written(&mut self) -> bool {
        if self.armed {
            self.armed = false;
            self.requested = true;
            true
        } else {
            false
        }
    }

    // ── RAM ───────────────────────────────────────────────────────────────────

    fn ram_read(&self, addr: u32) -> u8 {
        let a = (addr & (self.dram_wrap - 1)) as usize;
        self.ram.get(a).copied().unwrap_or(self.floating)
    }

    fn ram_write(&mut self, addr: u32, v: u8) {
        let a = (addr & (self.dram_wrap - 1)) as usize;
        if let Some(b) = self.ram.get_mut(a) {
            *b = v;
        }
    }

    /// The REC counts on 19 bits and wraps at 512K (128K on the 1700); the
    /// bits above (large REUs) do not change.
    fn next_reu(&self, addr: u32, step: u32) -> u32 {
        let mut next = (addr & 0x7FFFF) + step;
        if next == self.wrap_around {
            next = 0;
        }
        (addr & 0xF80000) | next
    }

    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    // ── DMA ───────────────────────────────────────────────────────────────────

    /// True if the REU owns the bus in this cycle, or takes it because the
    /// CPU is about to read (`cpu_writes`: the next CPU cycle is a write,
    /// which still goes through).
    pub fn wants_bus(&mut self, cpu_writes: bool) -> bool {
        if self.requested && !cpu_writes {
            self.requested = false;
            self.start();
        }
        self.dma.is_some()
    }

    fn start(&mut self) {
        let op = match self.command & 3 {
            0 => Op::Stash,
            1 => Op::Fetch,
            2 => Op::SwapRead,
            _ => Op::Verify,
        };
        self.dma = Some(Dma {
            op,
            // At the start the REU waits for the VIC to release the bus
            gate: Gate::BaHigh,
            host: self.c64_addr,
            reu: self.reu_addr as u32 | (self.bank as u32) << 16,
            host_step: if self.addr_ctrl & FIX_C64 != 0 { 0 } else { 1 },
            reu_step: if self.addr_ctrl & FIX_REU != 0 { 0 } else { 1 },
            len: if self.length == 0 { 0x10000 } else { self.length as u32 },
            delay: 0,
            last_cycle: false,
            c64_value: 0,
            status: 0,
        });
    }

    /// One transfer cycle, with the VIC's `ba_low` for this cycle.
    /// `bus.reu` is empty during the call: REU reads and writes to its own
    /// registers have no effect, as on the REC during DMA.
    pub fn dma_cycle(&mut self, bus: &mut Bus, ba_low: bool) -> DmaCycle {
        let Some(mut d) = self.dma else { return DmaCycle::Free };

        match d.gate {
            Gate::Now => {}
            Gate::BaHigh | Gate::Stolen if ba_low => {
                self.dma = Some(d);
                return DmaCycle::Busy;
            }
            Gate::BaHigh => {}
            Gate::Stolen => d.delay = 0,
            Gate::AfterWrite => {
                if ba_low {
                    d.delay += 1;
                } else {
                    d.delay = 0;
                }
                d.last_cycle = d.delay > 1;
                if d.last_cycle {
                    d.gate = Gate::Stolen;
                    self.dma = Some(d);
                    return DmaCycle::Busy;
                }
            }
        }
        d.gate = Gate::Now;
        match d.op {
            Op::Stash => {
                let v = bus.dma_read(d.host);
                self.ram_write(d.reu, v);
                self.floating = v;
                self.advance(&mut d);
                d.gate = Gate::BaHigh;
                if d.len == 0 {
                    d.op = Op::End;
                    d.status = STATUS_EOB;
                }
            }
            Op::Fetch => {
                let v = self.ram_read(d.reu);
                self.floating = v;
                bus.dma_write(d.host, v);
                self.advance(&mut d);
                d.gate = Gate::AfterWrite;
                if d.len == 0 {
                    d.op = Op::End;
                    d.status = STATUS_EOB;
                    // After the block the latch holds the next byte
                    self.floating = self.ram_read(d.reu);
                }
            }
            Op::SwapRead => {
                d.c64_value = bus.dma_read(d.host);
                d.op = Op::SwapWrite;
                d.gate = Gate::BaHigh;
            }
            Op::SwapWrite => {
                let v = self.ram_read(d.reu);
                self.ram_write(d.reu, d.c64_value);
                bus.dma_write(d.host, v);
                self.advance(&mut d);
                d.gate = Gate::AfterWrite;
                if d.len == 0 {
                    d.op = Op::End;
                    d.status = STATUS_EOB;
                } else {
                    d.op = Op::SwapRead;
                }
            }
            Op::Verify => {
                let r = self.ram_read(d.reu);
                let c = bus.dma_read(d.host);
                self.advance(&mut d);
                d.gate = Gate::BaHigh;
                if r != c {
                    d.status |= STATUS_VERIFY;
                    // A failed verify costs one extra cycle, unless it was
                    // the last byte
                    d.op = if d.len >= 1 { Op::Extra } else { Op::End };
                } else if d.len == 0 {
                    d.op = Op::End;
                }
            }
            Op::Extra => {
                d.op = Op::End;
                d.gate = Gate::BaHigh;
            }
            Op::End => {
                if d.last_cycle {
                    // The final write stopped the REU: one extra cycle
                    d.last_cycle = false;
                    d.gate = Gate::BaHigh;
                } else {
                    self.finish(bus, d);
                    // The CPU resumes with its access even if BA is already
                    // low (a single cycle, as in VICE)
                    return DmaCycle::FreeIgnoreBa;
                }
            }
        }
        self.dma = Some(d);
        DmaCycle::Busy
    }

    fn advance(&self, d: &mut Dma) {
        d.host = d.host.wrapping_add(d.host_step);
        d.reu = self.next_reu(d.reu, d.reu_step);
        d.len -= 1;
    }

    /// End of the transfer: registers, status and IRQ.
    fn finish(&mut self, bus: &mut Bus, mut d: Dma) {
        let mut status = d.status;
        if matches!(self.command & 3, 3) {
            if d.len == 0 {
                // All equal, or mismatch on the last byte: end of block
                d.len = 1;
                status |= STATUS_EOB;
            } else if d.len == 1 {
                // Mismatch on the second to last: end of block if the last one matches
                if self.ram_read(d.reu) == bus.dma_read(d.host) {
                    status |= STATUS_EOB;
                }
            }
        } else {
            d.len = 1;
        }
        let reu = d.reu & self.store_mask;
        self.status |= status;
        if self.command & CMD_AUTOLOAD != 0 {
            self.c64_addr = self.c64_shadow;
            self.reu_addr = self.reu_shadow;
            self.bank = self.bank_shadow;
            self.length = self.length_shadow;
        } else {
            if self.addr_ctrl & FIX_C64 == 0 {
                self.c64_addr = d.host;
            }
            if self.addr_ctrl & FIX_REU == 0 {
                self.reu_addr = reu as u16;
                self.bank = (reu >> 16) as u8;
            }
            self.length = d.len as u16;
        }
        self.check_irq(status);
        self.command = (self.command & !CMD_EXECUTE) | CMD_NO_FF00;
        self.dma = None;
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

crate::snapshot::impl_state_enum!(Gate { Now, BaHigh, AfterWrite, Stolen });
crate::snapshot::impl_state_enum!(Op { Stash, Fetch, Verify, SwapRead, SwapWrite, Extra, End });
crate::snapshot::impl_state!(Dma {
    op, gate, host, reu, host_step, reu_step, len, delay, last_cycle, c64_value, status,
});

impl Default for Dma {
    fn default() -> Self {
        Self {
            op: Op::End, gate: Gate::Now, host: 0, reu: 0, host_step: 0, reu_step: 0, len: 0,
            delay: 0, last_cycle: false, c64_value: 0, status: 0,
        }
    }
}

/// The RAM is saved as a block.
impl State for Reu {
    fn save(&self, w: &mut Writer) {
        let Self {
            ram, wrap_around, dram_wrap, store_mask, bank_unused, status_preset, status, command,
            c64_addr, reu_addr, bank, length, int_mask, addr_ctrl, c64_shadow, reu_shadow,
            bank_shadow, length_shadow, armed, requested, dma, floating, irq,
        } = self;
        (ram.len() as u32).save(w);
        w.put(ram);
        for v in [wrap_around, dram_wrap, store_mask] {
            v.save(w);
        }
        for v in [bank_unused, status_preset, status, command, bank, int_mask, addr_ctrl, bank_shadow, floating] {
            v.save(w);
        }
        for v in [c64_addr, reu_addr, length, c64_shadow, reu_shadow, length_shadow] {
            v.save(w);
        }
        for v in [armed, requested, irq] {
            v.save(w);
        }
        dma.save(w);
    }

    fn load(&mut self, r: &mut Reader) -> StateResult<()> {
        let Self {
            ram, wrap_around, dram_wrap, store_mask, bank_unused, status_preset, status, command,
            c64_addr, reu_addr, bank, length, int_mask, addr_ctrl, c64_shadow, reu_shadow,
            bank_shadow, length_shadow, armed, requested, dma, floating, irq,
        } = self;
        let mut n = 0u32;
        n.load(r)?;
        *ram = r.take(n as usize)?.to_vec();
        for v in [wrap_around, dram_wrap, store_mask] {
            v.load(r)?;
        }
        for v in [bank_unused, status_preset, status, command, bank, int_mask, addr_ctrl, bank_shadow, floating] {
            v.load(r)?;
        }
        for v in [c64_addr, reu_addr, length, c64_shadow, reu_shadow, length_shadow] {
            v.load(r)?;
        }
        for v in [armed, requested, irq] {
            v.load(r)?;
        }
        dma.load(r)
    }
}
