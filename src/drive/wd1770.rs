// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Derived from VICE 3.10 (src/drive/iec/wd1770.c, src/drive/iec/fdd.c),
// Copyright (C) Kajtar Zsolt.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! WD1770 floppy controller of the 1571 at $2000-$2003 (mirrored up to
//! $2FFF): command/status, track, sector and data registers, and the
//! "microcode" of its commands (type I: restore, seek, step; II: read
//! and write sector; III: read address, read and write track; IV: force
//! interrupt), run lazily up to the drive cycle of each access, as VICE
//! does.
//!
//! In the 1571 the controller reads MFM disks (CP/M, IBM formats). Here, as
//! in VICE with GCR images, its floppy has no MFM disk and its motor line is
//! never raised: no index pulses, no data, write protect and track 0 active.
//! The DOS only sees the registers and the status of commands that wait for
//! a disk. Unlike VICE, the step pulses of the controller do not move the
//! 1571 head: its stepper motor is driven by VIA2.

use crate::snapshot::impl_state;

// Registers (3: data)
const WD_STATUS: u8 = 0;
const WD_TRACK: u8 = 1;
const WD_SECTOR: u8 = 2;

// Command bits
const WD_A: u8 = 0x01;
const WD_R: u8 = 0x03;
const WD_V: u8 = 0x04;
const WD_E: u8 = 0x04;
const WD_H: u8 = 0x08;
const WD_U: u8 = 0x10;
const WD_M: u8 = 0x10;
const WD_I2: u8 = 0x04;
const WD_I3: u8 = 0x08;

// Status bits
const WD_MO: u8 = 0x80;
const WD_WP: u8 = 0x40;
const WD_SU: u8 = 0x20;
const WD_RT: u8 = 0x20;
const WD_SE: u8 = 0x10;
const WD_RNF: u8 = 0x10;
const WD_CRC: u8 = 0x08;
const WD_T0: u8 = 0x04;
const WD_LD: u8 = 0x04;
const WD_IP: u8 = 0x02;
const WD_DRQ: u8 = 0x02;
const WD_BSY: u8 = 0x01;

// Commands
const WD_RESTORE: u8 = 0x00;
const WD_SEEK: u8 = 0x10;
const WD_STEP: u8 = 0x20;
const WD_STEP_IN: u8 = 0x40;
const WD_STEP_OUT: u8 = 0x60;
const WD_READ_SECTOR: u8 = 0x80;
const WD_WRITE_SECTOR: u8 = 0xA0;
const WD_READ_ADDRESS: u8 = 0xC0;
const WD_FORCE_INTERRUPT: u8 = 0xD0;
const WD_READ_TRACK: u8 = 0xE0;
const WD_WRITE_TRACK: u8 = 0xF0;

/// Commands: mask, code, type (the first match wins, as in VICE).
const COMMANDS: [(u8, u8, i8); 11] = [
    (0xF0, WD_RESTORE, 1),
    (0xF0, WD_SEEK, 1),
    (0xE0, WD_STEP, 1),
    (0xE0, WD_STEP_IN, 1),
    (0xE0, WD_STEP_OUT, 1),
    (0xE0, WD_READ_SECTOR, 2),
    (0xE0, WD_WRITE_SECTOR, 2),
    (0xF0, WD_READ_ADDRESS, 3),
    (0xF0, WD_READ_TRACK, 3),
    (0xF0, WD_FORCE_INTERRUPT, 4),
    (0xF0, WD_WRITE_TRACK, 3),
];

/// Step rates of the WD1770 (in 1 MHz cycles), by the command's r1 r0 bits.
const STEP_RATE: [u64; 4] = [6000, 12000, 20000, 30000];
/// Timings at the 2 MHz VICE gives the controller in every drive.
const CLOCK_FREQUENCY: u64 = 2;
const SETTLING: u64 = CLOCK_FREQUENCY * 30000;
const BYTE_RATE: u64 = CLOCK_FREQUENCY * 8000 / 250;
const PREPARE: u64 = CLOCK_FREQUENCY * 24;

/// Raw bytes that pass under the head before the index hole ends.
const INDEX_LEN: u32 = 16;
/// Highest track of the mechanism.
const FDD_MAX_TRACK: i32 = 80 + 3;

/// CRC-CCITT step (polynomial $1021), as the controller computes it.
fn crc(crc: u16, b: u8) -> u16 {
    let mut w = ((crc >> 8) as u8 ^ b) as u16;
    w <<= 8;
    for _ in 0..8 {
        w = if w & 0x8000 != 0 { (w << 1) ^ 0x1021 } else { w << 1 };
    }
    w ^ (crc << 8)
}

/// The floppy under the controller: head position and index count. Without
/// an MFM disk nothing turns: the head stays at the index hole.
#[derive(Clone, Default)]
struct Fdd {
    /// Physical head position (0 = track 0).
    track: i32,
    /// Index pulses counted since the last reset of the count.
    index_count: u32,
    /// Position of the head in the raw track, in bytes.
    head: u32,
    /// Motor line from the drive (never raised in the 1571).
    motor: bool,
}

impl Fdd {
    /// Advances by `bytes` raw bytes: nothing moves without a disk.
    fn rotate(&mut self, bytes: u64) -> u64 {
        bytes
    }

    fn index(&self) -> bool {
        self.head < INDEX_LEN
    }

    fn track0(&self) -> bool {
        self.track == 0
    }

    fn write_protect(&self) -> bool {
        true
    }

    /// Next raw byte (bit 8: a sync mark): nothing without a disk and motor.
    fn read(&mut self) -> u16 {
        0
    }

    fn write(&mut self, _data: u16) {}

    fn seek_pulse(&mut self, inward: bool) {
        if self.motor {
            self.track += if inward { 1 } else { -1 };
        }
        self.track = self.track.clamp(0, FDD_MAX_TRACK);
    }
}

/// How the microcode goes on after a step.
enum Flow {
    /// Wait for the CPU to get further.
    Wait,
    /// The command is over: the interrupt request rises.
    Done,
}

/// As VICE leaves it in the 1571, where it is never reset: all zero.
#[derive(Default)]
pub struct Wd1770 {
    data: u8,
    track: u8,
    sector: u8,
    status: u8,
    cmd: u8,
    crc: u16,
    /// Command decoded from `cmd` and its type: -1 = type I status, 0 = idle.
    command: u8,
    kind: i8,
    step: i32,
    byte_count: i32,
    tmp: u32,
    /// Step direction: towards the centre (higher tracks).
    direction: bool,
    /// Drive cycle up to which the controller has run.
    clk: u64,
    irq: bool,
    dden: bool,
    sync: bool,
    fdd: Fdd,
}

impl Wd1770 {
    /// Advances the head over the bytes passed up to cycle `now`.
    fn spin(&mut self, now: u64) {
        let bytes = now.saturating_sub(self.clk) / BYTE_RATE;
        self.clk += self.fdd.rotate(bytes) * BYTE_RATE;
    }

    /// The microcode up to drive cycle `now`.
    fn execute(&mut self, now: u64) {
        loop {
            let flow = match self.kind {
                -1 | 0 => {
                    if self.kind == -1 {
                        self.status &= !(WD_WP | WD_IP | WD_T0);
                        self.status |= if self.fdd.index() { WD_IP } else { 0 };
                        self.status |= if self.fdd.track0() { WD_T0 } else { 0 };
                        self.status |= if self.fdd.write_protect() { WD_WP } else { 0 };
                    }
                    if now < self.clk + PREPARE {
                        return;
                    }
                    self.status &= !WD_BSY;
                    self.spin(now);
                    if self.fdd.index_count >= 10 {
                        self.status &= !WD_MO;
                    }
                    if self.cmd & WD_I2 != 0 && self.fdd.index_count != self.tmp {
                        self.irq = true;
                        self.tmp = self.fdd.index_count;
                    }
                    return;
                }
                1 => self.type1(now),
                2 => self.type2(now),
                3 => self.type3(now),
                4 => {
                    if now < self.clk + PREPARE {
                        return;
                    }
                    self.clk += PREPARE;
                    self.status &= WD_BSY;
                    if self.cmd & WD_I3 != 0 {
                        self.irq = true;
                    }
                    self.fdd.index_count = 0;
                    self.tmp = self.fdd.index_count;
                    self.kind = if self.status & WD_BSY != 0 { 0 } else { -1 };
                    continue;
                }
                _ => return,
            };
            match flow {
                Flow::Wait => return,
                Flow::Done => {
                    self.cmd = 0;
                    self.irq = true;
                    self.fdd.index_count = 0;
                }
            }
        }
    }

    /// Spin-up (steps 1-2 of types I-III): six index pulses with the motor
    /// on, unless the command skips it or the motor is already running.
    fn spin_up(&mut self, now: u64) -> bool {
        if self.step == 1 {
            if self.cmd & WD_H != 0 || self.status & WD_MO != 0 {
                self.status |= WD_MO;
                self.step += 2;
                return true;
            }
            self.status |= WD_MO;
            self.fdd.index_count = 0;
            self.step += 1;
        }
        self.spin(now);
        if self.fdd.index_count < 6 {
            return false;
        }
        self.step += 1;
        true
    }

    /// Looks for an ID address mark: false while more bytes are needed.
    fn find_id_mark(&mut self) -> bool {
        let res = self.fdd.read();
        if (!self.dden || res != 0x1FE) && (!self.sync || res != 0xFE) {
            self.sync = res == 0x1A1;
            return false;
        }
        true
    }

    fn type1(&mut self, now: u64) -> Flow {
        loop {
            match self.step {
                0 => {
                    if now < self.clk + PREPARE {
                        return Flow::Wait;
                    }
                    self.clk += PREPARE;
                    self.status |= WD_BSY;
                    self.status &= !(WD_CRC | WD_SE | WD_DRQ);
                    self.irq = false;
                    self.step += 1;
                }
                1 | 2 => {
                    if !self.spin_up(now) {
                        return Flow::Wait;
                    }
                }
                3 => {
                    match self.command {
                        WD_STEP => {}
                        WD_STEP_IN => self.direction = true,
                        WD_STEP_OUT => self.direction = false,
                        c => {
                            if c == WD_RESTORE {
                                self.track = 0xFF;
                                self.data = 0x00;
                            }
                            self.step += 1;
                            continue;
                        }
                    }
                    self.step = if self.cmd & WD_U != 0 { 5 } else { 6 };
                }
                4 => {
                    if self.data == self.track {
                        self.step = 8;
                        continue;
                    }
                    self.direction = self.data > self.track;
                    self.step += 1;
                }
                5 => {
                    self.track = if self.direction { self.track.wrapping_add(1) } else { self.track.wrapping_sub(1) };
                    self.step += 1;
                }
                6 => {
                    if self.fdd.track0() && !self.direction {
                        self.track = 0;
                        self.step = 8;
                        continue;
                    }
                    self.fdd.seek_pulse(self.direction);
                    self.step += 1;
                }
                7 => {
                    let rate = CLOCK_FREQUENCY * STEP_RATE[(self.cmd & WD_R) as usize];
                    if now < self.clk + rate {
                        return Flow::Wait;
                    }
                    self.clk += rate;
                    if self.cmd < WD_STEP {
                        self.step = 4;
                        continue;
                    }
                    self.step += 1;
                }
                8 => {
                    if self.cmd & WD_V == 0 {
                        self.kind = -1;
                        return Flow::Done;
                    }
                    self.step += 1;
                }
                9 => {
                    if now < self.clk + SETTLING {
                        return Flow::Wait;
                    }
                    self.clk += SETTLING;
                    self.fdd.index_count = 0;
                    self.sync = false;
                    self.step += 1;
                }
                10 => {
                    if self.fdd.index_count >= 6 {
                        self.status |= WD_SE;
                        self.kind = -1;
                        return Flow::Done;
                    }
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    if !self.find_id_mark() {
                        continue;
                    }
                    self.sync = false;
                    self.crc = 0xB230;
                    self.byte_count = 6;
                    self.step += 1;
                }
                11 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    let res = self.fdd.read();
                    if self.byte_count == 6 && res != self.track as u16 {
                        self.step -= 1;
                        continue;
                    }
                    self.crc = crc(self.crc, res as u8);
                    self.byte_count -= 1;
                    if self.byte_count != 0 {
                        continue;
                    }
                    if self.crc != 0 {
                        self.status |= WD_CRC;
                        self.step -= 1;
                        continue;
                    }
                    self.status &= !WD_CRC;
                    self.kind = -1;
                    return Flow::Done;
                }
                _ => return Flow::Wait,
            }
        }
    }

    fn type2(&mut self, now: u64) -> Flow {
        loop {
            match self.step {
                0 => {
                    if now < self.clk + PREPARE {
                        return Flow::Wait;
                    }
                    self.clk += PREPARE;
                    self.status |= WD_BSY;
                    self.status &= !(WD_DRQ | WD_LD | WD_RNF | WD_RT | WD_WP);
                    self.step += 1;
                }
                1 | 2 => {
                    if !self.spin_up(now) {
                        return Flow::Wait;
                    }
                }
                3 => {
                    self.step += if self.cmd & WD_E == 0 { 2 } else { 1 };
                }
                4 => {
                    if now < self.clk + SETTLING {
                        return Flow::Wait;
                    }
                    self.clk += SETTLING;
                    self.step += 1;
                }
                5 => {
                    if self.command == WD_WRITE_SECTOR && self.fdd.write_protect() {
                        self.status |= WD_WP;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    self.fdd.index_count = 0;
                    self.sync = false;
                    self.step += 1;
                }
                6 => {
                    if self.fdd.index_count >= 5 {
                        self.status |= WD_RNF;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    if !self.find_id_mark() {
                        continue;
                    }
                    self.sync = false;
                    self.crc = 0xB230;
                    self.byte_count = 6;
                    self.step += 1;
                }
                7 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    let res = self.fdd.read();
                    if self.byte_count == 6 && res != self.track as u16 {
                        self.step -= 1;
                        continue;
                    }
                    if self.byte_count == 4 && res != self.sector as u16 {
                        self.step -= 1;
                        continue;
                    }
                    if self.byte_count == 3 {
                        self.tmp = res as u32;
                    }
                    self.crc = crc(self.crc, res as u8);
                    self.byte_count -= 1;
                    if self.byte_count != 0 {
                        continue;
                    }
                    if self.crc != 0 {
                        self.status |= WD_CRC;
                        self.step -= 1;
                        continue;
                    }
                    self.status &= !WD_CRC;
                    self.crc = 0xFFFF;
                    if self.command == WD_WRITE_SECTOR {
                        self.byte_count = 0;
                        self.step = 10;
                        continue;
                    }
                    self.byte_count = 43;
                    self.step += 1;
                }
                8 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    let left = self.byte_count;
                    self.byte_count -= 1;
                    if left == 0 {
                        self.step -= 2;
                        continue;
                    }
                    self.clk += BYTE_RATE;
                    let res = self.fdd.read();
                    if (!self.dden || (res != 0x1FB && res != 0x1F8)) && (!self.sync || (res != 0xFB && res != 0xF8)) {
                        if !self.sync {
                            self.crc = 0xFFFF;
                        }
                        self.crc = crc(self.crc, res as u8);
                        self.sync = res == 0x1A1;
                        continue;
                    }
                    self.crc = crc(self.crc, res as u8);
                    self.status |= if res & 0xFF == 0xF8 { WD_RT } else { 0 };
                    self.byte_count = (128 << self.tmp.min(24)) + 2;
                    self.step += 1;
                }
                9 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    let res = self.fdd.read();
                    if self.byte_count > 2 {
                        self.status |= if self.status & WD_DRQ != 0 { WD_LD } else { WD_DRQ };
                        self.data = res as u8;
                    }
                    self.crc = crc(self.crc, res as u8);
                    self.byte_count -= 1;
                    if self.byte_count != 0 {
                        continue;
                    }
                    if self.crc != 0 {
                        self.status |= WD_CRC;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if self.cmd & WD_M != 0 {
                        self.sector = self.sector.wrapping_add(1);
                        self.step = 5;
                        continue;
                    }
                    self.kind = 0;
                    return Flow::Done;
                }
                10 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    self.byte_count += 1;
                    self.status |= if self.byte_count == 2 { WD_DRQ } else { 0 };
                    if self.byte_count == 2 + 9 && self.status & WD_DRQ != 0 {
                        self.status ^= WD_DRQ | WD_LD;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if self.byte_count <= if self.dden { 0 } else { 11 } + 2 + 9 {
                        self.fdd.read();
                        continue;
                    }
                    if self.byte_count <= if self.dden { 6 } else { 11 + 12 } + 2 + 9 {
                        self.fdd.write(0);
                        continue;
                    }
                    if !self.dden && self.byte_count <= 11 + 12 + 2 + 9 + 3 {
                        self.fdd.write(0x1A1);
                        self.crc = crc(self.crc, 0xA1);
                        continue;
                    }
                    let res = if self.cmd & WD_A != 0 { 0xF8 } else { 0xFB } | if self.dden { 0x100 } else { 0 };
                    self.fdd.write(res);
                    self.crc = crc(self.crc, res as u8);
                    self.byte_count = (128 << self.tmp.min(24)) + 3;
                    self.step += 1;
                }
                11 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    self.byte_count -= 1;
                    match self.byte_count {
                        0 => self.fdd.write(0xFF),
                        1 => {
                            self.fdd.write(self.crc & 0xFF);
                            continue;
                        }
                        2 => {
                            self.fdd.write(self.crc >> 8);
                            continue;
                        }
                        _ => {
                            self.status |= if self.status & WD_DRQ != 0 { WD_LD } else { WD_DRQ };
                            self.crc = crc(self.crc, self.data);
                            self.fdd.write(self.data as u16);
                            self.data = 0;
                            continue;
                        }
                    }
                    if self.cmd & WD_M != 0 {
                        self.sector = self.sector.wrapping_add(1);
                        self.step = 5;
                        continue;
                    }
                    self.kind = 0;
                    return Flow::Done;
                }
                _ => return Flow::Wait,
            }
        }
    }

    fn type3(&mut self, now: u64) -> Flow {
        loop {
            match self.step {
                0 => {
                    if now < self.clk + PREPARE {
                        return Flow::Wait;
                    }
                    self.clk += PREPARE;
                    self.status |= WD_BSY;
                    self.status &= !(WD_DRQ | WD_LD | WD_RNF | WD_CRC);
                    self.step += 1;
                }
                1 | 2 => {
                    if !self.spin_up(now) {
                        return Flow::Wait;
                    }
                }
                3 => {
                    self.step += if self.cmd & WD_E == 0 { 2 } else { 1 };
                }
                4 => {
                    if now < self.clk + SETTLING {
                        return Flow::Wait;
                    }
                    self.clk += SETTLING;
                    self.step += 1;
                }
                5 => {
                    self.fdd.index_count = 0;
                    self.sync = false;
                    self.step += 1;
                    if self.command == WD_WRITE_TRACK {
                        if self.fdd.write_protect() {
                            self.status |= WD_WP;
                            self.kind = 0;
                            return Flow::Done;
                        }
                        self.status |= WD_DRQ;
                        self.byte_count = 3;
                        self.step = 9;
                        continue;
                    }
                    if self.command != WD_READ_TRACK {
                        self.step += 1;
                    }
                }
                6 => {
                    if self.fdd.index_count < 1 {
                        self.spin(now);
                        return Flow::Wait;
                    }
                    if self.fdd.index_count > 1 {
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    self.data = self.fdd.read() as u8;
                    self.status |= if self.status & WD_DRQ != 0 { WD_LD } else { WD_DRQ };
                }
                7 => {
                    if self.fdd.index_count >= 6 {
                        self.status |= WD_RNF;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    if !self.find_id_mark() {
                        continue;
                    }
                    self.crc = 0xB230;
                    self.byte_count = 6;
                    self.step += 1;
                }
                8 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.status |= if self.status & WD_DRQ != 0 { WD_LD } else { WD_DRQ };
                    self.clk += BYTE_RATE;
                    self.data = self.fdd.read() as u8;
                    if self.byte_count == 6 {
                        self.sector = self.data;
                    }
                    self.crc = crc(self.crc, self.data);
                    self.byte_count -= 1;
                    if self.byte_count != 0 {
                        continue;
                    }
                    self.status |= if self.crc != 0 { WD_CRC } else { 0 };
                    self.kind = 0;
                    return Flow::Done;
                }
                9 => {
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    self.fdd.read();
                    self.byte_count -= 1;
                    if self.byte_count != 0 {
                        continue;
                    }
                    if self.status & WD_DRQ != 0 {
                        self.status ^= WD_DRQ | WD_LD;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    self.byte_count = 0;
                    self.tmp = 0;
                    self.step += 1;
                }
                10 => {
                    if self.fdd.index_count < 1 {
                        self.spin(now);
                        return Flow::Wait;
                    }
                    if self.fdd.index_count > 1 {
                        self.status &= !WD_DRQ;
                        self.kind = 0;
                        return Flow::Done;
                    }
                    if now < self.clk + BYTE_RATE {
                        return Flow::Wait;
                    }
                    self.clk += BYTE_RATE;
                    let mut res = self.data as u16;
                    if self.byte_count != 0 {
                        self.fdd.write(self.crc & 0xFF);
                        self.byte_count -= 1;
                    } else {
                        self.status |= if self.status & WD_DRQ != 0 { WD_LD } else { WD_DRQ };
                        if self.dden {
                            match res {
                                0xF7 => {
                                    self.byte_count = 1;
                                    res = self.crc >> 8;
                                    self.tmp = 0;
                                }
                                0xF8..=0xFB | 0xFE | 0xFC => {
                                    if res != 0xFC && self.tmp == 0 {
                                        self.crc = 0xFFFF;
                                        self.tmp = 1;
                                    }
                                    res |= 0x100;
                                }
                                _ => {}
                            }
                        } else {
                            match res {
                                0xF5 => {
                                    res = 0x1A1;
                                    if self.tmp == 0 {
                                        self.crc = 0xFFFF;
                                        self.tmp = 1;
                                    }
                                }
                                0xF6 => res = 0x1C2,
                                0xF7 => {
                                    self.byte_count = 1;
                                    res = self.crc >> 8;
                                    self.tmp = 0;
                                }
                                _ => {}
                            }
                        }
                        if self.tmp != 0 {
                            self.crc = crc(self.crc, res as u8);
                        }
                        self.fdd.write(res);
                        self.data = 0;
                    }
                }
                _ => return Flow::Wait,
            }
        }
    }

    /// CPU write to register `reg` (0-3) at drive cycle `now`.
    pub fn write(&mut self, reg: u8, v: u8, now: u64) {
        self.execute(now);
        match reg & 3 {
            WD_STATUS => {
                self.cmd = v;
                let (_, command, kind) = COMMANDS.iter().copied()
                    .find(|&(mask, command, _)| v & mask == command)
                    .unwrap_or(COMMANDS[COMMANDS.len() - 1]);
                self.command = command;
                self.kind = kind;
                self.spin(now);
                self.step = 0;
                self.execute(now);
            }
            WD_TRACK => self.track = v,
            WD_SECTOR => self.sector = v,
            _ => {
                self.status &= !WD_DRQ;
                self.data = v;
            }
        }
    }

    /// CPU read of register `reg` (0-3) at drive cycle `now`.
    pub fn read(&mut self, reg: u8, now: u64) -> u8 {
        self.execute(now);
        match reg & 3 {
            WD_STATUS => {
                self.irq = false;
                self.status
            }
            WD_TRACK => self.track,
            WD_SECTOR => self.sector,
            _ => {
                self.status &= !WD_DRQ;
                self.data
            }
        }
    }

    /// Read without side effects (debugger).
    pub fn peek(&self, reg: u8) -> u8 {
        match reg & 3 {
            WD_STATUS => self.status,
            WD_TRACK => self.track,
            WD_SECTOR => self.sector,
            _ => self.data,
        }
    }
}

// The spin-up status bit shares its value with record type (type II/III).
const _: () = assert!(WD_SU == WD_RT);

impl_state!(Fdd { track, index_count, head, motor });

impl_state!(Wd1770 {
    data, track, sector, status, cmd, crc, command, kind, step, byte_count, tmp, direction, clk, irq,
    dden, sync, fdd,
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_an_id_mark() {
        // Three $A1 sync marks and $FE give the preset VICE starts from
        let c = [0xA1, 0xA1, 0xA1, 0xFE].iter().fold(0xFFFF, |c, &b| crc(c, b));
        assert_eq!(c, 0xB230);
    }

    #[test]
    fn commands_without_a_disk() {
        let mut wd = Wd1770::default();
        // Restore: no spin-up (h), at track 0 at once; busy until the
        // controller goes idle
        wd.write(0, 0x08, 100);
        assert_eq!(wd.read(0, 150) & (WD_BSY | WD_MO | WD_T0), WD_BSY | WD_MO | WD_T0);
        assert_eq!(wd.read(0, 1000) & (WD_BSY | WD_MO), WD_MO);
        assert_eq!(wd.read(1, 1000), 0);
        // A read sector waits forever for the index pulses of the spin-up
        let mut wd = Wd1770::default();
        wd.write(0, 0x80, 1000);
        assert_eq!(wd.read(0, 1_000_000) & (WD_BSY | WD_MO), WD_BSY | WD_MO);
        // Force interrupt ends it
        wd.write(0, 0xD0, 1_000_100);
        assert_eq!(wd.read(0, 1_002_000) & WD_BSY, 0);
    }
}
