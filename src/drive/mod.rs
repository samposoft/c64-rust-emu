// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 SampoSoft - Francesco Sampoli
// Disk rotation derived from VICE 3.10 (src/drive/rotation.c),
// Copyright (C) Andreas Boose, Istvan Fabian, Benjamin Rosseaux,
// Peter Rittwage.
// Ported to Rust and modified by SampoSoft in 2026; see CREDITS.md.

//! 1541 drive emulated at the hardware level: 6502 at 1 MHz, 2 KB of RAM,
//! 16 KB DOS ROM, two 6522 VIAs, mechanics and flux-level disk reading.
//!
//! Drive memory: RAM $0000-$07FF, VIA1 (serial bus) $1800-$1BFF,
//! VIA2 (disk) $1C00-$1FFF, ROM $8000-$FFFF (16 KB mirrored). The rest is
//! open bus: the last byte that went over the bus is read back.
//!
//! VIA1 port B: 0 DATA IN, 1 DATA OUT, 2 CLK IN, 3 CLK OUT, 4 ATN ACK,
//! 5-6 drive number, 7 ATN IN; CA1 = ATN (rising edge when the C64
//! asserts it). The outputs go through open-collector inverters: 1 pulls the
//! line low; the inputs read 1 when the line is low. The ATN ACK circuit
//! pulls DATA low when ATN is asserted and the drive has not acknowledged it
//! (ATN ACK different from the ATN state).
//!
//! VIA2 port B: 0-1 phases of the head stepper motor, 2 motor,
//! 3 LED, 4 write protect (0 = protected), 5-6 density (speed
//! zone), 7 SYNC (0 = sync); port A: the byte read or to be written; CA2 =
//! BYTE READY enable towards the CPU SO pin (V flag), CB2 =
//! read (1) or write (0).
//!
//! Disk reading: VICE's circuit simulation (rotation.c), rewritten from
//! scratch, with the same "lazy" organization: the disk advances only when
//! the drive CPU looks at its state (VIA2 accesses, instructions that use
//! the V flag), up to the current cycle. A 16 MHz reference clock (16 cycles
//! per drive cycle) advances the disk: at 300 rpm one revolution lasts 3.2
//! million reference cycles, during which all the bits of the track pass.
//! A 1 bit is a flux reversal. UE7 divides the clock by 16, 15, 14 or
//! 13 depending on the density and clocks UF4; both restart at every
//! reversal. The bit that enters the shift register, when bit 1 of UF4
//! rises, is 1 only right after a reversal: so the density chosen by the
//! software decides how the bits are read. After 18 µs without flux,
//! random reversals arrive (weak bits). Ten 1s in a row are the SYNC,
//! which resets the bit count; otherwise every 8 bits there is BYTE READY,
//! which reaches the CPU with a delay aligned to its clock (VICE). When
//! writing, the same edges shift the register bits out to the
//! track.

pub mod gcr;
pub mod via;

use crate::cpu::{Cpu, CpuBus};
use crate::snapshot::{impl_state, Reader, Result as StateResult, State, Writer};
use gcr::{Disk, HALF_TRACKS};
use via::Via;

/// Reference cycles (16 MHz) in one revolution at 300 rpm.
const REF_PER_REVOLUTION: u32 = 3_200_000;
/// VIA2 register reads sample the disk at 14/16 of the cycle.
const BUS_READ_DELAY: u32 = 14;
/// Drive cycles in one revolution.
const CYCLES_PER_REVOLUTION: u64 = 200_000;
/// Disk change (VICE): after insertion the disk is unreadable and write
/// protect is active for 1.8 s; on ejection write protect is active for
/// 0.6 s; a disk inserted right after an ejection leaves write protect
/// inactive for 1.2 s. The DOS detects the change from the sensor.
const ATTACH_DELAY: u64 = 1_800_000;
const DETACH_DELAY: u64 = 600_000;
const ATTACH_DETACH_DELAY: u64 = 1_200_000;
/// The drive's clock: 1 MHz (the C64's is set by `Drive::set_c64_clock`).
const DRIVE_HZ: u32 = 1_000_000;

/// Mechanics and read/write circuit.
pub struct Mechanics {
    pub disk: Option<Disk>,
    /// Half-track under the head (2 = track 1).
    pub half_track: usize,
    /// Head position in the track, in bits.
    head_bit: usize,
    pub motor: bool,
    pub led: bool,
    /// Density chosen by the software (0-3): UE7 restarts from this value.
    zone: u8,
    /// true = read (VIA2 CB2 high).
    read_mode: bool,
    /// BYTE READY enabled towards the CPU (VIA2 CA2 high).
    soe: bool,
    // Circuit (names from the 1541 schematic)
    accum: u32,
    shift: u16,
    write_shift: u8,
    bit_counter: u8,
    ue7: u8,
    uf4: u8,
    fr_randcount: u32,
    filter_counter: u32,
    filter_state: bool,
    filter_last: bool,
    so_delay: u32,
    cycle_index: u32,
    xorshift: u32,
    /// Last complete byte read (VIA2 port A).
    pub gcr_read: u8,
    /// Byte to write (port A as output).
    gcr_write: u8,
    /// BYTE READY edge not yet seen by the CPU.
    so_edge: bool,
    /// V must be set right away (BYTE READY disabled or motor turned off with
    /// an edge pending).
    force_v: bool,
    /// Drive cycle up to which the disk has been simulated.
    last_clk: u64,
    /// Reference cycles simulated ahead (mid-cycle reads).
    ref_advance: u32,
    /// Lead requested by the current read.
    req_ref: u32,
    /// Insertion/ejection times still in progress (see the delays).
    attach_clk: Option<u64>,
    detach_clk: Option<u64>,
    attach_detach_clk: Option<u64>,
}

impl Default for Mechanics {
    fn default() -> Self {
        Self {
            disk: None,
            half_track: 36, // track 18 at power-on, as in VICE
            head_bit: 0,
            motor: false,
            led: false,
            zone: 0,
            read_mode: true,
            soe: false,
            accum: 0,
            shift: 0,
            write_shift: 0,
            bit_counter: 0,
            ue7: 0,
            uf4: 0,
            fr_randcount: 0,
            filter_counter: 0,
            filter_state: false,
            filter_last: false,
            so_delay: 0,
            cycle_index: 0,
            xorshift: 0x1234_ABCD,
            gcr_read: 0,
            gcr_write: 0,
            so_edge: false,
            force_v: false,
            last_clk: 0,
            ref_advance: 0,
            req_ref: 0,
            attach_clk: None,
            detach_clk: None,
            attach_detach_clk: None,
        }
    }
}

impl Mechanics {
    fn track(&self) -> Option<&gcr::Track> {
        self.disk.as_ref()?.tracks.get(self.half_track)?.as_ref()
    }

    /// Bits per revolution of the track under the head (0 without a track: on
    /// empty half-tracks the head reads nothing).
    fn track_bits(&self) -> u32 {
        self.track().map_or(0, |t| t.bits() as u32)
    }

    fn random(&mut self) -> u32 {
        self.xorshift ^= self.xorshift << 13;
        self.xorshift ^= self.xorshift >> 17;
        self.xorshift ^= self.xorshift << 5;
        self.xorshift
    }

    fn read_next_bit(&mut self) -> bool {
        let bits = self.track_bits() as usize;
        if bits == 0 {
            return false;
        }
        let pos = self.head_bit;
        self.head_bit = (pos + 1) % bits;
        match self.track() {
            Some(t) => (t.data[pos >> 3] >> (7 - (pos & 7))) & 1 != 0,
            None => false,
        }
    }

    fn write_next_bit(&mut self, value: bool) {
        let bits = self.track_bits() as usize;
        if bits == 0 {
            return;
        }
        let pos = self.head_bit;
        self.head_bit = (pos + 1) % bits;
        let ht = self.half_track;
        if let Some(disk) = self.disk.as_mut() {
            if let Some(Some(t)) = disk.tracks.get_mut(ht) {
                let mask = 0x80 >> (pos & 7);
                if value {
                    t.data[pos >> 3] |= mask;
                } else {
                    t.data[pos >> 3] &= !mask;
                }
                disk.dirty = true;
            }
        }
    }

    fn schedule_byte_ready(&mut self, todo: u32) {
        if self.soe {
            self.so_delay = 16 - ((self.cycle_index + todo - 1) & 15);
            if self.so_delay < 10 {
                self.so_delay += 16;
            }
        }
    }

    /// Advances by `ref_cycles` reference cycles.
    fn rotate(&mut self, mut ref_cycles: u32) {
        if !self.motor {
            return;
        }
        // Without a track the counter advances by 1: no bits in a revolution
        let bits = self.track_bits().max(1);
        while ref_cycles > 0 {
            // How many cycles can be done in one go without events
            let mut todo = 1;
            let delta = REF_PER_REVOLUTION as i64 - self.accum as i64;
            if delta > 0 && 2 * bits as i64 <= delta {
                todo = ((delta / bits as i64) as u32).min(ref_cycles);
                if self.ue7 < 16 {
                    todo = todo.min(16 - self.ue7 as u32);
                }
                if self.read_mode {
                    if self.filter_counter < 40 {
                        todo = todo.min(40 - self.filter_counter);
                    }
                    if self.fr_randcount > 0 {
                        todo = todo.min(self.fr_randcount);
                    }
                }
                if self.so_delay > 0 {
                    todo = todo.min(self.so_delay);
                }
            }
            if self.so_delay > 0 {
                self.so_delay -= todo;
                if self.so_delay == 0 {
                    self.so_edge = true;
                }
            }
            if self.read_mode {
                // 2.5 µs filter on flux reversals
                self.filter_counter += todo;
                if self.filter_counter >= 40 && self.filter_last != self.filter_state {
                    self.filter_last = self.filter_state;
                    self.ue7 = self.zone;
                    self.uf4 = 0;
                    self.fr_randcount = (self.random() >> 16) % 31 + 289;
                } else {
                    // After 18 µs without flux: random reversals
                    self.fr_randcount = self.fr_randcount.wrapping_sub(todo);
                    if self.fr_randcount == 0 {
                        self.ue7 = self.zone;
                        self.uf4 = 0;
                        self.fr_randcount = (self.random() >> 16) % 367 + 33;
                    }
                }
                self.ue7 += todo as u8;
                if self.ue7 == 16 {
                    self.ue7 = self.zone;
                    self.uf4 = (self.uf4 + 1) & 15;
                    if self.uf4 & 3 == 2 {
                        let bit = ((self.uf4 + 0x1C) >> 4) & 1;
                        self.shift = ((self.shift << 1) & 0x3FE) | bit as u16;
                        self.write_shift <<= 1;
                        if self.shift == 0x3FF {
                            // SYNC: the bit count restarts
                            self.bit_counter = 0;
                        } else {
                            self.bit_counter += 1;
                            if self.bit_counter == 8 {
                                self.bit_counter = 0;
                                self.gcr_read = self.shift as u8;
                                self.write_shift = self.gcr_read;
                                self.schedule_byte_ready(todo);
                            }
                        }
                    }
                }
                self.accum += bits * todo;
                if self.accum >= REF_PER_REVOLUTION {
                    self.accum -= REF_PER_REVOLUTION;
                    if self.read_next_bit() {
                        // GCR images are clean: the filter is already settled
                        self.filter_counter = 39;
                        self.filter_state = !self.filter_state;
                    }
                }
            } else {
                self.accum += bits * todo;
                if self.accum >= REF_PER_REVOLUTION {
                    self.accum -= REF_PER_REVOLUTION;
                }
                self.ue7 += todo as u8;
                if self.ue7 == 16 {
                    self.ue7 = self.zone;
                    self.uf4 = (self.uf4 + 1) & 15;
                    if self.uf4 & 3 == 2 {
                        let bit = ((self.uf4 + 0x1C) >> 4) & 1;
                        self.shift = ((self.shift << 1) & 0x3FE) | bit as u16;
                        let out = self.write_shift & 0x80 != 0;
                        self.write_next_bit(out);
                        self.write_shift <<= 1;
                        self.accum = bits * 2;
                        self.bit_counter += 1;
                        if self.bit_counter == 8 {
                            self.bit_counter = 0;
                            self.write_shift = self.gcr_write;
                            self.schedule_byte_ready(todo);
                        }
                    }
                }
            }
            self.cycle_index = self.cycle_index.wrapping_add(todo);
            ref_cycles -= todo;
        }
    }

    /// Brings the disk to drive cycle `clk` (plus the lead `req_ref`
    /// of the current read, in reference cycles).
    fn rotate_to(&mut self, clk: u64) {
        let adv = self.req_ref & 15;
        self.req_ref = 0;
        if !self.motor {
            return;
        }
        let mut cycles = clk.saturating_sub(self.last_clk);
        self.last_clk = clk;
        // After long intervals without accesses whole revolutions are skipped
        while cycles > 2 * CYCLES_PER_REVOLUTION {
            cycles -= CYCLES_PER_REVOLUTION;
        }
        let refs = cycles as u32 * 16 + adv;
        if refs > 0 {
            if refs > self.ref_advance {
                let todo = refs - self.ref_advance;
                self.ref_advance = adv;
                self.rotate(todo);
            } else {
                self.ref_advance -= refs;
            }
        }
    }

    /// Port A read (the byte from the disk) at cycle `clk`.
    fn read_byte(&mut self, clk: u64) {
        if let Some(t) = self.attach_clk {
            if clk - t < ATTACH_DELAY {
                self.gcr_read = 0;
                self.req_ref = 0;
                return;
            }
            self.attach_clk = None;
        } else if let Some(t) = self.attach_detach_clk {
            if clk - t < ATTACH_DETACH_DELAY {
                self.gcr_read = 0;
                self.req_ref = 0;
                return;
            }
            self.attach_detach_clk = None;
        }
        self.rotate_to(clk);
    }

    /// SYNC (VIA2 port B bit 7: 0 during a sync).
    fn sync_bit(&self) -> u8 {
        if self.read_mode && self.attach_clk.is_none() && self.shift == 0x3FF { 0 } else { 0x80 }
    }

    /// Write protect sensor (port B bit 4: 0 = protected); during a
    /// disk change it follows VICE's delays.
    fn write_protect_bit(&mut self, clk: u64) -> u8 {
        if let Some(t) = self.detach_clk {
            if clk - t < DETACH_DELAY {
                return 0;
            }
            self.detach_clk = None;
        }
        if let Some(t) = self.attach_detach_clk {
            if clk - t < ATTACH_DETACH_DELAY {
                return 0x10;
            }
            self.attach_detach_clk = None;
        }
        if let Some(t) = self.attach_clk {
            if clk - t < ATTACH_DELAY {
                return 0;
            }
            self.attach_clk = None;
        }
        match &self.disk {
            Some(d) if d.write_protected => 0,
            _ => 0x10,
        }
    }

    fn peek_write_protect_bit(&self) -> u8 {
        match &self.disk {
            Some(d) if d.write_protected => 0,
            _ => 0x10,
        }
    }

    /// New half-track: the head stays at the same fraction of the revolution
    /// (from the start, if it came from an empty half-track).
    fn set_half_track(&mut self, ht: usize) {
        let ht = ht.clamp(2, 84);
        let old_bits = self.track_bits() as usize;
        self.half_track = ht;
        let new_bits = self.track_bits() as usize;
        self.head_bit = if old_bits == 0 { 0 } else { self.head_bit * new_bits / old_bits };
        if self.head_bit >= new_bits.max(1) {
            self.head_bit = 0;
        }
    }

    /// VIA2 port B write at cycle `clk`: head, motor,
    /// LED, density.
    fn port_b_written(&mut self, pb: u8, clk: u64) {
        self.rotate_to(clk);
        self.led = pb & 0x08 != 0;
        self.zone = (pb >> 5) & 3;
        let motor = pb & 0x04 != 0;
        if motor {
            // The coils move only with the motor on: one step at a time
            let old = (self.half_track - 2) & 3;
            let step = ((pb & 3) as usize).wrapping_sub(old) & 3;
            match step {
                1 => self.set_half_track(self.half_track + 1),
                3 => self.set_half_track(self.half_track - 1),
                _ => {}
            }
        }
        if motor && !self.motor {
            // The disk restarts from here
            self.last_clk = clk;
            self.cycle_index = 0;
        } else if !motor && self.motor && self.so_edge {
            self.so_edge = false;
            self.force_v = true;
        }
        self.motor = motor;
    }

    /// VIA2 PCR at cycle `clk`: CA2 enables BYTE READY, CB2 selects
    /// read/write.
    fn control_written(&mut self, via2: &Via, clk: u64) {
        let soe = via2.ca2_out();
        if soe != self.soe {
            self.rotate_to(clk);
            self.soe = soe;
            if self.so_edge {
                self.so_edge = false;
                self.force_v = true;
            }
        }
        let read = via2.cb2_out();
        if read != self.read_mode {
            self.rotate_to(clk);
            self.read_mode = read;
        }
    }
}

/// Drive memory and peripherals, as seen by its CPU.
pub struct DriveBus {
    pub ram: [u8; 0x800],
    rom: Vec<u8>,
    pub via1: Via,
    pub via2: Via,
    pub mech: Mechanics,
    /// Last byte that went over the bus (reads of unmapped areas).
    last_data: u8,
    /// Lines driven by the C64 (true = asserted, i.e. low).
    c64_atn: bool,
    c64_clk: bool,
    c64_data: bool,
    /// Drive number minus 8 (jumpers, VIA1 bits 5-6).
    device: u8,
    /// Current drive cycle.
    clk: u64,
    /// Activity counter: bytes read from or written to the disk and serial bus
    /// line changes. Frontends use it to detect a load in
    /// progress (turbo); it is not part of the state.
    pub activity: u64,
}

impl DriveBus {
    /// Lines pulled low by the drive: (CLK, DATA). The ATN ACK circuit is
    /// combinational: it follows the ATN state immediately (`atn` = asserted).
    pub fn pulls(&self, atn: bool) -> (bool, bool) {
        let pb = self.via1.pb_out();
        let clk = pb & 0x08 != 0;
        let atn_ack = pb & 0x10 != 0;
        let data = pb & 0x02 != 0 || atn_ack != atn;
        (clk, data)
    }

    /// VIA1 port B input pins.
    fn iec_pins(&self) -> u8 {
        let (drive_clk, drive_data) = self.pulls(self.c64_atn);
        let clk_low = self.c64_clk || drive_clk;
        let data_low = self.c64_data || drive_data;
        (data_low as u8) | 0x02 | (clk_low as u8) << 2 | 0x18 | (self.device & 3) << 5
            | (self.c64_atn as u8) << 7
    }

    fn io_read(&mut self, addr: u16) -> u8 {
        let reg = (addr & 0x0F) as u8;
        if addr & 0x1C00 == 0x1800 {
            let pins = self.iec_pins();
            return self.via1.read(reg, 0xFF, pins);
        }
        // The disk data is sampled towards the end of the cycle
        let clk = self.clk;
        match reg {
            0x1 | 0xF => {
                self.mech.req_ref = BUS_READ_DELAY;
                self.mech.read_byte(clk);
                self.activity += 1;
            }
            0x0 => {
                self.mech.req_ref = BUS_READ_DELAY;
                self.mech.rotate_to(clk);
            }
            _ => {}
        }
        let wp = if reg == 0 { self.mech.write_protect_bit(clk) } else { self.mech.peek_write_protect_bit() };
        let pb = self.mech.sync_bit() | wp | 0x6F;
        self.via2.read(reg, self.mech.gcr_read, pb)
    }

    /// Read without side effects (debugger).
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize],
            0x1800..=0x1BFF => self.via1.peek((addr & 0xF) as u8, 0xFF, self.iec_pins()),
            0x1C00..=0x1FFF => {
                let pb = self.mech.sync_bit() | self.mech.peek_write_protect_bit() | 0x6F;
                self.via2.peek((addr & 0xF) as u8, self.mech.gcr_read, pb)
            }
            0x8000..=0xFFFF => self.rom[addr as usize & 0x3FFF],
            _ => self.last_data,
        }
    }
}

impl CpuBus for DriveBus {
    fn read(&mut self, addr: u16) -> u8 {
        let v = match addr {
            0x0000..=0x07FF => self.ram[addr as usize],
            0x1800..=0x1FFF => self.io_read(addr),
            0x8000..=0xFFFF => self.rom[addr as usize & 0x3FFF],
            _ => self.last_data,
        };
        self.last_data = v;
        v
    }

    fn write(&mut self, addr: u16, v: u8) {
        self.last_data = v;
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize] = v,
            0x1800..=0x1BFF => {
                let pulls = self.via1.pb_out() & 0x1A;
                self.via1.write((addr & 0xF) as u8, v);
                if self.via1.pb_out() & 0x1A != pulls {
                    self.activity += 1;
                }
            }
            0x1C00..=0x1FFF => {
                let reg = (addr & 0xF) as u8;
                // VIA outputs change at the end of the cycle: motor, head and
                // mode act on the disk from the next cycle (as in VICE)
                let clk = self.clk + 1;
                self.via2.write(reg, v);
                match reg {
                    0x0 | 0x2 => self.mech.port_b_written(self.via2.pb_out(), clk),
                    0x1 | 0x3 | 0xF => {
                        self.mech.rotate_to(clk);
                        self.mech.gcr_write = self.via2.pa_out();
                        self.activity += 1;
                    }
                    0xC => self.mech.control_written(&self.via2, clk),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn so_edge(&mut self, ahead: u8) -> bool {
        let clk = self.clk + ahead as u64;
        self.mech.rotate_to(clk);
        std::mem::take(&mut self.mech.so_edge)
    }

    fn so_discard(&mut self, ahead: u8) {
        let clk = self.clk + ahead as u64;
        self.mech.rotate_to(clk);
        self.mech.so_edge = false;
    }
}

pub struct Drive {
    pub cpu: Cpu,
    pub bus: DriveBus,
    /// C64 time not yet covered by the drive (units: 1/(c64_hz*DRIVE_HZ)
    /// of a second, scaled): positive if the next drive cycle starts
    /// before the end of the current C64 cycle.
    frac: i32,
    /// C64 clock, Hz: PAL 985248, NTSC 1022730.
    c64_hz: u32,
    /// Drive instruction trace (debugger): PC and cycle of each one.
    pub trace: Option<Box<dyn FnMut(&Cpu, &DriveBus, u64)>>,
}

impl Drive {
    /// Clock of the C64 it is connected to (PAL or NTSC), Hz: the drive
    /// runs 1,000,000 cycles for every `c64_hz` of the C64.
    pub fn set_c64_clock(&mut self, c64_hz: u64) {
        self.c64_hz = c64_hz as u32;
    }

    /// Drive with the DOS ROM (16 KB) and number `device` (8-11).
    pub fn new(rom: &[u8], device: u8) -> Result<Drive, String> {
        if rom.len() != 0x4000 {
            return Err(format!("1541 ROM of {} bytes, expected 16384", rom.len()));
        }
        let mut d = Drive {
            cpu: Cpu::new(),
            bus: DriveBus {
                ram: [0; 0x800],
                rom: rom.to_vec(),
                via1: Via::new(),
                via2: Via::new(),
                mech: Mechanics::default(),
                last_data: 0,
                c64_atn: false,
                c64_clk: false,
                c64_data: false,
                device: device.wrapping_sub(8) & 3,
                clk: 0,
                activity: 0,
            },
            frac: 0,
            c64_hz: crate::timing::Standard::Pal.clock_hz() as u32,
            trace: None,
        };
        d.reset();
        Ok(d)
    }

    /// Reset (the C64 reset button also reaches the drive).
    pub fn reset(&mut self) {
        self.bus.via1.reset();
        self.bus.via2.reset();
        let m = &mut self.bus.mech;
        // VIA2 CA2 and CB2 go back high: read, BYTE READY enabled
        m.motor = false;
        m.led = false;
        m.read_mode = true;
        m.soe = true;
        self.cpu.reset(&mut self.bus);
    }

    /// Inserts a disk (ejecting the current one, with the disk change
    /// delays the DOS uses to notice it).
    pub fn insert(&mut self, disk: Disk) {
        if self.bus.mech.disk.is_some() {
            self.eject();
        }
        let m = &mut self.bus.mech;
        let clk = self.bus.clk;
        m.attach_clk = Some(clk);
        if m.detach_clk.is_some() {
            m.attach_detach_clk = Some(clk);
        }
        m.disk = Some(disk);
    }

    pub fn eject(&mut self) -> Option<Disk> {
        let m = &mut self.bus.mech;
        m.detach_clk = Some(self.bus.clk);
        m.disk.take()
    }

    /// End of a C64 cycle: the drive runs the cycles that start within
    /// this cycle (1 or 2), so at a C64 access it has run exactly the
    /// cycles that started before; then it receives the serial bus lines
    /// the C64 is now driving (`atn`, `clk`, `data`: true = low).
    ///
    /// A C64 write to $DD00 therefore reaches the drive from the next
    /// cycle, as in VICE: slow rising edge (pull-up on the cable) and
    /// VIA setup time. Fast 2-bit transfers (IFFL) are timed around
    /// this delay: between the C64 write and its read of the bit the
    /// drive sends in response they tolerate from about
    /// 0.6 to 1.25 cycles.
    #[inline]
    pub fn run_c64_cycle(&mut self, atn: bool, clk: bool, data: bool) {
        self.frac += DRIVE_HZ as i32;
        while self.frac > 0 {
            self.frac -= self.c64_hz as i32;
            self.cycle();
        }
        let b = &mut self.bus;
        if atn != b.c64_atn {
            // ATN asserted: rising edge on VIA1 CA1
            b.via1.set_ca1(atn);
        }
        if atn != b.c64_atn || clk != b.c64_clk || data != b.c64_data {
            b.activity += 1;
        }
        b.c64_atn = atn;
        b.c64_clk = clk;
        b.c64_data = data;
    }

    fn cycle(&mut self) {
        if let Some(t) = self.trace.as_mut() {
            if self.cpu.at_boundary() {
                t(&self.cpu, &self.bus, self.bus.clk);
            }
        }
        self.cpu.cycle(&mut self.bus, false, false);
        let b = &mut self.bus;
        b.via1.tick();
        b.via2.tick();
        if b.mech.force_v {
            b.mech.force_v = false;
            self.cpu.set_overflow();
        }
        let irq = b.via1.irq() || b.via2.irq();
        b.clk += 1;
        self.cpu.sample_lines(irq, false, false);
    }

    /// A queued DOS job ($00-$05 with bit 7 set) with the motor on:
    /// covers waits without disk accesses, such as motor spin-up
    /// (almost a second) before the first read. With the motor off it does
    /// not count: a resident loader may use that zero page for other things.
    pub fn job_pending(&self) -> bool {
        self.bus.mech.motor && self.bus.ram[..6].iter().any(|&j| j & 0x80 != 0)
    }

    /// Lines pulled low by the drive with ATN in state `atn`: (CLK, DATA).
    pub fn pulls(&self, atn: bool) -> (bool, bool) {
        self.bus.pulls(atn)
    }

    /// Mechanics and VIA state for the debugger.
    pub fn describe(&self) -> String {
        let m = &self.bus.mech;
        let (v1, v2) = (&self.bus.via1, &self.bus.via2);
        format!(
            "  track {}{} (half {})  head bit {}  motor {}  LED {}  density {}  {}  BYTE READY {}  disk {}\n  \
             VIA1 PB ${:02X} DDRB ${:02X} IFR ${:02X} IER ${:02X}  VIA2 PB ${:02X} DDRB ${:02X} PCR ${:02X} IFR ${:02X} IER ${:02X}  last byte ${:02X}\n  \
             activity {} (disk bytes and serial bus changes)  queued DOS job {}\n",
            m.half_track / 2, if m.half_track % 2 == 1 { ".5" } else { "" }, m.half_track, m.head_bit,
            if m.motor { "on" } else { "off" }, if m.led { "on" } else { "off" }, m.zone,
            if m.read_mode { "read" } else { "WRITE" }, if m.soe { "on" } else { "off" },
            if m.disk.is_some() { "inserted" } else { "none" },
            v1.pb_out(), v1.ddrb, v1.ifr, v1.ier, v2.pb_out(), v2.ddrb, v2.pcr, v2.ifr, v2.ier, m.gcr_read,
            self.bus.activity, if self.job_pending() { "yes" } else { "no" },
        )
    }
}

// ── Saveable state ───────────────────────────────────────────────────────────

// The disk is part of the state: the drive may have written it. The ROM too,
// to reload the state on a C64 without a drive.
impl_state!(Mechanics {
    disk, half_track, head_bit, motor, led, zone, read_mode, soe, accum, shift, write_shift,
    bit_counter, ue7, uf4, fr_randcount, filter_counter, filter_state, filter_last, so_delay,
    cycle_index, xorshift, gcr_read, gcr_write, so_edge, force_v, last_clk, ref_advance, req_ref,
    attach_clk, detach_clk, attach_detach_clk,
});

impl_state!(DriveBus {
    ram, rom, via1, via2, mech, last_data, c64_atn, c64_clk, c64_data, device, clk,
} skip { activity });

impl_state!(Drive { cpu, bus, frac } skip { trace, c64_hz });

impl Default for Drive {
    fn default() -> Self {
        Drive::new(&[0; 0x4000], 8).unwrap()
    }
}

/// Disk tracks in the state (the disk changes during use).
impl State for Disk {
    fn save(&self, w: &mut Writer) {
        for t in &self.tracks {
            match t {
                None => false.save(w),
                Some(t) => {
                    true.save(w);
                    (t.data.len() as u32).save(w);
                    w.put(&t.data);
                }
            }
        }
        self.write_protected.save(w);
        self.dirty.save(w);
        self.from_d64.save(w);
        self.d64_tracks.save(w);
    }

    fn load(&mut self, r: &mut Reader) -> StateResult<()> {
        self.tracks = vec![None; HALF_TRACKS];
        for t in self.tracks.iter_mut() {
            let mut some = false;
            some.load(r)?;
            if some {
                let mut n = 0u32;
                n.load(r)?;
                *t = Some(gcr::Track { data: r.take(n as usize)?.to_vec() });
            }
        }
        self.write_protected.load(r)?;
        self.dirty.load(r)?;
        self.from_d64.load(r)?;
        self.d64_tracks.load(r)
    }
}
