// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// The CIA1 port reads (keyboard matrix) follow VICE 3.10 c64/c64cia1.c,
// Copyright (C) Andre Fachat, Ettore Perazzoli, Andreas Boose, Marco van den
// Heuvel; the processor port follows c64/c64pla.c and c64/c64memsc.c,
// Copyright (C) Andreas Boose, Ettore Perazzoli, Marco van den Heuvel.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

pub mod power_on;

use crate::cia::CiaState;
use crate::vic::VicState;
use crate::keyboard::KeyMatrix;
use crate::sid::Sid;
use crate::cart::{CartState, Mode};
use crate::reu::Reu;
use crate::drive::Drive;

/// Debugger watchpoint on an address range (CPU accesses).
#[derive(Clone, Copy, Debug)]
pub struct Watch {
    pub start: u16,
    pub end: u16,
    pub on_read: bool,
    pub on_write: bool,
}

/// Access that triggered a watchpoint.
#[derive(Clone, Copy, Debug)]
pub struct WatchHit {
    pub addr: u16,
    pub val: u8,
    pub write: bool,
    /// PC of the instruction that made the access.
    pub pc: u16,
}

/// Banking result for a CPU read.
enum Mapped {
    Byte(u8),
    Cia1(u8),
    Cia2(u8),
    /// $D01E/$D01F: the CPU read clears the register.
    Vic(u8),
    /// $D41B/$D41C: the SID must be brought to the current cycle before the read.
    Sid(u8),
    /// Second SID.
    Sid2(u8),
    /// Ethernet cartridge (the address).
    Eth(u16),
    /// Cartridge: ROM and I/O areas, whose reads can switch bank or
    /// return the state of a flash chip.
    Roml(u16),
    Romh(u16),
    Io1(u8),
    Io2(u8),
}

/// Cycles for which bits 6-7 of the processor port keep their charge as
/// inputs: VICE uses 350000 plus a random 0-70000; the middle of that range,
/// so runs stay reproducible.
const PORT_FALL_OFF: u64 = 385_000;

pub struct Bus {
    pub ram: [u8; 0x10000],
    pub kernal_rom: [u8; 0x2000],
    pub basic_rom: [u8; 0x2000],
    pub char_rom: [u8; 0x1000],
    pub color_ram: [u8; 0x0400],

    pub cart: Option<CartState>,

    /// RAM Expansion Unit, if attached (registers in I/O2, $DF00).
    pub reu: Option<Reu>,
    /// Ethernet cartridge (RR-Net, TFE), if attached: 16 bytes of I/O1 or
    /// I/O2, which take precedence over the cartridge and the REU.
    pub eth: Option<Box<crate::net::EthernetCart>>,
    /// 1541 drive on the serial bus, if emulated.
    pub drive: Option<Box<Drive>>,
    /// The REU DMA was started by the first write of a
    /// read-modify-write to $FF00: the second one does not happen.
    skip_write: bool,

    // 6510 I/O port
    pub cpu_port_dir: u8,
    pub cpu_port_data: u8,
    /// Last value driven on each pin while it was an output: an input
    /// without pull-up (bit 3) reads it back (VICE: pport.data_out).
    cpu_port_out: u8,
    /// Bits 6 and 7 have no pin: as inputs they read the charge left by the
    /// last value written while they were outputs, until the cycle it has
    /// leaked away (VICE: data_set_bit6/7 and the fall-off time).
    port_charge: [u8; 2],
    port_charge_until: [u64; 2],

    // Peripherals
    pub vic: VicState,
    pub sid: Sid,
    /// Second SID (stereo music), at address `sid2_base`: 32 bytes between
    /// $D420 and $D7E0 or in the cartridge I/O areas, where it takes
    /// precedence.
    pub sid2: Option<Box<Sid>>,
    pub sid2_base: u16,
    /// Datasette.
    pub tape: crate::tape::Datasette,
    pub cia1: CiaState,
    pub cia2: CiaState,

    // Keyboard
    pub keyboard: KeyMatrix,

    // Joystick: bit 0-4 = UP,DOWN,LEFT,RIGHT,FIRE (active low: 0=pressed, default 0xFF)
    pub joy1: u8,  // Port 1 → CIA1 PB bit 0-4
    pub joy2: u8,  // Port 2 → CIA1 PA bit 0-4
    /// Paddles and 1351 mouse in the control ports, SID POT inputs.
    pub ctrl: crate::ctrlport::ControlPorts,

    // PC of the current instruction (updated by c64.rs before cpu.step)
    pub cpu_pc: u16,

    /// Clock cycles since power-on (cartridge flash timings).
    pub cycle: u64,

    // Debugger watchpoints. Empty = no cost on accesses.
    pub dbg_watch: Vec<Watch>,
    pub dbg_watch_hit: Option<WatchHit>,
}

impl Bus {
    pub fn new() -> Self {
        let mut b = Self {
            ram: *power_on::ram(),
            kernal_rom: [0u8; 0x2000],
            basic_rom: [0u8; 0x2000],
            char_rom: [0u8; 0x1000],
            color_ram: power_on::COLOR_RAM,
            cart: None,
            reu: None,
            eth: None,
            drive: None,
            skip_write: false,
            cpu_port_dir: 0x2F,
            cpu_port_data: 0x37,
            cpu_port_out: 0,
            port_charge: [0; 2],
            port_charge_until: [0; 2],
            vic: VicState::new(),
            sid: Sid::new(),
            sid2: None,
            sid2_base: 0xD420,
            tape: crate::tape::Datasette::new(),
            cia1: CiaState::new(),
            cia2: CiaState::new(),
            keyboard: KeyMatrix::new(),
            joy1: 0xFF,
            joy2: 0xFF,
            ctrl: crate::ctrlport::ControlPorts::new(),
            cpu_pc: 0,
            cycle: 0,
            dbg_watch: Vec::new(),
            dbg_watch_hit: None,
        };
        // CIA2 port A default: bits 0-1 = 11 → VIC bank 0
        b.cia2.regs[0] = 0xFF;
        b.cia2.regs[2] = 0x3F; // PA bits 0-1 output
        b
    }

    pub fn load_kernal(&mut self, data: &[u8]) {
        let n = data.len().min(0x2000);
        self.kernal_rom[..n].copy_from_slice(&data[..n]);
    }
    pub fn load_basic(&mut self, data: &[u8]) {
        let n = data.len().min(0x2000);
        self.basic_rom[..n].copy_from_slice(&data[..n]);
    }
    pub fn load_char(&mut self, data: &[u8]) {
        let n = data.len().min(0x1000);
        self.char_rom[..n].copy_from_slice(&data[..n]);
    }
    pub fn load_ram(&mut self, addr: u16, data: &[u8]) {
        let s = addr as usize;
        let e = (s + data.len()).min(0x10000);
        self.ram[s..e].copy_from_slice(&data[..e - s]);
    }

    /// Read of $01, as VICE (c64pla_config_changed, zero_read): output bits
    /// return the register; input bits 0-2 and 4 are pulled up (bit 4 low
    /// with a Datasette key pressed), bit 3 reads the last value it had as
    /// an output, bit 5 (motor) reads 0, bits 6-7 the charge they keep.
    pub fn cpu_port_read(&self) -> u8 {
        let dir = self.cpu_port_dir;
        let mut v = (self.cpu_port_data | !dir) & (self.cpu_port_out | 0x17);
        if dir & 0x20 == 0 {
            v &= !0x20;
        }
        if self.tape.sense() && dir & 0x10 == 0 {
            v &= !0x10;
        }
        for (i, bit) in [(0, 0x40u8), (1, 0x80)] {
            if dir & bit == 0 {
                v &= !bit;
                if self.cycle <= self.port_charge_until[i] {
                    v |= self.port_charge[i];
                }
            }
        }
        v
    }

    /// Write to $00 (direction) or $01 (data). A bit 6/7 written while it
    /// is an output, or turned from output into input, keeps that value as
    /// a charge for `PORT_FALL_OFF` cycles.
    fn write_cpu_port(&mut self, addr: u16, val: u8) {
        let deadline = self.cycle + PORT_FALL_OFF;
        for (i, bit) in [(0, 0x40u8), (1, 0x80)] {
            let charge = match addr {
                0 => (self.cpu_port_dir & bit != 0 && val & bit == 0).then_some(self.cpu_port_data & bit),
                _ => (self.cpu_port_dir & bit != 0).then_some(val & bit),
            };
            if let Some(c) = charge {
                self.port_charge[i] = c;
                self.port_charge_until[i] = deadline;
            }
        }
        if addr == 0 {
            self.cpu_port_dir = val;
        } else {
            self.cpu_port_data = val;
        }
        let dir = self.cpu_port_dir;
        self.cpu_port_out = (self.cpu_port_out & !dir) | (self.cpu_port_data & dir);
        self.update_tape_lines();
    }

    /// 6510 reset: the port direction and data registers are cleared, so all
    /// the pins are inputs and the pull-ups make KERNAL, BASIC and I/O
    /// visible (VICE: c64pla_pport_reset).
    pub fn reset_cpu_port(&mut self) {
        self.cpu_port_dir = 0;
        self.cpu_port_data = 0;
        self.cpu_port_out = 0;
        self.port_charge = [0; 2];
        self.port_charge_until = [0; 2];
        self.update_tape_lines();
    }

    /// Tape MOTOR and WRITE lines after a write to $00/$01. The motor
    /// (bit 5) runs when the line is low, even with the bit set as input;
    /// WRITE (bit 3) as input is high (as in VICE).
    fn update_tape_lines(&mut self) {
        let on = self.cpu_port_dir & self.cpu_port_data & 0x20 == 0;
        self.tape.set_motor_line(on, self.cycle);
        let write = (!self.cpu_port_dir | self.cpu_port_data) & 0x08 != 0;
        self.tape.set_write_line(write, self.cycle);
    }

    /// Effective bits of the 6510 I/O port (output bits from the register, input = 1 via pull-up).
    pub fn effective_port(&self) -> u8 {
        (self.cpu_port_data & self.cpu_port_dir) | (!self.cpu_port_dir)
    }

    pub fn loram(&self)  -> bool { self.effective_port() & 0x01 != 0 }
    pub fn hiram(&self)  -> bool { self.effective_port() & 0x02 != 0 }
    pub fn charen(&self) -> bool { self.effective_port() & 0x04 != 0 }

    pub fn io_active(&self) -> bool { self.charen() && (self.hiram() || self.loram()) }

    // ── Watchpoint ────────────────────────────────────────────────────────────

    fn watch_check(&mut self, addr: u16, val: u8, write: bool) {
        for w in &self.dbg_watch {
            if addr >= w.start && addr <= w.end && (if write { w.on_write } else { w.on_read }) {
                // Keep the first hit of the current instruction
                if self.dbg_watch_hit.is_none() {
                    self.dbg_watch_hit = Some(WatchHit { addr, val, write, pc: self.cpu_pc });
                }
                return;
            }
        }
    }

    // ── CPU accesses ──────────────────────────────────────────────────────────

    /// CPU read (with side effects on I/O registers, e.g. CIA ICR).
    pub fn read(&mut self, addr: u16) -> u8 {
        let open = self.open_bus();
        let v = match self.map_read(addr) {
            Mapped::Byte(b) => b,
            Mapped::Cia1(reg) => self.cia1.read_mut(reg),
            Mapped::Cia2(reg) => self.cia2.read_mut(reg),
            Mapped::Vic(reg) => self.vic.read_mut(reg),
            // The POT pins of the first SID are wired to the control ports
            Mapped::Sid(reg @ (0x19 | 0x1A)) => self.ctrl.read(reg, self.cycle),
            Mapped::Sid(reg) => self.sid.read_mut(reg),
            Mapped::Sid2(reg) => self.sid2.as_mut().map_or(open, |s| s.read_mut(reg)),
            Mapped::Eth(addr) => self.eth.as_mut().map_or(open, |e| e.read(addr)),
            Mapped::Roml(off) => {
                let now = self.cycle;
                match self.cart.as_mut() {
                    // Action Replay in its broken mode: C64 RAM and
                    // cartridge RAM together on the bus
                    Some(c) if c.ram_contention() => c.read_roml(off, now) | self.ram[0x8000 + off as usize],
                    Some(c) => c.read_roml(off, now),
                    None => open,
                }
            }
            Mapped::Romh(off) => {
                let now = self.cycle;
                self.cart.as_mut().map_or(open, |c| c.read_romh(off, now))
            }
            Mapped::Io1(off) => self.cart.as_mut().and_then(|c| c.read_io1(off, open)).unwrap_or(open),
            Mapped::Io2(off) => match self.reu.as_mut() {
                Some(reu) => reu.read(off),
                None => self.cart.as_mut().and_then(|c| c.read_io2(off)).unwrap_or(open),
            },
        };
        if !self.dbg_watch.is_empty() { self.watch_check(addr, v, false); }
        v
    }

    /// Side-effect-free read: used by the debugger (dump, disassembly).
    /// Returns the same byte the CPU would see, without touching the state.
    pub fn peek(&self, addr: u16) -> u8 {
        let open = self.open_bus();
        match self.map_read(addr) {
            Mapped::Byte(b) => b,
            Mapped::Cia1(reg) => self.cia1.read(reg),
            Mapped::Cia2(reg) => self.cia2.read(reg),
            Mapped::Vic(reg) => self.vic.read(reg),
            Mapped::Sid(reg @ (0x19 | 0x1A)) => self.ctrl.peek(reg),
            Mapped::Sid(reg) => self.sid.read(reg),
            Mapped::Sid2(reg) => self.sid2.as_ref().map_or(open, |s| s.read(reg)),
            Mapped::Eth(addr) => self.eth.as_ref().map_or(open, |e| e.peek(addr)),
            Mapped::Roml(off) => match self.cart.as_ref() {
                Some(c) if c.ram_contention() => c.peek_roml(off) | self.ram[0x8000 + off as usize],
                Some(c) => c.peek_roml(off),
                None => open,
            },
            Mapped::Romh(off) => self.cart.as_ref().map_or(open, |c| c.peek_romh(off)),
            Mapped::Io1(off) => self.cart.as_ref().and_then(|c| c.peek_io1(off, open)).unwrap_or(open),
            Mapped::Io2(off) => match self.reu.as_ref() {
                Some(reu) => reu.peek(off),
                None => self.cart.as_ref().and_then(|c| c.peek_io2(off)).unwrap_or(open),
            },
        }
    }

    /// True if the cartridge forces Ultimax mode.
    pub fn ultimax(&self) -> bool {
        self.cart_mode() == Mode::Ultimax
    }

    /// NMI line pulled low by the cartridge (freeze button).
    #[inline]
    pub fn cart_nmi(&self) -> bool {
        self.cart.as_ref().is_some_and(|c| c.nmi)
    }

    /// Cartridge mode (/EXROM and /GAME lines); `Off` without a cartridge.
    pub fn cart_mode(&self) -> Mode {
        self.cart.as_ref().map_or(Mode::Off, |c| c.mode)
    }

    /// Byte on the open bus, seen by the CPU when it reads an address no
    /// chip answers (Ultimax holes, empty I/O1/I/O2, the top nibble of the
    /// color RAM): the one the VIC read in the phi1 half of this cycle, as
    /// in VICE (`vicii_read_phi1`).
    #[inline]
    pub fn open_bus(&self) -> u8 {
        self.vic.last_read_phi1
    }

    /// Resolves banking for a CPU read, like the PLA from the
    /// LORAM/HIRAM/CHAREN lines (port $01) and /EXROM//GAME (cartridge):
    ///
    /// - $8000-$9FFF: ROML in 8K and 16K with LORAM and HIRAM high;
    /// - $A000-$BFFF: ROMH in 16K with HIRAM high, otherwise BASIC with LORAM
    ///   and HIRAM high;
    /// - $D000-$DFFF: I/O with CHAREN and LORAM or HIRAM high; without CHAREN
    ///   the CHAR ROM, except in 16K with HIRAM low (there it is RAM);
    /// - $E000-$FFFF: KERNAL with HIRAM high.
    ///
    /// In Ultimax RAM is only at $0000-$0FFF, ROML and ROMH are fixed and the
    /// rest is not connected (open bus). Registers with read side effects
    /// (CIA, VIC collisions, SID, cartridge) are returned as a reference,
    /// not read.
    fn map_read(&self, addr: u16) -> Mapped {
        let mode = self.cart_mode();
        if mode == Mode::Ultimax {
            let byte = match addr {
                0x0000 => self.cpu_port_dir,
                0x0001 => self.cpu_port_read(),
                0x0002..=0x0FFF => self.ram[addr as usize],
                0x8000..=0x9FFF => return Mapped::Roml(addr - 0x8000),
                0xD000..=0xDFFF => return self.map_io(addr),
                0xE000..=0xFFFF => return Mapped::Romh(addr - 0xE000),
                _ => self.open_bus(),
            };
            return Mapped::Byte(byte);
        }
        let byte = match addr {
            0x0000 => self.cpu_port_dir,
            0x0001 => self.cpu_port_read(),

            0x0002..=0x7FFF | 0xC000..=0xCFFF => self.ram[addr as usize],

            0x8000..=0x9FFF => {
                if matches!(mode, Mode::Game8K | Mode::Game16K) && self.loram() && self.hiram() {
                    return Mapped::Roml(addr - 0x8000);
                }
                self.ram[addr as usize]
            }

            0xA000..=0xBFFF => {
                if mode == Mode::Game16K && self.hiram() {
                    return Mapped::Romh(addr - 0xA000);
                } else if self.loram() && self.hiram() {
                    self.basic_rom[(addr - 0xA000) as usize]
                } else {
                    self.ram[addr as usize]
                }
            }

            0xD000..=0xDFFF => {
                if self.io_active() {
                    return self.map_io(addr);
                } else if !self.charen() && (self.hiram() || (self.loram() && mode != Mode::Game16K)) {
                    self.char_rom[(addr - 0xD000) as usize]
                } else {
                    self.ram[addr as usize]
                }
            }

            0xE000..=0xFFFF => {
                if self.hiram() {
                    self.kernal_rom[(addr - 0xE000) as usize]
                } else {
                    self.ram[addr as usize]
                }
            }
        };
        Mapped::Byte(byte)
    }

    /// Serial bus lines pulled low by the C64: (ATN, CLK, DATA). The CIA2
    /// PA3-5 outputs go through the inverting 7406: a bit at 1 pulls the
    /// line low, and an input pin reads 1 (pull-up), so it pulls it low too.
    pub fn iec_c64_lines(&self) -> (bool, bool, bool) {
        let pa = self.cia2.regs[0] | !self.cia2.regs[2];
        (pa & 0x08 != 0, pa & 0x10 != 0, pa & 0x20 != 0)
    }

    /// CIA2 Port A ($DD00): bit 0-1 VIC bank, 3-5 ATN/CLK/DATA OUT, 6-7 CLK/DATA IN.
    /// CLK and DATA are low if the C64 or the drive pulls them (open collector);
    /// the inputs read the line level. Bits configured as output read back
    /// the register.
    pub fn cia2_port_a(&self) -> u8 {
        let ddr = self.cia2.regs[2];
        let out = self.cia2.regs[0];
        let (atn, mut clk_low, mut data_low) = self.iec_c64_lines();
        if let Some(d) = &self.drive {
            let (clk, data) = d.pulls(atn);
            clk_low |= clk;
            data_low |= data;
        }
        let lines = 0x3F | if clk_low { 0 } else { 0x40 } | if data_low { 0 } else { 0x80 };
        (out & ddr) | (lines & !ddr)
    }

    fn map_io(&self, addr: u16) -> Mapped {
        Mapped::Byte(match addr {
            0xD000..=0xD3FF => {
                return Mapped::Vic((addr & 0x3F) as u8);
            }
            0xD400..=0xD7FF if self.is_sid2(addr) => return Mapped::Sid2((addr & 0x1F) as u8),
            0xD400..=0xD7FF => return Mapped::Sid((addr & 0x1F) as u8),
            0xD800..=0xDBFF => self.color_ram[(addr - 0xD800) as usize] & 0x0F | self.open_bus() & 0xF0,
            0xDC00..=0xDCFF => {
                let reg = (addr & 0x0F) as u8;
                match reg {
                    0x00 => self.cia1_port_a(),
                    0x01 => self.cia1_port_b(),
                    _ => return Mapped::Cia1(reg),
                }
            }
            0xDD00..=0xDDFF => {
                let reg = (addr & 0x0F) as u8;
                if reg == 0 { self.cia2_port_a() } else { return Mapped::Cia2(reg) }
            }
            0xDE00..=0xDFFF if self.is_sid2(addr) => return Mapped::Sid2((addr & 0x1F) as u8),
            0xDE00..=0xDFFF if self.is_eth(addr) => return Mapped::Eth(addr),
            // Cartridge I/O areas (open bus if it does not respond)
            0xDE00..=0xDEFF => return Mapped::Io1(addr as u8),
            0xDF00..=0xDFFF => return Mapped::Io2(addr as u8),
            _ => self.open_bus(),
        })
    }

    /// Joystick lines of control ports 1 (PB) and 2 (PA), active low:
    /// joystick, and paddle or mouse buttons.
    fn joy_lines(&self) -> (u8, u8) {
        (self.joy1 & self.ctrl.lines(0, self.cycle), self.joy2 & self.ctrl.lines(1, self.cycle))
    }

    /// CIA1 Port A ($DC00), as VICE's read_ciapa: pins of the port pulled
    /// low by joystick 2 and by the keyboard. A PB line held low (an output
    /// at 0, or joystick 1) pulls low the PA lines of its keys, which lets
    /// programs scan the keyboard "backwards" (PB selects, PA reads); if
    /// the keys also reach a PB output driven high, only the direct keys
    /// count (no ghost keys). A PA line held low pulls the PA lines it is
    /// joined to. Output bits read back the register, ANDed with the pins.
    pub fn cia1_port_a(&self) -> u8 {
        let (pra, ddra) = (self.cia1.regs[0], self.cia1.regs[2]);
        let (prb, ddrb) = (self.cia1.regs[1], self.cia1.regs[3]);
        let (joy1, joy2) = self.joy_lines();
        let kb = &self.keyboard;
        let mut val = 0xFF;
        if kb.any_pressed() {
            let low_pb = !((prb | !ddrb) & joy1);
            for j in (0..8).filter(|j| low_pb & (1 << j) != 0) {
                let (rows, cols) = kb.connected(0, 1 << j);
                val &= if cols & prb & ddrb != 0 { !kb.pa_of_pb(j) } else { !rows };
            }
            let low_pa = !((pra | !ddra) & joy2);
            for i in (0..8).filter(|i| low_pa & (1 << i) != 0) {
                val &= !kb.connected(1 << i, 0).0;
            }
        }
        val & (pra | !ddra) & joy2
    }

    /// PA6/PA7 of CIA1 (output value, or high as inputs) drive the 4066
    /// that connects the SID POT pins to control port 1 and/or 2.
    pub fn pot_mask_changed(&mut self) {
        let pa = self.cia1.regs[0] | !self.cia1.regs[2];
        self.ctrl.set_mask(pa >> 6, self.cycle);
    }

    /// CIA1 Port B ($DC01), as VICE's read_ciapb: a PA line held low (an
    /// output at 0 selecting a keyboard column, or joystick 2) pulls low the
    /// PB lines joined to it through the keys, ghost keys included; a PB
    /// line held low (joystick 1) pulls the PB lines joined to it. Outputs
    /// read back the register: at 0 they stay low, at 1 they are pulled low
    /// only by two or more PA outputs at 0 on the same line, or by
    /// joystick 1.
    pub fn cia1_port_b(&self) -> u8 {
        let (pra, ddra) = (self.cia1.regs[0], self.cia1.regs[2]);
        let (prb, ddrb) = (self.cia1.regs[1], self.cia1.regs[3]);
        let (joy1, joy2) = self.joy_lines();
        let eff_pa = pra | !ddra;
        let kb = &self.keyboard;
        let mut val = 0xFF;
        let mut val_outhi = ddrb & prb;
        if kb.any_pressed() {
            let low_pa = !(eff_pa & joy2);
            let pa_out_low = ddra & !pra;
            for i in (0..8).filter(|i| low_pa & (1 << i) != 0) {
                let (rows, cols) = kb.connected(1 << i, 0);
                val &= !cols;
                if pa_out_low & (1 << i) != 0 && ddrb & prb & cols != 0 {
                    let v = rows & pa_out_low;
                    if v & v.wrapping_sub(1) != 0 {
                        val_outhi &= !cols;
                    }
                }
            }
            let low_pb = !((prb | !ddrb) & joy1);
            for j in (0..8).filter(|j| low_pb & (1 << j) != 0) {
                val &= !kb.connected(0, 1 << j).1;
            }
        }
        let mut pb = ((val & (prb | !ddrb)) | val_outhi) & joy1;
        // When no column is selected (eff_pa=0xFF, polling joy1 fire),
        // mirror joy2 FIRE onto joy1 bit 4 as well.
        // This lets Space/LCtrl/LAlt (joy2 fire) be detected also by
        // games/demos that check $DC01 bit 4 with $DC00=0xFF.
        // It causes no phantom keypress since it only applies when PA=0xFF.
        if eff_pa == 0xFF && (self.joy2 & 0x10 == 0) {
            pb &= !0x10;
        }
        // Timer outputs on PB6/PB7 (PBON)
        let (m, v) = self.cia1.pb_timer_bits();
        (pb & !m) | v
    }

    /// CPU write.
    #[inline]
    pub fn write(&mut self, addr: u16, val: u8) {
        if !self.dbg_watch.is_empty() { self.watch_check(addr, val, true); }
        if self.skip_write {
            self.skip_write = false;
            return;
        }
        self.write_mem(addr, val);
        // A write to $FF00 (even in RAM or under the ROM) starts the
        // pending REU command
        if addr == 0xFF00 && self.reu.is_some() {
            self.ff00_written();
        }
    }

    #[cold]
    fn ff00_written(&mut self) {
        if let Some(reu) = self.reu.as_mut() {
            reu.ff00_written();
        }
    }

    /// First write (with the value read) of a read-modify-write
    /// instruction: if it starts the REU, the second one is skipped.
    #[inline]
    pub fn write_dummy(&mut self, addr: u16, val: u8) {
        if !self.dbg_watch.is_empty() { self.watch_check(addr, val, true); }
        self.write_mem(addr, val);
        if addr == 0xFF00 {
            if let Some(reu) = self.reu.as_mut() {
                self.skip_write = reu.ff00_written();
            }
        }
    }

    /// DMA read (REU): like the CPU, but $00/$01 are RAM.
    pub fn dma_read(&mut self, addr: u16) -> u8 {
        if addr < 2 { self.ram[addr as usize] } else { self.read(addr) }
    }

    /// DMA write (REU): like the CPU, but $00/$01 are RAM and $FF00 does not
    /// start commands.
    pub fn dma_write(&mut self, addr: u16, val: u8) {
        if addr < 2 { self.ram[addr as usize] = val } else { self.write_mem(addr, val) }
    }

    #[inline(always)]
    fn write_mem(&mut self, addr: u16, val: u8) {
        let now = self.cycle;
        if addr < 2 {
            // The processor port answers, but the RAM under it is written
            // too, with the byte left on the bus by the VIC (VICE: zero_store)
            self.ram[addr as usize] = self.open_bus();
        }
        if self.ultimax() {
            // In Ultimax RAM above $0FFF is not selected even for
            // writes; ROML and ROMH receive the write (flash)
            match addr {
                0x0000 | 0x0001 => self.write_cpu_port(addr, val),
                0x0002..=0x0FFF => self.ram[addr as usize] = val,
                0x8000..=0x9FFF => self.cart.as_mut().unwrap().write_roml(addr - 0x8000, val, now),
                0xD000..=0xDFFF => self.write_io(addr, val),
                0xE000..=0xFFFF => self.cart.as_mut().unwrap().write_romh(addr - 0xE000, val, now),
                _ => {}
            }
            return;
        }
        match addr {
            0x0000 | 0x0001 => self.write_cpu_port(addr, val),

            0x0002..=0x7FFF => self.ram[addr as usize] = val,
            0x8000..=0x9FFF => {
                // With ROML visible the cartridge sees the write too (the
                // Action Replay RAM); the C64 RAM is written anyway
                let roml = self.loram() && self.hiram();
                if let Some(c) = self.cart.as_mut().filter(|c| roml && matches!(c.mode, Mode::Game8K | Mode::Game16K)) {
                    c.write_roml_game(addr - 0x8000, val);
                }
                self.ram[addr as usize] = val;
            }
            0xA000..=0xBFFF => self.ram[addr as usize] = val,
            0xC000..=0xCFFF => self.ram[addr as usize] = val,

            0xD000..=0xDFFF => {
                if self.io_active() {
                    self.write_io(addr, val);
                } else {
                    self.ram[addr as usize] = val;
                }
            }

            0xE000..=0xFFFF => match self.cart.as_mut().filter(|c| c.ultimax_writes()) {
                // GMod2 in programming mode: the write goes to the flash, not to RAM
                Some(c) => c.write_romh(addr - 0xE000, val, now),
                None => self.ram[addr as usize] = val,
            },
        }
    }

    fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            0xD000..=0xD3FF => self.vic.write((addr & 0x3F) as u8, val),
            0xD400..=0xD7FF | 0xDE00..=0xDFFF if self.is_sid2(addr) => {
                if let Some(s) = self.sid2.as_mut() {
                    s.write((addr & 0x1F) as u8, val);
                }
            }
            0xD400..=0xD7FF => self.sid.write((addr & 0x1F) as u8, val),
            0xD800..=0xDBFF => self.color_ram[(addr - 0xD800) as usize] = val & 0x0F,
            0xDC00..=0xDCFF => {
                let reg = (addr & 0x0F) as u8;
                self.cia1.write(reg, val);
                if reg == 0 || reg == 2 {
                    self.pot_mask_changed();
                }
            }
            0xDD00..=0xDDFF => self.cia2.write((addr & 0x0F) as u8, val),
            0xDE00..=0xDFFF if self.is_eth(addr) => {
                if let Some(eth) = self.eth.as_mut() {
                    eth.write(addr, val);
                }
            }
            0xDE00..=0xDEFF => {
                if let Some(cart) = self.cart.as_mut() {
                    cart.write_io1(addr as u8, val);
                }
            }
            // I/O2: seen by both the REU and the cartridge
            0xDF00..=0xDFFF => {
                if let Some(reu) = self.reu.as_mut() {
                    reu.write(addr as u8, val);
                }
                if let Some(cart) = self.cart.as_mut() {
                    cart.write_io2(addr as u8, val);
                }
            }
            _ => {}
        }
    }

    /// The address falls within the 32 bytes of the second SID.
    #[inline]
    fn is_sid2(&self, addr: u16) -> bool {
        self.sid2.is_some() && addr & 0xFFE0 == self.sid2_base
    }

    /// The address falls within the 16 bytes of the Ethernet cartridge.
    #[inline]
    fn is_eth(&self, addr: u16) -> bool {
        self.eth.as_ref().is_some_and(|e| e.contains(addr))
    }

    /// Direct RAM read (used by the VIC, which bypasses CPU banking).
    pub fn read_ram(&self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }

    /// Current VIC bank (0-3) from CIA2 PA bits 0-1 (DDR-aware).
    pub fn vic_bank(&self) -> u16 {
        let ddr = self.cia2.regs[2];
        let pa  = (self.cia2.regs[0] & ddr) | (!ddr);
        (!(pa) & 0x03) as u16
    }

    pub fn vic_read(&self, vic_addr: u16) -> u8 {
        // VIC bank: bits 0-1 of CIA2 PA, taking DDR (regs[2]) into account.
        // Output bits (DDR=1): use the value from regs[0]. Input bits (DDR=0): pull-up → 1.
        let bank_num = self.vic_bank();
        let phys     = bank_num * 0x4000 + vic_addr;
        // Ultimax: at offsets $3000-$3FFF of every bank the VIC sees the
        // last 4K of ROMH ($F000-$FFFF), elsewhere RAM (no character ROM)
        if let Some(c) = self.cart.as_ref().filter(|c| c.vic_ultimax()) {
            return if vic_addr & 0x3000 == 0x3000 {
                c.peek_romh(0x1000 | (vic_addr & 0x0FFF))
            } else {
                self.ram[phys as usize & 0xFFFF]
            };
        }
        // Char ROM visible only in banks 0 ($0000) and 2 ($8000) at VIC offset $1000–$1FFF.
        // In banks 1 and 3 that range is ordinary RAM.
        if (bank_num == 0 || bank_num == 2) && (0x1000..=0x1FFF).contains(&vic_addr) {
            self.char_rom[(vic_addr - 0x1000) as usize]
        } else {
            self.ram[phys as usize & 0xFFFF]
        }
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

// ROMs are not part of the state: those of the machine it is reloaded on
// are kept. Debugger watchpoints excluded.
crate::snapshot::impl_state!(Bus {
    ram, color_ram, cart, reu, eth, drive, skip_write, cpu_port_dir, cpu_port_data,
    cpu_port_out, port_charge, port_charge_until, vic, sid, sid2, sid2_base, tape, cia1, cia2,
    keyboard, joy1, joy2, ctrl, cpu_pc, cycle,
} skip { kernal_rom, basic_rom, char_rom, dbg_watch, dbg_watch_hit });

#[cfg(test)]
mod tests {
    use super::*;

    /// Bus with I/O visible and the keys `keys` (PA line, PB line) down.
    fn bus(keys: &[(usize, usize)]) -> Bus {
        let mut b = Bus::new();
        for &(pa, pb) in keys {
            b.keyboard.press(pa, pb);
        }
        b
    }

    fn cia1(b: &mut Bus, ddra: u8, pra: u8, ddrb: u8, prb: u8) {
        b.write(0xDC02, ddra);
        b.write(0xDC00, pra);
        b.write(0xDC03, ddrb);
        b.write(0xDC01, prb);
    }

    #[test]
    fn keyboard_scan_by_columns() {
        let mut b = bus(&[(1, 2)]);                 // A
        cia1(&mut b, 0xFF, 0xFD, 0x00, 0xFF);       // column 1 selected
        assert_eq!(b.read(0xDC01), 0xFB);
        cia1(&mut b, 0xFF, 0xFE, 0x00, 0xFF);
        assert_eq!(b.read(0xDC01), 0xFF);
    }

    #[test]
    fn keyboard_scan_backwards() {
        let mut b = bus(&[(1, 2)]);
        cia1(&mut b, 0x00, 0xFF, 0xFF, 0xFB);       // PB2 low, PA inputs
        assert_eq!(b.read(0xDC00), 0xFD, "A seen on PA1");
        cia1(&mut b, 0x00, 0xFF, 0xFF, 0xF7);
        assert_eq!(b.read(0xDC00), 0xFF);
    }

    #[test]
    fn ghost_key() {
        // Keys on PA1/PB2, PA1/PB4, PA3/PB2: selecting PA3 also reaches PB4
        let mut b = bus(&[(1, 2), (1, 4), (3, 2)]);
        cia1(&mut b, 0xFF, 0xF7, 0x00, 0xFF);
        assert_eq!(b.read(0xDC01), 0xEB);
    }

    #[test]
    fn backwards_ghost_removed_by_high_outputs() {
        let keys = [(1, 2), (1, 4), (5, 4)];
        // PB2 the only output: the ghost PA5 (via PB4) is seen
        let mut b = bus(&keys);
        cia1(&mut b, 0x00, 0xFF, 0x04, 0x00);
        assert_eq!(b.read(0xDC00), 0xDD);
        // Other PB outputs driven high: only the direct keys of PB2
        cia1(&mut b, 0x00, 0xFF, 0xFF, 0xFB);
        assert_eq!(b.read(0xDC00), 0xFD);
    }

    #[test]
    fn joystick_1_through_the_matrix() {
        // LEFT (PB2) reaches PB4 through the keys PA1/PB2 and PA1/PB4
        let mut b = bus(&[(1, 2), (1, 4)]);
        b.joy1 = !0x04;
        cia1(&mut b, 0xFF, 0xFF, 0x00, 0xFF);
        assert_eq!(b.read(0xDC01), 0xEB);
    }
}
